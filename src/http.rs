use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[cfg(target_os = "linux")]
use std::process::Command;

use futures_util::StreamExt;
use reqwest::header::{ACCEPT, CONTENT_TYPE};
use serde_json::Value;

use crate::error::{OttoError, Result};

const DEFAULT_MAX_RESPONSE_BYTES: usize = 16 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
struct SystemProxySettings {
    http: Option<String>,
    https: Option<String>,
    socks: Option<String>,
    no_proxy: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum SystemProxyState {
    Disabled,
    Manual(SystemProxySettings),
}

/// Build a client with the operating system's proxy behavior.
///
/// Reqwest follows proxy environment variables by default. On Linux, desktop
/// proxy switches are commonly stored in GNOME's gsettings instead of being
/// exported to the environment, so honor that setting when it is available.
/// If no desktop setting can be read, leave reqwest's default behavior intact.
pub fn client_builder() -> Result<reqwest::ClientBuilder> {
    let builder = reqwest::Client::builder();
    match system_proxy_state() {
        Some(SystemProxyState::Disabled) => Ok(builder.no_proxy()),
        Some(SystemProxyState::Manual(settings)) => configure_manual_proxy(builder, settings),
        None => Ok(builder),
    }
}

fn configure_manual_proxy(
    mut builder: reqwest::ClientBuilder,
    settings: SystemProxySettings,
) -> Result<reqwest::ClientBuilder> {
    // An explicit desktop setting is authoritative. This prevents a stale
    // HTTP_PROXY/ALL_PROXY value from defeating the system proxy switch.
    builder = builder.no_proxy();
    let no_proxy = settings
        .no_proxy
        .as_deref()
        .and_then(reqwest::NoProxy::from_string);

    if let Some(endpoint) = settings.http {
        let proxy = reqwest::Proxy::http(endpoint)
            .map_err(|error| OttoError::Network(format!("系统 HTTP 代理无效：{error}")))?
            .no_proxy(no_proxy.clone());
        builder = builder.proxy(proxy);
    }
    if let Some(endpoint) = settings.https {
        let proxy = reqwest::Proxy::https(endpoint)
            .map_err(|error| OttoError::Network(format!("系统 HTTPS 代理无效：{error}")))?
            .no_proxy(no_proxy.clone());
        builder = builder.proxy(proxy);
    }
    if let Some(endpoint) = settings.socks {
        let proxy = reqwest::Proxy::all(endpoint)
            .map_err(|error| OttoError::Network(format!("系统 SOCKS 代理无效：{error}")))?
            .no_proxy(no_proxy);
        builder = builder.proxy(proxy);
    }
    Ok(builder)
}

fn system_proxy_state() -> Option<SystemProxyState> {
    #[cfg(target_os = "linux")]
    {
        return gnome_system_proxy_state();
    }

    #[cfg(not(target_os = "linux"))]
    {
        None
    }
}

#[cfg(target_os = "linux")]
fn gnome_system_proxy_state() -> Option<SystemProxyState> {
    let mode = gsettings_value("org.gnome.system.proxy", "mode")?;
    match parse_gsettings_scalar(&mode).as_str() {
        "none" => Some(SystemProxyState::Disabled),
        "manual" => {
            let http = gsettings_endpoint("http", "org.gnome.system.proxy.http");
            let use_same_proxy = gsettings_value("org.gnome.system.proxy", "use-same-proxy")
                .is_some_and(|value| parse_gsettings_scalar(&value) == "true");
            let https = if use_same_proxy {
                http.clone()
            } else {
                gsettings_endpoint("http", "org.gnome.system.proxy.https")
            };
            let socks = gsettings_endpoint("socks5h", "org.gnome.system.proxy.socks");
            let no_proxy = gsettings_value("org.gnome.system.proxy", "ignore-hosts")
                .and_then(|value| normalize_no_proxy(&value));
            Some(SystemProxyState::Manual(SystemProxySettings {
                http,
                https,
                socks,
                no_proxy,
            }))
        }
        // Reqwest has no PAC resolver. Keep its normal environment behavior
        // for automatic proxy mode instead of guessing at a proxy endpoint.
        _ => None,
    }
}

#[cfg(target_os = "linux")]
fn gsettings_value(schema: &str, key: &str) -> Option<String> {
    let output = Command::new("gsettings")
        .args(["get", schema, key])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let value = String::from_utf8(output.stdout).ok()?;
    Some(value.trim().to_owned())
}

fn parse_gsettings_scalar(value: &str) -> String {
    let value = value.trim().strip_prefix("uint32 ").unwrap_or(value.trim());
    if value.len() >= 2
        && ((value.starts_with('\'') && value.ends_with('\''))
            || (value.starts_with('"') && value.ends_with('"')))
    {
        value[1..value.len() - 1].to_owned()
    } else {
        value.to_owned()
    }
}

#[cfg(target_os = "linux")]
fn gsettings_endpoint(scheme: &str, schema: &str) -> Option<String> {
    let host = parse_gsettings_scalar(&gsettings_value(schema, "host")?);
    if host.is_empty() {
        return None;
    }
    let port = parse_gsettings_scalar(&gsettings_value(schema, "port")?)
        .parse::<u16>()
        .ok()?;
    if port == 0 {
        return None;
    }
    let host = if host.parse::<std::net::IpAddr>().is_ok() && host.contains(':') {
        format!("[{host}]")
    } else {
        host
    };
    Some(format!("{scheme}://{host}:{port}"))
}

fn normalize_no_proxy(value: &str) -> Option<String> {
    let value = value.trim().strip_prefix("@as ").unwrap_or(value.trim());
    let value = value
        .strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
        .unwrap_or(value);
    let entries = value
        .split(',')
        .filter_map(|entry| {
            let entry = parse_gsettings_scalar(entry.trim());
            normalize_no_proxy_entry(&entry)
        })
        .collect::<Vec<_>>();
    (!entries.is_empty()).then(|| entries.join(","))
}

fn normalize_no_proxy_entry(entry: &str) -> Option<String> {
    if entry.is_empty() {
        return None;
    }
    if entry == "*" {
        return Some(entry.to_owned());
    }
    if let Some(prefix) = entry.strip_suffix(".*") {
        let parts = prefix.split('.').collect::<Vec<_>>();
        if !parts.is_empty()
            && parts.len() <= 3
            && parts.iter().all(|part| part.parse::<u8>().is_ok())
        {
            let address = format!(
                "{}.{}",
                prefix,
                (0..(4 - parts.len()))
                    .map(|_| "0")
                    .collect::<Vec<_>>()
                    .join(".")
            );
            return Some(format!("{address}/{}", parts.len() * 8));
        }
    }
    Some(entry.strip_prefix('*').unwrap_or(entry).to_owned())
}

#[derive(Debug)]
pub enum ChatEvent {
    Data(Value),
    ResponseFinished,
    ResponseFailed(String),
    Done,
}

#[derive(Clone, Copy)]
pub enum ApiAuth<'a> {
    Bearer(&'a str),
    Anthropic(&'a str),
}

pub struct ChatRequest<'a> {
    pub endpoint: &'a str,
    pub auth: ApiAuth<'a>,
    pub accept: &'a str,
    pub body: &'a str,
    pub connect_timeout: Duration,
    pub timeout: Duration,
    pub max_response_bytes: usize,
}

fn dispatch_event<F>(data: &mut String, callback: &mut F) -> Result<()>
where
    F: FnMut(ChatEvent) -> Result<()>,
{
    if data.is_empty() {
        return Ok(());
    }

    let event = if data.trim() == "[DONE]" {
        ChatEvent::Done
    } else {
        let value: Value = serde_json::from_str(data.trim())?;
        if let Some(message) = api_error(&value) {
            return Err(OttoError::Api(message));
        }
        let finish_reason = value
            .pointer("/choices/0/finish_reason")
            .and_then(Value::as_str)
            .or_else(|| value.pointer("/delta/stop_reason").and_then(Value::as_str))
            .map(str::to_owned);
        let anthropic_stop = value.get("type").and_then(Value::as_str) == Some("message_stop");
        callback(ChatEvent::Data(value))?;
        if let Some(reason) = finish_reason {
            if matches!(
                reason.as_str(),
                "stop" | "tool_calls" | "end_turn" | "stop_sequence" | "tool_use"
            ) {
                ChatEvent::ResponseFinished
            } else {
                ChatEvent::ResponseFailed(reason)
            }
        } else if anthropic_stop {
            ChatEvent::Done
        } else {
            data.clear();
            return Ok(());
        }
    };
    data.clear();
    callback(event)
}

async fn read_bounded(response: reqwest::Response, maximum: usize) -> Result<(Vec<u8>, bool)> {
    let mut stream = response.bytes_stream();
    let mut body = Vec::with_capacity(maximum.min(16 * 1024));
    let mut exceeded = false;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| OttoError::Network(error.to_string()))?;
        let available = maximum.saturating_sub(body.len());
        let keep = chunk.len().min(available);
        body.extend_from_slice(&chunk[..keep]);
        if keep < chunk.len() {
            exceeded = true;
            break;
        }
    }
    Ok((body, exceeded))
}

fn ensure_sse_finished(finished: bool) -> Result<()> {
    if finished {
        Ok(())
    } else {
        Err(OttoError::Api(
            "SSE 响应提前结束，缺少正常结束标记".to_owned(),
        ))
    }
}

fn api_error(value: &Value) -> Option<String> {
    let error = value.get("error")?;
    if let Some(message) = error.as_str() {
        return Some(message.to_owned());
    }
    if let Some(message) = error.get("message").and_then(Value::as_str) {
        return Some(message.to_owned());
    }
    Some(error.to_string())
}

fn process_line<F>(line: &[u8], event_data: &mut String, callback: &mut F) -> Result<()>
where
    F: FnMut(ChatEvent) -> Result<()>,
{
    let line = line.strip_suffix(b"\r").unwrap_or(line);
    if line.is_empty() {
        return dispatch_event(event_data, callback);
    }
    if line.starts_with(b":") {
        return Ok(());
    }

    if let Some(value) = line.strip_prefix(b"data:") {
        let value = value.strip_prefix(b" ").unwrap_or(value);
        let value = std::str::from_utf8(value)
            .map_err(|_| OttoError::Api("SSE 数据不是有效的 UTF-8".to_owned()))?;
        if !event_data.is_empty() {
            event_data.push('\n');
        }
        event_data.push_str(value);
    }
    Ok(())
}

pub fn event_content(value: &Value) -> Option<&str> {
    let choice = value.get("choices")?.as_array()?.first()?;
    choice
        .get("delta")
        .and_then(|delta| delta.get("content"))
        .and_then(Value::as_str)
        .or_else(|| {
            choice
                .get("message")
                .and_then(|message| message.get("content"))
                .and_then(Value::as_str)
        })
}

pub async fn chat_stream_events<F>(request: ChatRequest<'_>, mut on_event: F) -> Result<()>
where
    F: FnMut(ChatEvent) -> Result<()>,
{
    let client = client_builder()?
        .connect_timeout(request.connect_timeout)
        .timeout(request.timeout)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|error| OttoError::Network(error.to_string()))?;

    let mut builder = client
        .post(request.endpoint)
        .header(CONTENT_TYPE, "application/json")
        .header(ACCEPT, request.accept);
    builder = match request.auth {
        ApiAuth::Bearer(api_key) => builder.bearer_auth(api_key),
        ApiAuth::Anthropic(api_key) => builder
            .header("x-api-key", api_key)
            .header("anthropic-version", "2023-06-01"),
    };
    let response = builder
        .body(request.body.to_owned())
        .send()
        .await
        .map_err(|error| OttoError::Network(error.to_string()))?;

    let status = response.status();
    let content_type = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_ascii_lowercase();

    let maximum = if request.max_response_bytes == 0 {
        DEFAULT_MAX_RESPONSE_BYTES
    } else {
        request.max_response_bytes
    };

    if !status.is_success() {
        let (body, truncated) = read_bounded(response, 8 * 1024).await?;
        let body = String::from_utf8_lossy(&body);
        let detail = if body.is_empty() {
            format!("HTTP {}", status.as_u16())
        } else {
            format!(
                "HTTP {}：{}{}",
                status.as_u16(),
                body,
                if truncated {
                    "…[响应已截断]"
                } else {
                    ""
                }
            )
        };
        return Err(OttoError::Api(detail));
    }

    if !content_type.contains("text/event-stream") {
        let (body, exceeded) = read_bounded(response, maximum).await?;
        if exceeded {
            return Err(OttoError::Api("API 响应超过大小限制".to_owned()));
        }
        let value: Value = serde_json::from_slice(&body)?;
        if let Some(message) = api_error(&value) {
            return Err(OttoError::Api(message));
        }
        on_event(ChatEvent::Data(value))?;
        on_event(ChatEvent::Done)?;
        return Ok(());
    }

    let mut stream = response.bytes_stream();
    let mut pending = Vec::new();
    let mut event_data = String::new();
    let mut received = 0usize;
    let finished = Arc::new(AtomicBool::new(false));
    let finished_for_dispatch = Arc::clone(&finished);
    let response_finished = Arc::new(AtomicBool::new(false));
    let response_finished_for_dispatch = Arc::clone(&response_finished);
    let response_failure = Arc::new(Mutex::new(None::<String>));
    let response_failure_for_dispatch = Arc::clone(&response_failure);
    let mut dispatch = |event: ChatEvent| {
        match &event {
            ChatEvent::Done => finished_for_dispatch.store(true, Ordering::Relaxed),
            ChatEvent::ResponseFinished => {
                response_finished_for_dispatch.store(true, Ordering::Relaxed)
            }
            ChatEvent::ResponseFailed(reason) => {
                response_finished_for_dispatch.store(true, Ordering::Relaxed);
                *response_failure_for_dispatch
                    .lock()
                    .map_err(|_| OttoError::Api("无法保存模型流结束状态".to_owned()))? =
                    Some(reason.clone());
                return Ok(());
            }
            ChatEvent::Data(_) => {}
        }
        on_event(event)
    };

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| OttoError::Network(error.to_string()))?;
        received = received
            .checked_add(chunk.len())
            .ok_or_else(|| OttoError::Api("SSE 响应大小溢出".to_owned()))?;
        if received > maximum {
            return Err(OttoError::Api("SSE 响应超过大小限制".to_owned()));
        }
        pending.extend_from_slice(&chunk);

        while let Some(position) = pending.iter().position(|value| *value == b'\n') {
            let line: Vec<u8> = pending.drain(..=position).collect();
            process_line(&line[..line.len() - 1], &mut event_data, &mut dispatch)?;
        }
        if finished.load(Ordering::Relaxed) {
            break;
        }
    }

    if !finished.load(Ordering::Relaxed) && !pending.is_empty() {
        process_line(&pending, &mut event_data, &mut dispatch)?;
    }
    if !finished.load(Ordering::Relaxed) && !event_data.is_empty() {
        dispatch_event(&mut event_data, &mut dispatch)?;
    }

    if !finished.load(Ordering::Relaxed) && response_finished.load(Ordering::Relaxed) {
        dispatch(ChatEvent::Done)?;
    }
    ensure_sse_finished(finished.load(Ordering::Relaxed))?;
    if let Some(reason) = response_failure
        .lock()
        .map_err(|_| OttoError::Api("无法读取模型流结束状态".to_owned()))?
        .clone()
    {
        return Err(OttoError::Api(format!("模型未正常完成响应：{reason}")));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        dispatch_event, ensure_sse_finished, event_content, normalize_no_proxy,
        parse_gsettings_scalar, process_line, ChatEvent,
    };

    #[test]
    fn parses_gsettings_values() {
        assert_eq!(parse_gsettings_scalar("'manual'"), "manual");
        assert_eq!(parse_gsettings_scalar("uint32 7892"), "7892");
    }

    #[test]
    fn normalizes_gnome_no_proxy_patterns() {
        assert_eq!(
            normalize_no_proxy("['localhost', '127.*', '*.example.com']"),
            Some("localhost,127.0.0.0/8,.example.com".to_owned())
        );
    }

    #[test]
    fn parses_multiline_sse_data() {
        let mut data = String::new();
        let mut events = Vec::new();
        process_line(
            b"data: {\"choices\":[{\"delta\":{\"content\":\"hi\"}}]}",
            &mut data,
            &mut |event| {
                events.push(event);
                Ok(())
            },
        )
        .expect("first line");
        process_line(b"", &mut data, &mut |event| {
            events.push(event);
            Ok(())
        })
        .expect("event boundary");

        assert_eq!(events.len(), 1);
        match &events[0] {
            ChatEvent::Data(value) => assert_eq!(event_content(value), Some("hi")),
            ChatEvent::ResponseFinished => panic!("expected data event"),
            ChatEvent::ResponseFailed(_) => panic!("expected data event"),
            ChatEvent::Done => panic!("expected data event"),
        }
    }

    #[test]
    fn parses_done_event() {
        let mut data = String::new();
        let mut done = false;
        process_line(b"data: [DONE]", &mut data, &mut |_| Ok(())).expect("data line");
        process_line(b"", &mut data, &mut |event| {
            if matches!(event, ChatEvent::Done) {
                done = true;
            }
            Ok(())
        })
        .expect("event boundary");
        assert!(done);
    }

    #[test]
    fn requires_a_terminal_sse_marker() {
        assert!(ensure_sse_finished(false).is_err());
        assert!(ensure_sse_finished(true).is_ok());
    }

    #[test]
    fn rejects_truncated_model_output() {
        let mut data = r#"{"choices":[{"finish_reason":"length"}]}"#.to_owned();
        let mut failed = false;
        dispatch_event(&mut data, &mut |event| {
            if matches!(event, ChatEvent::ResponseFailed(_)) {
                failed = true;
            }
            Ok(())
        })
        .expect("usage may still follow the failure marker");
        assert!(failed);
    }

    #[test]
    fn accepts_anthropic_tool_use_as_normal_completion() {
        let mut data = r#"{"type":"message_delta","delta":{"stop_reason":"tool_use"}}"#.to_owned();
        let mut events = Vec::new();
        dispatch_event(&mut data, &mut |event| {
            events.push(event);
            Ok(())
        })
        .expect("tool_use is a completed model turn");

        assert!(matches!(
            events.as_slice(),
            [ChatEvent::Data(_), ChatEvent::ResponseFinished]
        ));
    }
}
