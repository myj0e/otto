use serde_json::{json, Value};
use url::Url;

use crate::config::Config;
use crate::error::{OttoError, Result};
use crate::http::ApiAuth;
use crate::usage::RequestUsage;

const MAX_REQUEST_BYTES: usize = 8 * 1024 * 1024;
// DeepSeek currently permits up to 384 Ki output tokens for its supported models.
// Keep the required Anthropic `max_tokens` field at the provider ceiling so OTTO
// does not truncate long responses at an arbitrary lower limit.
const DEEPSEEK_MAX_OUTPUT_TOKENS: usize = 384 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeSearchProtocol {
    DeepSeekAnthropic,
    OpenAiChat,
    OpenRouterChat,
    AlibabaChat,
}

/// Provider-specific behavior for the model API wire protocol.
///
/// Vendor adapters own endpoint selection, authentication, request encoding,
/// response event normalization, and the matching native-search protocol.
pub trait ApiAdapter: Sync {
    fn id(&self) -> &'static str;
    fn matches(&self, config: &Config) -> bool;
    fn endpoint(&self, config: &Config) -> Result<String>;
    fn auth<'a>(&self, api_key: &'a str) -> ApiAuth<'a>;
    fn request_body(&self, config: &Config, messages: &[Value], tools: &[Value]) -> Result<String>;
    fn normalize_event(&self, value: &Value) -> Option<Value>;

    fn usage_from_event(&self, value: &Value) -> Option<RequestUsage> {
        usage_from_raw(value, self.id() == "deepseek-anthropic")
    }

    fn native_search_protocol(&self) -> Option<NativeSearchProtocol> {
        None
    }
}

struct DeepSeekAdapter;

struct OpenAiCompatibleAdapter {
    adapter_id: &'static str,
    matcher: fn(&Config) -> bool,
    search_protocol: Option<NativeSearchProtocol>,
    include_stream_usage: bool,
}

static DEEPSEEK: DeepSeekAdapter = DeepSeekAdapter;
static OPENROUTER: OpenAiCompatibleAdapter = OpenAiCompatibleAdapter {
    adapter_id: "openrouter",
    matcher: is_openrouter,
    search_protocol: Some(NativeSearchProtocol::OpenRouterChat),
    include_stream_usage: true,
};
static ALIBABA: OpenAiCompatibleAdapter = OpenAiCompatibleAdapter {
    adapter_id: "alibaba-compatible",
    matcher: is_alibaba,
    search_protocol: Some(NativeSearchProtocol::AlibabaChat),
    include_stream_usage: true,
};
static OPENAI: OpenAiCompatibleAdapter = OpenAiCompatibleAdapter {
    adapter_id: "openai",
    matcher: is_openai,
    search_protocol: Some(NativeSearchProtocol::OpenAiChat),
    include_stream_usage: true,
};
static OPENAI_COMPATIBLE_FALLBACK: OpenAiCompatibleAdapter = OpenAiCompatibleAdapter {
    adapter_id: "openai-compatible-fallback",
    matcher: |_| false,
    search_protocol: Some(NativeSearchProtocol::OpenAiChat),
    include_stream_usage: false,
};

pub fn adapter_for(config: &Config) -> &'static dyn ApiAdapter {
    let adapters: [&'static dyn ApiAdapter; 4] = [&DEEPSEEK, &OPENROUTER, &ALIBABA, &OPENAI];
    adapters
        .into_iter()
        .find(|adapter| adapter.matches(config))
        .unwrap_or(&OPENAI_COMPATIBLE_FALLBACK)
}

fn configured_baseurl(config: &Config) -> Result<&str> {
    config
        .baseurl
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| OttoError::Config("baseurl 不能为空".to_owned()))
}

fn openai_chat_endpoint(baseurl: &str) -> String {
    let trimmed = baseurl.trim_end_matches('/');
    if trimmed.ends_with("/chat/completions") {
        trimmed.to_owned()
    } else if trimmed.ends_with("/v1") {
        format!("{trimmed}/chat/completions")
    } else {
        format!("{trimmed}/v1/chat/completions")
    }
}

fn deepseek_messages_endpoint(baseurl: &str) -> String {
    let trimmed = baseurl.trim_end_matches('/');
    if trimmed.ends_with("/anthropic/v1/messages")
        || trimmed.ends_with("/v1/messages")
        || trimmed.ends_with("/messages")
    {
        return trimmed.to_owned();
    }
    if trimmed.ends_with("/anthropic/v1") {
        return format!("{trimmed}/messages");
    }
    if trimmed.ends_with("/anthropic") {
        return format!("{trimmed}/v1/messages");
    }

    let root = trimmed
        .strip_suffix("/v1/chat/completions")
        .or_else(|| trimmed.strip_suffix("/chat/completions"))
        .or_else(|| trimmed.strip_suffix("/v1"))
        .unwrap_or(trimmed)
        .trim_end_matches('/');
    format!("{root}/anthropic/v1/messages")
}

fn check_request_size(body: String) -> Result<String> {
    if body.len() > MAX_REQUEST_BYTES {
        return Err(OttoError::Limit("Agent 消息总大小超过 8 MiB".to_owned()));
    }
    Ok(body)
}

fn openai_request_body(
    config: &Config,
    messages: &[Value],
    tools: &[Value],
    include_stream_usage: bool,
) -> Result<String> {
    let messages = messages
        .iter()
        .map(|message| {
            let mut message = message.clone();
            if let Some(object) = message.as_object_mut() {
                object.remove("_otto_cache_prefix");
            }
            message
        })
        .collect::<Vec<_>>();
    let mut body = json!({
        "model": config.model,
        "messages": messages,
        "stream": true
    });
    if include_stream_usage {
        body["stream_options"] = json!({"include_usage": true});
    }
    if !tools.is_empty() {
        body["tools"] = Value::Array(tools.to_vec());
        body["tool_choice"] = json!("auto");
    }
    check_request_size(body.to_string())
}

fn message_content_text(content: &Value) -> String {
    if let Some(text) = content.as_str() {
        return text.to_owned();
    }
    content
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|part| {
            part.get("text")
                .and_then(Value::as_str)
                .or_else(|| part.get("content").and_then(Value::as_str))
        })
        .collect::<String>()
}

fn anthropic_tools(tools: &[Value]) -> Vec<Value> {
    tools
        .iter()
        .filter_map(|tool| {
            let function = tool.get("function")?;
            let name = function.get("name")?.as_str()?;
            let input_schema = function
                .get("parameters")
                .cloned()
                .unwrap_or_else(|| json!({"type": "object", "properties": {}}));
            let mut mapped = json!({"name": name, "input_schema": input_schema});
            if let Some(description) = function.get("description").and_then(Value::as_str) {
                mapped["description"] = Value::String(description.to_owned());
            }
            Some(mapped)
        })
        .collect()
}

fn anthropic_tool_input(arguments: &str) -> Value {
    match serde_json::from_str::<Value>(arguments) {
        Ok(value) if value.is_object() => value,
        Ok(value) => json!({"value": value}),
        Err(error) => json!({
            "_otto_argument_error": format!("工具参数 JSON 无效：{error}"),
            "_otto_raw_arguments": arguments
        }),
    }
}

fn anthropic_messages(messages: &[Value]) -> (Vec<Value>, Vec<Value>) {
    let mut system = Vec::new();
    let mut converted = Vec::new();
    let mut position = 0usize;

    while position < messages.len() {
        let message = &messages[position];
        let role = message
            .get("role")
            .and_then(Value::as_str)
            .unwrap_or_default();
        match role {
            "system" | "developer" => {
                let content = message
                    .get("content")
                    .map(message_content_text)
                    .unwrap_or_default();
                if !content.is_empty() {
                    let mut block = json!({"type": "text", "text": content});
                    // The base prompt and agent instructions are unchanged
                    // across turns. Keep the rolling summary in a later block
                    // so it does not invalidate this stable cache prefix.
                    if message
                        .get("_otto_cache_prefix")
                        .and_then(Value::as_bool)
                        .unwrap_or(false)
                    {
                        block["cache_control"] = json!({"type": "ephemeral"});
                    }
                    system.push(block);
                }
                position += 1;
            }
            "user" => {
                let content = message.get("content").cloned().unwrap_or(Value::Null);
                converted.push(json!({"role": "user", "content": content}));
                position += 1;
            }
            "assistant" => {
                let mut blocks = Vec::new();
                let text = message
                    .get("content")
                    .map(message_content_text)
                    .unwrap_or_default();
                if !text.is_empty() {
                    blocks.push(json!({"type": "text", "text": text}));
                }
                if let Some(calls) = message.get("tool_calls").and_then(Value::as_array) {
                    for (index, call) in calls.iter().enumerate() {
                        let function = call.get("function").unwrap_or(&Value::Null);
                        let name = function
                            .get("name")
                            .and_then(Value::as_str)
                            .unwrap_or("unknown_tool");
                        let id = call
                            .get("id")
                            .and_then(Value::as_str)
                            .map(str::to_owned)
                            .unwrap_or_else(|| format!("otto_call_{index}"));
                        let arguments = function
                            .get("arguments")
                            .and_then(Value::as_str)
                            .unwrap_or("{}");
                        blocks.push(json!({
                            "type": "tool_use",
                            "id": id,
                            "name": name,
                            "input": anthropic_tool_input(arguments)
                        }));
                    }
                }
                if blocks.is_empty() {
                    blocks.push(json!({"type": "text", "text": ""}));
                }
                converted.push(json!({"role": "assistant", "content": blocks}));
                position += 1;
            }
            "tool" => {
                let mut results = Vec::new();
                while position < messages.len()
                    && messages[position].get("role").and_then(Value::as_str) == Some("tool")
                {
                    let result = &messages[position];
                    let tool_use_id = result
                        .get("tool_call_id")
                        .and_then(Value::as_str)
                        .unwrap_or("otto_unknown_tool_call");
                    let content = result.get("content").cloned().unwrap_or(Value::Null);
                    results.push(json!({
                        "type": "tool_result",
                        "tool_use_id": tool_use_id,
                        "content": content
                    }));
                    position += 1;
                }
                converted.push(json!({"role": "user", "content": results}));
            }
            _ => {
                // Preserve otherwise unknown roles as user data instead of
                // dropping conversation context during provider conversion.
                let content = message.get("content").cloned().unwrap_or(Value::Null);
                converted.push(json!({"role": "user", "content": content}));
                position += 1;
            }
        }
    }

    (system, converted)
}

fn anthropic_request_body(config: &Config, messages: &[Value], tools: &[Value]) -> Result<String> {
    let (system, messages) = anthropic_messages(messages);
    let mut body = json!({
        "model": config.model,
        "max_tokens": DEEPSEEK_MAX_OUTPUT_TOKENS,
        "messages": messages,
        "stream": true
    });
    if !system.is_empty() {
        body["system"] = Value::Array(system);
    }
    let tools = anthropic_tools(tools);
    if !tools.is_empty() {
        body["tools"] = Value::Array(tools);
        body["tool_choice"] = json!({"type": "auto"});
    }
    check_request_size(body.to_string())
}

fn token_count(value: &Value) -> Option<u64> {
    value.as_u64()
}

fn openai_usage(value: &Value) -> Option<RequestUsage> {
    let usage = value.get("usage")?;
    let details = usage
        .get("prompt_tokens_details")
        .or_else(|| usage.get("input_tokens_details"));
    let request = RequestUsage {
        input_tokens: usage
            .get("prompt_tokens")
            .or_else(|| usage.get("input_tokens"))
            .and_then(token_count),
        output_tokens: usage
            .get("completion_tokens")
            .or_else(|| usage.get("output_tokens"))
            .and_then(token_count),
        cache_read_input_tokens: details
            .and_then(|details| details.get("cached_tokens"))
            .or_else(|| usage.get("cached_tokens"))
            .or_else(|| usage.get("prompt_cache_hit_tokens"))
            .and_then(token_count),
        cache_write_input_tokens: usage
            .get("cache_creation_input_tokens")
            .or_else(|| usage.get("cache_write_input_tokens"))
            .and_then(token_count),
    };
    request.has_usage().then_some(request)
}

fn anthropic_usage(value: &Value) -> Option<RequestUsage> {
    let usage = value
        .pointer("/message/usage")
        .or_else(|| value.get("usage"))?;
    let uncached_input = usage.get("input_tokens").and_then(token_count);
    let cache_read = usage.get("cache_read_input_tokens").and_then(token_count);
    let cache_write = usage
        .get("cache_creation_input_tokens")
        .and_then(token_count);
    let output_tokens = if value.get("type").and_then(Value::as_str) == Some("message_start") {
        None
    } else {
        usage.get("output_tokens").and_then(token_count)
    };
    let input_tokens = (uncached_input.is_some() || cache_read.is_some() || cache_write.is_some())
        .then(|| {
            uncached_input
                .into_iter()
                .chain(cache_read)
                .chain(cache_write)
                .try_fold(0u64, u64::checked_add)
        })
        .flatten();
    let request = RequestUsage {
        input_tokens,
        output_tokens,
        cache_read_input_tokens: cache_read,
        cache_write_input_tokens: cache_write,
    };
    request.has_usage().then_some(request)
}

pub fn usage_from_raw(value: &Value, anthropic: bool) -> Option<RequestUsage> {
    if anthropic {
        anthropic_usage(value)
    } else {
        openai_usage(value)
    }
}

fn normalized_text_event(text: &str) -> Value {
    json!({"choices": [{"index": 0, "delta": {"content": text}}]})
}

fn normalized_tool_start(index: usize, block: &Value) -> Option<Value> {
    let name = block.get("name")?.as_str()?;
    let id = block
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or("otto_anthropic_tool_call");
    let arguments = block
        .get("input")
        .and_then(Value::as_object)
        .filter(|input| !input.is_empty())
        .map(|input| Value::Object(input.clone()).to_string())
        .unwrap_or_default();
    Some(json!({
        "choices": [{
            "index": 0,
            "delta": {
                "tool_calls": [{
                    "index": index,
                    "id": id,
                    "type": "function",
                    "function": {"name": name, "arguments": arguments}
                }]
            }
        }]
    }))
}

fn normalize_anthropic_event(value: &Value) -> Option<Value> {
    match value.get("type").and_then(Value::as_str) {
        Some("content_block_start") => {
            let block = value.get("content_block")?;
            match block.get("type").and_then(Value::as_str) {
                Some("text") => {
                    let text = block
                        .get("text")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    (!text.is_empty()).then(|| normalized_text_event(text))
                }
                Some("tool_use") => normalized_tool_start(
                    value.get("index").and_then(Value::as_u64).unwrap_or(0) as usize,
                    block,
                ),
                _ => None,
            }
        }
        Some("content_block_delta") => {
            let index = value.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
            let delta = value.get("delta")?;
            match delta.get("type").and_then(Value::as_str) {
                Some("text_delta") => delta
                    .get("text")
                    .and_then(Value::as_str)
                    .filter(|text| !text.is_empty())
                    .map(normalized_text_event),
                Some("input_json_delta") => {
                    let partial = delta
                        .get("partial_json")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    (!partial.is_empty()).then(|| {
                        json!({
                            "choices": [{
                                "index": 0,
                                "delta": {
                                    "tool_calls": [{
                                        "index": index,
                                        "function": {"arguments": partial}
                                    }]
                                }
                            }]
                        })
                    })
                }
                _ => None,
            }
        }
        Some("message") => normalize_anthropic_message(value),
        _ => None,
    }
}

fn normalize_anthropic_message(value: &Value) -> Option<Value> {
    let blocks = value.get("content")?.as_array()?;
    let mut text = String::new();
    let mut calls = Vec::new();
    for (index, block) in blocks.iter().enumerate() {
        match block.get("type").and_then(Value::as_str) {
            Some("text") => {
                if let Some(content) = block.get("text").and_then(Value::as_str) {
                    text.push_str(content);
                }
            }
            Some("tool_use") => {
                let name = block.get("name").and_then(Value::as_str)?;
                let id = block
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or("otto_anthropic_tool_call");
                let input = block.get("input").cloned().unwrap_or_else(|| json!({}));
                calls.push(json!({
                    "index": index,
                    "id": id,
                    "type": "function",
                    "function": {"name": name, "arguments": input.to_string()}
                }));
            }
            _ => {}
        }
    }

    if text.is_empty() && calls.is_empty() {
        return None;
    }
    let mut message = json!({
        "content": if text.is_empty() { Value::Null } else { Value::String(text) }
    });
    if !calls.is_empty() {
        message["tool_calls"] = Value::Array(calls);
    }
    Some(json!({"choices": [{"index": 0, "message": message}]}))
}

fn deepseek_host(config: &Config) -> bool {
    config
        .baseurl
        .as_deref()
        .and_then(|baseurl| Url::parse(baseurl).ok())
        .and_then(|url| url.host_str().map(str::to_ascii_lowercase))
        .is_some_and(|host| host == "deepseek.com" || host.ends_with(".deepseek.com"))
}

fn named(config: &Config) -> String {
    config
        .name
        .as_deref()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase()
}

fn endpoint_identity(config: &Config) -> String {
    config
        .baseurl
        .as_deref()
        .unwrap_or_default()
        .to_ascii_lowercase()
}

fn is_deepseek(config: &Config) -> bool {
    deepseek_host(config) || named(config).contains("deepseek")
}

fn is_openrouter(config: &Config) -> bool {
    named(config).contains("openrouter") || endpoint_identity(config).contains("openrouter.ai")
}

fn is_alibaba(config: &Config) -> bool {
    let identity = format!("{} {}", named(config), endpoint_identity(config));
    ["dashscope", "aliyuncs", "alibaba", "qwen"]
        .iter()
        .any(|marker| identity.contains(marker))
}

fn is_openai(config: &Config) -> bool {
    let name = named(config);
    let host_matches = config
        .baseurl
        .as_deref()
        .and_then(|baseurl| Url::parse(baseurl).ok())
        .and_then(|url| url.host_str().map(str::to_ascii_lowercase))
        .is_some_and(|host| host == "openai.com" || host.ends_with(".openai.com"));
    host_matches || matches!(name.as_str(), "openai" | "open ai")
}

impl ApiAdapter for DeepSeekAdapter {
    fn id(&self) -> &'static str {
        "deepseek-anthropic"
    }

    fn matches(&self, config: &Config) -> bool {
        is_deepseek(config)
    }

    fn endpoint(&self, config: &Config) -> Result<String> {
        Ok(deepseek_messages_endpoint(configured_baseurl(config)?))
    }

    fn auth<'a>(&self, api_key: &'a str) -> ApiAuth<'a> {
        ApiAuth::Anthropic(api_key)
    }

    fn request_body(&self, config: &Config, messages: &[Value], tools: &[Value]) -> Result<String> {
        anthropic_request_body(config, messages, tools)
    }

    fn normalize_event(&self, value: &Value) -> Option<Value> {
        normalize_anthropic_event(value)
    }

    fn native_search_protocol(&self) -> Option<NativeSearchProtocol> {
        Some(NativeSearchProtocol::DeepSeekAnthropic)
    }
}

impl ApiAdapter for OpenAiCompatibleAdapter {
    fn id(&self) -> &'static str {
        self.adapter_id
    }

    fn matches(&self, config: &Config) -> bool {
        (self.matcher)(config)
    }

    fn endpoint(&self, config: &Config) -> Result<String> {
        Ok(openai_chat_endpoint(configured_baseurl(config)?))
    }

    fn auth<'a>(&self, api_key: &'a str) -> ApiAuth<'a> {
        ApiAuth::Bearer(api_key)
    }

    fn request_body(&self, config: &Config, messages: &[Value], tools: &[Value]) -> Result<String> {
        openai_request_body(config, messages, tools, self.include_stream_usage)
    }

    fn normalize_event(&self, value: &Value) -> Option<Value> {
        Some(value.clone())
    }

    fn native_search_protocol(&self) -> Option<NativeSearchProtocol> {
        self.search_protocol
    }
}
