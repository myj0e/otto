mod bash;
mod local;
mod native_search;
mod otto_storage;
mod web;

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Map, Value};

use crate::config::{Config, SearchConfig};
use crate::error::{OttoError, Result};
use crate::permission::PermissionManager;
use crate::workspace::Workspace;

pub const MAX_TOOL_OUTPUT_BYTES: usize = 64 * 1024;

pub(super) fn env_value(names: &[&str]) -> Option<String> {
    names.iter().find_map(|name| {
        std::env::var(name)
            .ok()
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
    })
}

pub struct ToolContext<'a> {
    pub workspace: &'a Workspace,
    pub permissions: &'a mut PermissionManager,
    pub mode: Option<&'a str>,
    pub model_config: &'a Config,
    pub model_endpoint: &'a str,
    pub search_config: &'a SearchConfig,
}

#[derive(Debug, Clone)]
pub struct ToolOutput {
    pub content: String,
    /// Optional terminal-only display name. This is deliberately kept separate from
    /// `content`, which is sent back to the model as the tool result.
    pub display_name: Option<String>,
}

impl ToolOutput {
    pub fn text(content: impl Into<String>) -> Self {
        Self {
            content: truncate(content.into(), MAX_TOOL_OUTPUT_BYTES),
            display_name: None,
        }
    }

    pub fn error(error: impl Into<String>) -> Self {
        Self::text(format!("[tool_error]\n{}", error.into()))
    }

    pub fn with_display_name(mut self, name: impl Into<String>) -> Self {
        self.display_name = Some(name.into());
        self
    }
}

#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &'static str;
    fn definition(&self) -> Value;
    async fn execute(&self, arguments: Value, context: &mut ToolContext<'_>) -> Result<ToolOutput>;
}

pub struct ToolRegistry {
    tools: HashMap<String, Arc<dyn Tool>>,
    ordered: Vec<Arc<dyn Tool>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        let mut registry = Self {
            tools: HashMap::new(),
            ordered: Vec::new(),
        };
        registry.register(bash::BashTool);
        registry.register(local::GlobTool);
        registry.register(local::GrepTool);
        registry.register(local::ReadTool);
        registry.register(local::EditTool);
        registry.register(local::WriteTool);
        registry.register(otto_storage::OttoStorageTool);
        registry.register(web::WebSearchTool);
        registry.register(web::WebFetchTool);
        registry
    }

    pub fn register<T>(&mut self, tool: T)
    where
        T: Tool + 'static,
    {
        let tool: Arc<dyn Tool> = Arc::new(tool);
        self.tools.insert(tool.name().to_owned(), Arc::clone(&tool));
        self.ordered.push(tool);
    }

    pub fn definitions(&self) -> Vec<Value> {
        self.ordered.iter().map(|tool| tool.definition()).collect()
    }

    pub async fn execute(
        &self,
        name: &str,
        arguments: Value,
        context: &mut ToolContext<'_>,
    ) -> ToolOutput {
        let key = name.to_ascii_lowercase();
        let Some(tool) = self.tools.get(&key) else {
            return ToolOutput::error(format!("未知工具：{name}"));
        };
        match tool.execute(arguments, context).await {
            Ok(output) => output,
            Err(error) => ToolOutput::error(error.to_string()),
        }
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

pub fn arguments_object(arguments: Value) -> Result<Map<String, Value>> {
    arguments
        .as_object()
        .cloned()
        .ok_or_else(|| OttoError::Tool("工具参数必须是 JSON 对象".to_owned()))
}

pub fn required_string(arguments: &Map<String, Value>, key: &str) -> Result<String> {
    let value = arguments
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| OttoError::Tool(format!("缺少字符串参数：{key}")))?;
    Ok(value.to_owned())
}

pub fn optional_string(arguments: &Map<String, Value>, key: &str) -> Result<Option<String>> {
    match arguments.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_str()
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
            .map(Some)
            .ok_or_else(|| OttoError::Tool(format!("参数 {key} 必须是字符串"))),
    }
}

pub fn optional_bool(arguments: &Map<String, Value>, key: &str, default: bool) -> Result<bool> {
    match arguments.get(key) {
        None | Some(Value::Null) => Ok(default),
        Some(value) => value
            .as_bool()
            .ok_or_else(|| OttoError::Tool(format!("参数 {key} 必须是布尔值"))),
    }
}

pub fn optional_usize(
    arguments: &Map<String, Value>,
    key: &str,
    default: usize,
    maximum: usize,
) -> Result<usize> {
    let value = match arguments.get(key) {
        None | Some(Value::Null) => default,
        Some(value) => value
            .as_u64()
            .ok_or_else(|| OttoError::Tool(format!("参数 {key} 必须是正整数")))?
            as usize,
    };
    if value == 0 || value > maximum {
        return Err(OttoError::Tool(format!(
            "参数 {key} 必须在 1 到 {maximum} 之间"
        )));
    }
    Ok(value)
}

pub fn function_definition(
    name: &str,
    description: &str,
    properties: Value,
    required: &[&str],
) -> Value {
    json!({
        "type": "function",
        "function": {
            "name": name,
            "description": description,
            "parameters": {
                "type": "object",
                "properties": properties,
                "required": required,
                "additionalProperties": false
            }
        }
    })
}

pub fn truncate(mut value: String, maximum: usize) -> String {
    if value.len() <= maximum {
        return value;
    }
    let suffix = "\n[otto: 工具输出已截断]";
    let mut limit = maximum.saturating_sub(suffix.len());
    while limit > 0 && !value.is_char_boundary(limit) {
        limit -= 1;
    }
    value.truncate(limit);
    value.push_str(suffix);
    value
}

#[cfg(test)]
mod tests {
    use super::{function_definition, truncate, ToolRegistry};

    #[test]
    fn truncates_on_utf8_boundary() {
        let output = truncate("你好世界".to_owned(), 8);
        assert!(output.is_char_boundary(output.len()));
        assert!(output.contains("工具输出已截断") || output == "你好世界");
    }

    #[test]
    fn builds_openai_function_definition() {
        let definition = function_definition(
            "read",
            "read a file",
            serde_json::json!({"path": {"type": "string"}}),
            &["path"],
        );
        assert_eq!(definition["function"]["name"], "read");
        assert_eq!(definition["function"]["parameters"]["required"][0], "path");
    }

    #[test]
    fn registers_current_tool_set() {
        let registry = ToolRegistry::default();
        let names = registry
            .definitions()
            .into_iter()
            .filter_map(|definition| definition["function"]["name"].as_str().map(str::to_owned))
            .collect::<Vec<_>>();
        assert_eq!(
            names,
            vec![
                "bash",
                "glob",
                "grep",
                "read",
                "edit",
                "write",
                "websearch",
                "webfetch"
            ]
        );
    }
}
