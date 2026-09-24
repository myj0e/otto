use std::fs;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use globset::GlobBuilder;
use regex::RegexBuilder;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use similar::TextDiff;
use tempfile::NamedTempFile;
use walkdir::WalkDir;

use super::{
    arguments_object, function_definition, optional_bool, optional_string, optional_usize,
    required_string, Tool, ToolContext, ToolOutput,
};
use crate::error::{OttoError, Result};
use crate::permission::Capability;

const MAX_READ_CONTENT_BYTES: usize = 8 * 1024;
const MAX_READ_RANGE_BYTES: usize = 64 * 1024 * 1024;
const MAX_LINE_SKIP_BYTES: u64 = 64 * 1024 * 1024;
const MAX_EDIT_BYTES: u64 = 4 * 1024 * 1024;
const MAX_GREP_FILE_BYTES: u64 = 4 * 1024 * 1024;
const MAX_RESULTS: usize = 1000;
const MAX_PREVIEW_BYTES: usize = 12 * 1024;
const MAX_DIFF_LINES: usize = 20_000;
static FILE_WRITE_LOCK: Mutex<()> = Mutex::new(());

pub struct GlobTool;
pub struct GrepTool;
pub struct ReadTool;
pub struct EditTool;
pub struct WriteTool;

fn authorize(
    context: &mut ToolContext<'_>,
    tool_name: &str,
    capability: Capability,
    action: &str,
) -> Result<()> {
    context
        .permissions
        .authorize(tool_name, capability, context.mode, action)
}

fn require_file(path: &Path, input: &str) -> Result<()> {
    if !path.is_file() {
        return Err(OttoError::Tool(format!("目标不是普通文件：{input}")));
    }
    Ok(())
}

fn required_string_allow_empty(
    arguments: &serde_json::Map<String, Value>,
    key: &str,
) -> Result<String> {
    arguments
        .get(key)
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .ok_or_else(|| OttoError::Tool(format!("缺少字符串参数：{key}")))
}

fn read_utf8(path: &Path, input: &str, maximum: u64) -> Result<String> {
    let file = fs::File::open(path)?;
    if file.metadata()?.len() > maximum {
        return Err(OttoError::Tool(format!(
            "文件 {} 超过 {} KiB 限制",
            input,
            maximum / 1024
        )));
    }
    let mut bytes = Vec::with_capacity(maximum.min(64 * 1024) as usize);
    file.take(maximum.saturating_add(1))
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > maximum {
        return Err(OttoError::Tool(format!(
            "文件 {} 在读取期间超过 {} KiB 限制",
            input,
            maximum / 1024
        )));
    }
    String::from_utf8(bytes)
        .map_err(|_| OttoError::Tool(format!("文件不是有效的 UTF-8 文本：{input}")))
}

struct LineRead {
    bytes: Vec<u8>,
    terminated: bool,
    eof: bool,
    truncated: bool,
    consumed: u64,
}

/// Read no more than `maximum` bytes from one line. A long or minified line
/// cannot cause an unbounded allocation, even when the file itself is huge.
fn read_line_limited(reader: &mut impl BufRead, maximum: usize) -> std::io::Result<LineRead> {
    let mut bytes = Vec::with_capacity(maximum.min(4096));
    let mut consumed = 0u64;
    loop {
        let buffer = reader.fill_buf()?;
        if buffer.is_empty() {
            return Ok(LineRead {
                eof: bytes.is_empty(),
                bytes,
                terminated: false,
                truncated: false,
                consumed,
            });
        }
        let newline = buffer.iter().position(|byte| *byte == b'\n');
        let segment_length = newline.map_or(buffer.len(), |index| index + 1);
        let available = maximum.saturating_sub(bytes.len());
        if segment_length > available {
            bytes.extend_from_slice(&buffer[..available]);
            reader.consume(available);
            consumed = consumed.saturating_add(available as u64);
            return Ok(LineRead {
                bytes,
                terminated: false,
                eof: false,
                truncated: true,
                consumed,
            });
        }
        bytes.extend_from_slice(&buffer[..segment_length]);
        reader.consume(segment_length);
        consumed = consumed.saturating_add(segment_length as u64);
        if newline.is_some() {
            return Ok(LineRead {
                bytes,
                terminated: true,
                eof: false,
                truncated: false,
                consumed,
            });
        }
    }
}

enum SkippedLine {
    Complete(u64),
    Eof,
    LimitReached,
}

fn skip_one_line(reader: &mut impl BufRead, maximum: u64) -> std::io::Result<SkippedLine> {
    let mut consumed = 0u64;
    let mut saw_bytes = false;
    loop {
        let buffer = reader.fill_buf()?;
        if buffer.is_empty() {
            return Ok(if saw_bytes {
                SkippedLine::Complete(consumed)
            } else {
                SkippedLine::Eof
            });
        }
        saw_bytes = true;
        let take = buffer
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(buffer.len(), |index| index + 1);
        let ended = buffer.get(take.saturating_sub(1)) == Some(&b'\n');
        let remaining = maximum.saturating_sub(consumed) as usize;
        if take > remaining {
            reader.consume(remaining);
            return Ok(SkippedLine::LimitReached);
        }
        reader.consume(take);
        consumed = consumed.saturating_add(take as u64);
        if ended {
            return Ok(SkippedLine::Complete(consumed));
        }
    }
}

fn decode_read_fragment(bytes: &[u8], input: &str, allow_incomplete_tail: bool) -> Result<String> {
    if bytes.contains(&0) {
        return Err(OttoError::Tool(format!(
            "文件包含 NUL 字节，不是可读取的文本：{input}"
        )));
    }
    match std::str::from_utf8(bytes) {
        Ok(value) => Ok(value.to_owned()),
        Err(error) if error.error_len().is_none() && allow_incomplete_tail => {
            let valid = std::str::from_utf8(&bytes[..error.valid_up_to()])
                .map_err(|_| OttoError::Tool(format!("文件不是有效的 UTF-8 文本：{input}")))?;
            Ok(valid.to_owned())
        }
        Err(_) => Err(OttoError::Tool(format!(
            "文件不是有效的 UTF-8 文本：{input}"
        ))),
    }
}

fn read_fingerprint(metadata: &fs::Metadata) -> String {
    let modified = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |duration| duration.as_nanos());
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        format!(
            "{:x}-{:x}-{:x}-{:x}",
            metadata.len(),
            modified,
            metadata.dev(),
            metadata.ino()
        )
    }
    #[cfg(not(unix))]
    {
        format!("{:x}-{:x}", metadata.len(), modified)
    }
}

fn read_bytes(
    path: &Path,
    input: &str,
    file_size: u64,
    fingerprint: &str,
    requested_start: u64,
    requested_length: Option<usize>,
    output_limit: usize,
) -> Result<String> {
    if requested_start > file_size {
        return Err(OttoError::Tool(format!(
            "byte_start {requested_start} 超出文件长度 {file_size}"
        )));
    }
    let mut file = fs::File::open(path)?;
    let mut actual_start = requested_start;
    if actual_start > 0 && actual_start < file_size {
        file.seek(SeekFrom::Start(actual_start))?;
        let mut first = [0u8; 1];
        file.read_exact(&mut first)?;
        let mut skipped = 0;
        while first[0] & 0b1100_0000 == 0b1000_0000 && skipped < 3 {
            actual_start = actual_start.saturating_add(1);
            skipped += 1;
            if actual_start >= file_size {
                break;
            }
            file.read_exact(&mut first)?;
        }
    }
    file.seek(SeekFrom::Start(actual_start))?;
    let maximum = requested_length.unwrap_or(output_limit).min(output_limit);
    let mut bytes = Vec::with_capacity(maximum.min(MAX_READ_CONTENT_BYTES));
    file.take(maximum as u64).read_to_end(&mut bytes)?;
    let at_file_end = actual_start.saturating_add(bytes.len() as u64) >= file_size;
    let content = decode_read_fragment(&bytes, input, !at_file_end)?;
    let valid_bytes = content.len();
    let next_byte = actual_start.saturating_add(valid_bytes as u64);
    let has_more = next_byte < file_size;
    Ok(format!(
        "[file: {input}]\n[mode: byte-range; file_bytes: {file_size}; file_fingerprint: {fingerprint}; requested_start: {requested_start}; actual_start: {actual_start}; returned_bytes: {valid_bytes}; truncated: {has_more}]\n{content}{}",
        if has_more {
            format!("\n[read_truncated: true; next_byte: {next_byte}]")
        } else {
            "\n[read_truncated: false]".to_owned()
        }
    ))
}

fn read_lines(
    path: &Path,
    input: &str,
    file_size: u64,
    fingerprint: &str,
    line_start: usize,
    line_end: Option<usize>,
    output_limit: usize,
) -> Result<String> {
    let file = fs::File::open(path)?;
    let mut reader = BufReader::with_capacity(16 * 1024, file);
    let mut byte_offset = 0u64;
    let mut line_number = 1usize;
    while line_number < line_start {
        match skip_one_line(&mut reader, MAX_LINE_SKIP_BYTES.saturating_sub(byte_offset))? {
            SkippedLine::Complete(consumed) => {
                byte_offset = byte_offset.saturating_add(consumed);
                line_number = line_number.saturating_add(1);
            }
            SkippedLine::Eof => break,
            SkippedLine::LimitReached => {
                return Err(OttoError::Tool(format!(
                "定位到第 {line_start} 行需要扫描超过 {} MiB；请使用 byte_start 或 grep 缩小范围",
                MAX_LINE_SKIP_BYTES / 1024 / 1024
            )))
            }
        }
    }

    let mut rendered = String::new();
    let mut returned_lines = 0usize;
    let mut truncated = false;
    let mut next_line = line_number;
    let mut next_byte = byte_offset;
    let mut at_eof = false;
    loop {
        if line_end.is_some_and(|end| line_number > end) {
            break;
        }
        let prefix = format!("{line_number}: ");
        let remaining = output_limit.saturating_sub(rendered.len());
        if remaining <= prefix.len() {
            truncated = true;
            next_line = line_number;
            next_byte = byte_offset;
            break;
        }
        let line = read_line_limited(&mut reader, remaining - prefix.len())?;
        if line.eof {
            at_eof = true;
            break;
        }
        byte_offset = byte_offset.saturating_add(line.consumed);
        let mut raw = line.bytes;
        let raw_length = raw.len();
        if raw.last() == Some(&b'\n') {
            raw.pop();
        }
        if raw.last() == Some(&b'\r') {
            raw.pop();
        }
        let text = decode_read_fragment(&raw, input, line.truncated)?;
        if !rendered.is_empty() {
            rendered.push('\n');
        }
        rendered.push_str(&prefix);
        rendered.push_str(&text);
        returned_lines += 1;
        if line.truncated {
            truncated = true;
            next_line = line_number;
            next_byte = byte_offset.saturating_sub(raw_length.saturating_sub(text.len()) as u64);
            break;
        }
        if line.terminated || !line.eof {
            line_number = line_number.saturating_add(1);
            next_line = line_number;
            next_byte = byte_offset;
        }
        if !line.terminated {
            at_eof = true;
            break;
        }
    }

    let has_more = if truncated {
        true
    } else if line_end.is_some_and(|end| line_number > end) {
        !reader.fill_buf()?.is_empty()
    } else {
        !at_eof && !reader.fill_buf()?.is_empty()
    };
    let truncated = truncated || has_more;
    Ok(format!(
        "[file: {input}]\n[mode: line-range; file_bytes: {file_size}; file_fingerprint: {fingerprint}; lines: {line_start}–{}; returned_lines: {returned_lines}; truncated: {truncated}]\n{}{}",
        if returned_lines == 0 {
            line_start
        } else {
            line_number.saturating_sub(1)
        },
        rendered,
        if truncated {
            format!("\n[read_truncated: true; next_line: {next_line}; next_byte: {next_byte}]")
        } else {
            "\n[read_truncated: false]".to_owned()
        }
    ))
}

fn preview_excerpt(value: &str) -> String {
    if value.len() <= MAX_PREVIEW_BYTES {
        return value.to_owned();
    }
    let mut end = MAX_PREVIEW_BYTES;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    format!(
        "{}\n… 预览已截断，完整变更共 {} 字节",
        &value[..end],
        value.len()
    )
}

fn estimate_replacement_size(
    original_len: usize,
    old_len: usize,
    new_len: usize,
    occurrence_count: usize,
    replace_all: bool,
) -> Result<usize> {
    let replacements = if replace_all { occurrence_count } else { 1 };
    let removed = old_len
        .checked_mul(replacements)
        .ok_or_else(|| OttoError::Tool("编辑结果大小计算溢出".to_owned()))?;
    let added = new_len
        .checked_mul(replacements)
        .ok_or_else(|| OttoError::Tool("编辑结果大小计算溢出".to_owned()))?;
    original_len
        .checked_sub(removed)
        .and_then(|size| size.checked_add(added))
        .ok_or_else(|| OttoError::Tool("编辑结果大小计算溢出".to_owned()))
}

fn workspace_relative(context: &ToolContext<'_>, path: &Path) -> String {
    context.workspace.display(path)
}

fn output_json(value: Value) -> ToolOutput {
    ToolOutput::text(
        serde_json::to_string_pretty(&value)
            .unwrap_or_else(|_| "{\"error\":\"无法序列化工具结果\"}".to_owned()),
    )
}

fn build_matcher(pattern: &str) -> Result<globset::GlobMatcher> {
    GlobBuilder::new(pattern)
        .literal_separator(false)
        .case_insensitive(false)
        .build()
        .map(|glob| glob.compile_matcher())
        .map_err(|error| OttoError::Tool(format!("Glob 模式无效：{error}")))
}

fn search_root(context: &ToolContext<'_>, value: Option<&str>) -> Result<PathBuf> {
    context.workspace.resolve_directory(value)
}

fn search_target(context: &ToolContext<'_>, value: Option<&str>) -> Result<PathBuf> {
    match value.filter(|value| !value.trim().is_empty()) {
        Some(value) => {
            let path = context.workspace.resolve_existing(value)?;
            if !path.is_file() && !path.is_dir() {
                return Err(OttoError::Tool(format!("搜索范围不是文件或目录：{value}")));
            }
            Ok(path)
        }
        None => search_root(context, None),
    }
}

fn relative_to(path: &Path, root: &Path) -> Option<PathBuf> {
    path.strip_prefix(root).ok().map(Path::to_path_buf)
}

fn sha256(content: &[u8]) -> String {
    let digest = Sha256::digest(content);
    format!("{digest:x}")
}

fn check_expected_hash(content: &[u8], expected: Option<&str>) -> Result<()> {
    if let Some(expected) = expected {
        let expected = expected.trim().to_ascii_lowercase();
        if expected.len() != 64 || !expected.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(OttoError::Tool(
                "expected_sha256 必须是 64 位十六进制哈希".to_owned(),
            ));
        }
        let actual = sha256(content);
        if actual != expected {
            return Err(OttoError::Tool(format!(
                "文件内容已变化，哈希校验失败：expected={expected}, actual={actual}"
            )));
        }
    }
    Ok(())
}

fn atomic_write(path: &Path, content: &[u8], create_only: bool) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| OttoError::Tool("无法确定写入文件的父目录".to_owned()))?;
    fs::create_dir_all(parent)?;

    let existing_permissions = fs::metadata(path)
        .ok()
        .map(|metadata| metadata.permissions());
    let temporary = NamedTempFile::new_in(parent)?;
    if let Some(permissions) = existing_permissions {
        temporary.as_file().set_permissions(permissions)?;
    } else {
        set_private_permissions(temporary.as_file())?;
    }
    temporary.as_file().write_all(content)?;
    temporary.as_file().sync_all()?;
    if create_only {
        temporary.persist_noclobber(path).map_err(|error| {
            if error.error.kind() == std::io::ErrorKind::AlreadyExists {
                OttoError::Tool("写入预览后目标文件已创建，已拒绝覆盖".to_owned())
            } else {
                OttoError::Tool(format!("创建文件 {} 失败：{}", path.display(), error.error))
            }
        })?;
    } else {
        temporary.persist(path).map_err(|error| {
            OttoError::Tool(format!("保存文件 {} 失败：{}", path.display(), error.error))
        })?;
    }
    Ok(())
}

fn set_private_permissions(file: &std::fs::File) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

fn edit_diff(path: &str, original: &str, updated: &str) -> String {
    TextDiff::from_lines(original, updated)
        .unified_diff()
        .context_radius(3)
        .header(&format!("a/{path}"), &format!("b/{path}"))
        .to_string()
}

fn bounded_edit_diff(
    path: &str,
    original: &str,
    updated: &str,
    changed_old: &str,
    changed_new: &str,
) -> String {
    if original
        .lines()
        .count()
        .saturating_add(updated.lines().count())
        > MAX_DIFF_LINES
    {
        return format!(
            "--- a/{path}\n+++ b/{path}\n(diff omitted: 文件行数超过安全预览上限)\n-{}\n+{}",
            preview_excerpt(changed_old),
            preview_excerpt(changed_new)
        );
    }
    edit_diff(path, original, updated)
}

#[async_trait::async_trait]
impl Tool for GlobTool {
    fn name(&self) -> &'static str {
        "glob"
    }

    fn definition(&self) -> Value {
        function_definition(
            self.name(),
            "在 workspace 内按 Glob 模式查找文件。只返回相对路径，不读取文件内容。",
            json!({
                "pattern": {"type": "string", "description": "例如 **/*.rs 或 src/*.rs"},
                "path": {"type": "string", "description": "可选的 workspace 内相对目录"},
                "max_results": {"type": "integer", "minimum": 1, "maximum": 1000}
            }),
            &["pattern"],
        )
    }

    async fn execute(&self, arguments: Value, context: &mut ToolContext<'_>) -> Result<ToolOutput> {
        let arguments = arguments_object(arguments)?;
        let pattern = required_string(&arguments, "pattern")?;
        let path = optional_string(&arguments, "path")?;
        let max_results = optional_usize(&arguments, "max_results", 100, MAX_RESULTS)?;
        authorize(
            context,
            self.name(),
            Capability::Read,
            &format!(
                "使用 Glob 查找 {} 下的 {}",
                path.as_deref().unwrap_or("."),
                pattern
            ),
        )?;
        let scope = search_root(context, path.as_deref())?;
        let matcher = build_matcher(&pattern)?;
        let mut results = Vec::new();
        let mut truncated = false;

        for entry in WalkDir::new(&scope)
            .follow_links(false)
            .max_depth(32)
            .into_iter()
            .filter_entry(|entry| !context.workspace.is_session_storage_path(entry.path()))
            .filter_map(|entry| entry.ok())
        {
            let file_type = entry.file_type();
            if !file_type.is_file() {
                continue;
            }
            let Some(relative) = relative_to(entry.path(), &scope) else {
                continue;
            };
            if !matcher.is_match(&relative) {
                continue;
            }
            results.push(workspace_relative(context, entry.path()));
            if results.len() >= max_results {
                truncated = true;
                break;
            }
        }

        results.sort();
        Ok(output_json(json!({
            "pattern": pattern,
            "path": path.unwrap_or_else(|| ".".to_owned()),
            "matches": results,
            "truncated": truncated
        })))
    }
}

#[async_trait::async_trait]
impl Tool for GrepTool {
    fn name(&self) -> &'static str {
        "grep"
    }

    fn definition(&self) -> Value {
        function_definition(
            self.name(),
            "在 workspace 内的 UTF-8 文本文件中搜索正则表达式，返回文件、行号和匹配行。",
            json!({
                "pattern": {"type": "string", "description": "Rust regex 表达式"},
                "path": {"type": "string", "description": "可选的 workspace 内相对目录或文件"},
                "include": {"type": "string", "description": "可选的文件 Glob，例如 *.rs"},
                "case_sensitive": {"type": "boolean", "default": true},
                "max_results": {"type": "integer", "minimum": 1, "maximum": 1000}
            }),
            &["pattern"],
        )
    }

    async fn execute(&self, arguments: Value, context: &mut ToolContext<'_>) -> Result<ToolOutput> {
        let arguments = arguments_object(arguments)?;
        let pattern = required_string(&arguments, "pattern")?;
        let path = optional_string(&arguments, "path")?;
        let include = optional_string(&arguments, "include")?;
        let case_sensitive = optional_bool(&arguments, "case_sensitive", true)?;
        let max_results = optional_usize(&arguments, "max_results", 100, MAX_RESULTS)?;
        authorize(
            context,
            self.name(),
            Capability::Read,
            &format!("使用 Grep 搜索 {} 下的内容", path.as_deref().unwrap_or(".")),
        )?;
        let scope = search_target(context, path.as_deref())?;
        let regex = RegexBuilder::new(&pattern)
            .case_insensitive(!case_sensitive)
            .build()
            .map_err(|error| OttoError::Tool(format!("Grep 正则表达式无效：{error}")))?;
        let include_matcher = include.as_deref().map(build_matcher).transpose()?;
        let mut matches = Vec::new();
        let mut truncated = false;

        for entry in WalkDir::new(&scope)
            .follow_links(false)
            .max_depth(32)
            .into_iter()
            .filter_entry(|entry| !context.workspace.is_session_storage_path(entry.path()))
            .filter_map(|entry| entry.ok())
        {
            if !entry.file_type().is_file() {
                continue;
            }
            let Some(relative) = relative_to(entry.path(), &scope) else {
                continue;
            };
            if let Some(matcher) = &include_matcher {
                let filename_matches = relative
                    .file_name()
                    .map(|name| matcher.is_match(Path::new(name)))
                    .unwrap_or(false);
                if !matcher.is_match(&relative) && !filename_matches {
                    continue;
                }
            }
            let Ok(metadata) = entry.metadata() else {
                continue;
            };
            if metadata.len() > MAX_GREP_FILE_BYTES {
                continue;
            }
            let content = match fs::read_to_string(entry.path()) {
                Ok(content) => content,
                Err(_) => continue,
            };
            let display = workspace_relative(context, entry.path());
            for (line_number, line) in content.lines().enumerate() {
                if regex.is_match(line) {
                    matches.push(format!("{display}:{}: {line}", line_number + 1));
                    if matches.len() >= max_results {
                        truncated = true;
                        break;
                    }
                }
            }
            if truncated {
                break;
            }
        }

        Ok(output_json(json!({
            "pattern": pattern,
            "matches": matches,
            "truncated": truncated
        })))
    }
}

#[async_trait::async_trait]
impl Tool for ReadTool {
    fn name(&self) -> &'static str {
        "read"
    }

    fn definition(&self) -> Value {
        function_definition(
            self.name(),
            "流式读取 workspace 内的大型 UTF-8 文本文件。默认有界读取；结果被截断时使用 next_line 或 next_byte，并传回 expected_file_fingerprint 以确认文件未变化。",
            json!({
                "path": {"type": "string", "description": "workspace 内相对文件路径"},
                "line_start": {"type": "integer", "minimum": 1},
                "line_end": {"type": "integer", "minimum": 1},
                "byte_start": {"type": "integer", "minimum": 0, "description": "用于续读超长行或读取 minified 文件的字节偏移"},
                "byte_length": {"type": "integer", "minimum": 1, "maximum": 67108864, "description": "字节模式下最多请求读取的字节数；单次返回仍受安全预算限制"},
                "expected_file_fingerprint": {"type": "string", "description": "续读时传回上次结果中的 fingerprint；文件变化时拒绝拼接两个版本"}
            }),
            &["path"],
        )
    }

    async fn execute(&self, arguments: Value, context: &mut ToolContext<'_>) -> Result<ToolOutput> {
        let arguments = arguments_object(arguments)?;
        let input = required_string(&arguments, "path")?;
        let line_start = optional_usize(&arguments, "line_start", 1, 1_000_000)?;
        let line_end = arguments
            .get("line_end")
            .filter(|value| !value.is_null())
            .map(|_| optional_usize(&arguments, "line_end", 1_000_000, 1_000_000))
            .transpose()?;
        let byte_start = arguments
            .get("byte_start")
            .filter(|value| !value.is_null())
            .map(|value| {
                value
                    .as_u64()
                    .ok_or_else(|| OttoError::Tool("参数 byte_start 必须是非负整数".to_owned()))
            })
            .transpose()?;
        let byte_length = arguments
            .get("byte_length")
            .filter(|value| !value.is_null())
            .map(|_| {
                optional_usize(
                    &arguments,
                    "byte_length",
                    MAX_READ_RANGE_BYTES,
                    MAX_READ_RANGE_BYTES,
                )
            })
            .transpose()?;
        let expected_fingerprint = optional_string(&arguments, "expected_file_fingerprint")?;
        let byte_mode = byte_start.is_some() || byte_length.is_some();
        if byte_mode && (line_start != 1 || line_end.is_some()) {
            return Err(OttoError::Tool(
                "byte_start/byte_length 不能与 line_start/line_end 同时使用".to_owned(),
            ));
        }
        if line_end.is_some_and(|line_end| line_start > line_end) {
            return Err(OttoError::Tool("line_start 不能大于 line_end".to_owned()));
        }
        let in_otto_storage = context.workspace.otto_relative_path(&input)?.is_some();
        if !in_otto_storage {
            authorize(
                context,
                self.name(),
                Capability::Read,
                &format!("读取本地文本文件 {input}"),
            )?;
        }
        let path = if in_otto_storage {
            context
                .workspace
                .resolve_otto_workspace_existing(&input)?
                .ok_or_else(|| OttoError::Tool("无法解析 .otto 文件路径".to_owned()))?
        } else {
            context.workspace.resolve_existing(&input)?
        };
        require_file(&path, &input)?;
        let metadata = fs::metadata(&path)?;
        let file_size = metadata.len();
        let fingerprint = read_fingerprint(&metadata);
        if expected_fingerprint
            .as_deref()
            .is_some_and(|expected| expected != fingerprint)
        {
            return Err(OttoError::Tool(
                "文件已变化，拒绝把新内容拼接到之前的读取结果；请从头读取".to_owned(),
            ));
        }
        let remaining = *context.remaining_tool_output_bytes;
        if remaining < 512 {
            return Err(OttoError::Tool(
                "本轮工具输出预算不足，无法读取文件内容".to_owned(),
            ));
        }
        let output_limit = MAX_READ_CONTENT_BYTES.min(remaining.saturating_sub(1024));
        let display_path = workspace_relative(context, &path);
        let content = if byte_mode {
            read_bytes(
                &path,
                &display_path,
                file_size,
                &fingerprint,
                byte_start.unwrap_or_default(),
                byte_length,
                output_limit,
            )?
        } else {
            read_lines(
                &path,
                &display_path,
                file_size,
                &fingerprint,
                line_start,
                line_end,
                output_limit,
            )?
        };
        if read_fingerprint(&fs::metadata(&path)?) != fingerprint {
            return Err(OttoError::Tool(
                "文件在读取期间发生变化，本次结果已丢弃；请重新读取".to_owned(),
            ));
        }
        Ok(ToolOutput::text(content))
    }
}

#[async_trait::async_trait]
impl Tool for EditTool {
    fn name(&self) -> &'static str {
        "edit"
    }

    fn definition(&self) -> Value {
        function_definition(
            self.name(),
            "在 workspace 内的 UTF-8 文本文件中进行精确文本替换，并返回 unified diff；workspace 根目录 .otto 内的普通数据默认允许修改，.otto/sessions 由 OTTO 内部保留。",
            json!({
                "path": {"type": "string"},
                "old_text": {"type": "string"},
                "new_text": {"type": "string"},
                "replace_all": {"type": "boolean", "default": false},
                "expected_sha256": {"type": "string", "description": "可选，写入前校验原文件哈希"}
            }),
            &["path", "old_text", "new_text"],
        )
    }

    async fn execute(&self, arguments: Value, context: &mut ToolContext<'_>) -> Result<ToolOutput> {
        let arguments = arguments_object(arguments)?;
        let input = required_string(&arguments, "path")?;
        let old_text = required_string_allow_empty(&arguments, "old_text")?;
        if old_text.is_empty() {
            return Err(OttoError::Tool("old_text 不能为空".to_owned()));
        }
        let new_text = required_string_allow_empty(&arguments, "new_text")?;
        let replace_all = optional_bool(&arguments, "replace_all", false)?;
        let expected_sha256 = optional_string(&arguments, "expected_sha256")?;
        let in_otto_storage = context.workspace.otto_relative_path(&input)?.is_some();
        let _write_guard = FILE_WRITE_LOCK
            .lock()
            .map_err(|_| OttoError::Tool("文件写入协调锁不可用".to_owned()))?;
        let path = if in_otto_storage {
            context
                .workspace
                .resolve_otto_workspace_existing(&input)?
                .ok_or_else(|| OttoError::Tool("无法解析 .otto 文件路径".to_owned()))?
        } else {
            context.workspace.resolve_existing(&input)?
        };
        require_file(&path, &input)?;
        let original = read_utf8(&path, &input, MAX_EDIT_BYTES)?;
        let original_bytes = original.as_bytes();
        check_expected_hash(&original_bytes, expected_sha256.as_deref())?;
        let occurrences = original.match_indices(&old_text).count();
        if occurrences == 0 {
            return Err(OttoError::Tool(format!(
                "在文件 {input} 中没有找到 old_text"
            )));
        }
        if occurrences > 1 && !replace_all {
            return Err(OttoError::Tool(format!(
                "old_text 在文件 {input} 中出现 {occurrences} 次，请提供更精确的片段或设置 replace_all=true"
            )));
        }
        let replacements = if replace_all { occurrences } else { 1 };
        let updated_size = estimate_replacement_size(
            original.len(),
            old_text.len(),
            new_text.len(),
            occurrences,
            replace_all,
        )?;
        if updated_size > MAX_EDIT_BYTES as usize {
            return Err(OttoError::Tool(format!(
                "编辑结果超过 {} MiB 限制，尚未写入",
                MAX_EDIT_BYTES / 1024 / 1024
            )));
        }
        let updated = if replace_all {
            original.replace(&old_text, &new_text)
        } else {
            original.replacen(&old_text, &new_text, 1)
        };
        let diff = bounded_edit_diff(&input, &original, &updated, &old_text, &new_text);
        if !in_otto_storage {
            context.permissions.authorize_with_preview(
                self.name(),
                Capability::Write,
                context.mode,
                &format!("修改本地文本文件 {input} · {replacements} 处替换"),
                Some(&preview_excerpt(&diff)),
            )?;
        }
        let current_path = if in_otto_storage {
            context.workspace.resolve_otto_workspace_existing(&input)?
        } else {
            Some(context.workspace.resolve_existing(&input)?)
        }
        .ok_or_else(|| OttoError::Tool("编辑目标在预览后消失".to_owned()))?;
        if current_path != path
            || read_utf8(&path, &input, MAX_EDIT_BYTES)?.as_bytes() != original_bytes
        {
            return Err(OttoError::Tool(format!(
                "文件 {input} 在预览后发生变化，已放弃写入"
            )));
        }
        atomic_write(&path, updated.as_bytes(), false)?;
        Ok(ToolOutput::text(
            serde_json::to_string_pretty(&json!({
                "path": input,
                "replacements": if replace_all { occurrences } else { 1 },
                "sha256": sha256(updated.as_bytes()),
                "diff": diff
            }))
            .unwrap_or_else(|_| "{\"error\":\"无法序列化编辑结果\"}".to_owned()),
        ))
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::{
        atomic_write, bounded_edit_diff, estimate_replacement_size, read_utf8, MAX_DIFF_LINES,
        MAX_EDIT_BYTES,
    };

    #[test]
    fn estimates_replacement_growth_and_overflow() {
        assert_eq!(estimate_replacement_size(4, 1, 3, 4, true).unwrap(), 12);
        assert_eq!(estimate_replacement_size(4, 1, 3, 4, false).unwrap(), 6);
        assert!(estimate_replacement_size(usize::MAX, 1, usize::MAX, 2, true).is_err());
    }

    #[test]
    fn bounded_file_reader_rejects_growth_past_limit() {
        let directory = tempdir().expect("temporary directory");
        let path = directory.path().join("large.txt");
        fs::write(&path, vec![b'a'; MAX_EDIT_BYTES as usize + 1]).expect("large file");
        assert!(read_utf8(&path, "large.txt", MAX_EDIT_BYTES).is_err());
    }

    #[test]
    fn create_only_write_does_not_replace_a_new_target() {
        let directory = tempdir().expect("temporary directory");
        let path = directory.path().join("new.txt");
        fs::write(&path, "created concurrently").expect("existing target");
        assert!(atomic_write(&path, b"previewed content", true).is_err());
        assert_eq!(
            fs::read_to_string(path).expect("preserved file"),
            "created concurrently"
        );
    }

    #[test]
    fn avoids_building_a_line_diff_past_the_preview_budget() {
        let many_lines = "x\n".repeat(MAX_DIFF_LINES / 2 + 1);
        let diff = bounded_edit_diff("large.txt", &many_lines, &many_lines, "x", "y");
        assert!(diff.contains("超过安全预览上限"));
        assert!(diff.contains("-x"));
        assert!(diff.contains("+y"));
    }
}

#[async_trait::async_trait]
impl Tool for WriteTool {
    fn name(&self) -> &'static str {
        "write"
    }

    fn definition(&self) -> Value {
        function_definition(
            self.name(),
            "在 workspace 内创建或覆盖 UTF-8 文本文件；workspace 根目录 .otto 内的普通数据默认允许写入，其他路径写入前会请求授权。.otto/sessions 由 OTTO 内部保留。",
            json!({
                "path": {"type": "string"},
                "content": {"type": "string"},
                "expected_sha256": {"type": "string", "description": "可选，覆盖已有文件前校验原文件哈希"}
            }),
            &["path", "content"],
        )
    }

    async fn execute(&self, arguments: Value, context: &mut ToolContext<'_>) -> Result<ToolOutput> {
        let arguments = arguments_object(arguments)?;
        let input = required_string(&arguments, "path")?;
        let content = arguments
            .get("content")
            .and_then(Value::as_str)
            .ok_or_else(|| OttoError::Tool("缺少字符串参数：content".to_owned()))?;
        let expected_sha256 = optional_string(&arguments, "expected_sha256")?;
        if content.len() > MAX_EDIT_BYTES as usize {
            return Err(OttoError::Tool("写入内容超过 4 MiB 限制".to_owned()));
        }
        let in_otto_storage = context.workspace.otto_relative_path(&input)?.is_some();
        let _write_guard = FILE_WRITE_LOCK
            .lock()
            .map_err(|_| OttoError::Tool("文件写入协调锁不可用".to_owned()))?;
        let path = if in_otto_storage {
            context
                .workspace
                .resolve_otto_workspace_for_create(&input)?
                .ok_or_else(|| OttoError::Tool("无法解析 .otto 文件路径".to_owned()))?
        } else {
            context.workspace.resolve_for_create(&input)?
        };
        let existing = match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(OttoError::Tool("拒绝覆盖符号链接".to_owned()))
            }
            Ok(metadata) if !metadata.is_file() => {
                return Err(OttoError::Tool("写入目标不是普通文件".to_owned()))
            }
            Ok(_) => Some(read_utf8(&path, &input, MAX_EDIT_BYTES)?),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(error.into()),
        };
        if let Some(existing) = &existing {
            check_expected_hash(existing.as_bytes(), expected_sha256.as_deref())?;
        } else if expected_sha256.is_some() {
            return Err(OttoError::Tool(
                "文件不存在时不能使用 expected_sha256".to_owned(),
            ));
        }
        let action = if existing.is_some() {
            "覆盖"
        } else {
            "新建"
        };
        let preview = if let Some(existing) = &existing {
            preview_excerpt(&bounded_edit_diff(
                &input, existing, content, existing, content,
            ))
        } else {
            let excerpt = preview_excerpt(content);
            preview_excerpt(&format!(
                "--- /dev/null\n+++ b/{input}\n{}",
                excerpt
                    .lines()
                    .map(|line| format!("+{line}\n"))
                    .collect::<String>()
            ))
        };
        if !in_otto_storage {
            context.permissions.authorize_with_preview(
                self.name(),
                Capability::Write,
                context.mode,
                &format!("{action}本地文本文件 {input} · {} 字节", content.len()),
                Some(&preview),
            )?;
        }
        let current_path = if in_otto_storage {
            if existing.is_some() {
                context.workspace.resolve_otto_workspace_existing(&input)?
            } else {
                context
                    .workspace
                    .resolve_otto_workspace_for_create(&input)?
            }
        } else if existing.is_some() {
            Some(context.workspace.resolve_existing(&input)?)
        } else {
            Some(context.workspace.resolve_for_create(&input)?)
        }
        .ok_or_else(|| OttoError::Tool("写入目标在预览后消失".to_owned()))?;
        if current_path != path {
            return Err(OttoError::Tool(
                "写入目标路径在预览后发生变化，已放弃写入".to_owned(),
            ));
        }
        match (&existing, fs::symlink_metadata(&path)) {
            (None, Err(error)) if error.kind() == std::io::ErrorKind::NotFound => {}
            (Some(_), Ok(metadata)) if metadata.is_file() && !metadata.file_type().is_symlink() => {
                if read_utf8(&path, &input, MAX_EDIT_BYTES)? != *existing.as_ref().unwrap() {
                    return Err(OttoError::Tool(
                        "文件在预览后发生变化，已放弃写入".to_owned(),
                    ));
                }
            }
            _ => {
                return Err(OttoError::Tool(
                    "写入目标在预览后发生变化，已放弃写入".to_owned(),
                ))
            }
        }
        atomic_write(&path, content.as_bytes(), existing.is_none())?;
        Ok(ToolOutput::text(
            serde_json::to_string_pretty(&json!({
                "path": input,
                "bytes": content.len(),
                "sha256": sha256(content.as_bytes())
            }))
            .unwrap_or_else(|_| "{\"error\":\"无法序列化写入结果\"}".to_owned()),
        ))
    }
}
