use std::cell::Cell;
use std::time::Duration;

use futures_util::StreamExt;
use reqwest::header::{ACCEPT, CONTENT_TYPE};
use serde_json::Value;

use crate::error::{OttoError, Result};

const DEFAULT_MAX_RESPONSE_BYTES: usize = 16 * 1024 * 1024;

#[derive(Debug)]
pub enum ChatEvent {
    Data(Value),
    Done,
}

pub struct ChatRequest<'a> {
    pub endpoint: &'a str,
    pub apikey: &'a str,
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
        ChatEvent::Data(value)
    };
    data.clear();
    callback(event)
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

fn event_content(value: &Value) -> Option<&str> {
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
    let client = reqwest::Client::builder()
        .connect_timeout(request.connect_timeout)
        .timeout(request.timeout)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|error| OttoError::Network(error.to_string()))?;

    let response = client
        .post(request.endpoint)
        .bearer_auth(request.apikey)
        .header(CONTENT_TYPE, "application/json")
        .header(ACCEPT, "text/event-stream")
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

    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        let detail = if body.is_empty() {
            format!("HTTP {}", status.as_u16())
        } else {
            format!("HTTP {}：{}", status.as_u16(), body)
        };
        return Err(OttoError::Api(detail));
    }

    let maximum = if request.max_response_bytes == 0 {
        DEFAULT_MAX_RESPONSE_BYTES
    } else {
        request.max_response_bytes
    };

    if !content_type.contains("text/event-stream") {
        let body = response
            .text()
            .await
            .map_err(|error| OttoError::Network(error.to_string()))?;
        if body.len() > maximum {
            return Err(OttoError::Api("API 响应超过大小限制".to_owned()));
        }
        let value: Value = serde_json::from_str(&body)?;
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
    let finished = Cell::new(false);
    let mut dispatch = |event: ChatEvent| {
        if matches!(&event, ChatEvent::Done) {
            finished.set(true);
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
        if finished.get() {
            break;
        }
    }

    if !pending.is_empty() {
        process_line(&pending, &mut event_data, &mut dispatch)?;
    }
    if !event_data.is_empty() {
        dispatch_event(&mut event_data, &mut dispatch)?;
    }

    Ok(())
}

pub async fn chat_stream<F>(request: ChatRequest<'_>, mut on_content: F) -> Result<()>
where
    F: FnMut(&str) -> Result<()>,
{
    let mut wrote_anything = false;
    chat_stream_events(request, |event| match event {
        ChatEvent::Data(value) => {
            if let Some(content) = event_content(&value) {
                on_content(content)?;
                wrote_anything = true;
            }
            Ok(())
        }
        ChatEvent::Done => Ok(()),
    })
    .await?;
    if !wrote_anything {
        return Err(OttoError::Api("API 响应中没有回答内容".to_owned()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{event_content, process_line, ChatEvent};

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
}
