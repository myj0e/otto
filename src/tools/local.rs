use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

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

const MAX_READ_BYTES: u64 = 512 * 1024;
const MAX_EDIT_BYTES: u64 = 4 * 1024 * 1024;
const MAX_GREP_FILE_BYTES: u64 = 4 * 1024 * 1024;
const MAX_RESULTS: usize = 1000;

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
    let metadata = fs::metadata(path)?;
    if metadata.len() > maximum {
        return Err(OttoError::Tool(format!(
            "文件 {} 超过 {} KiB 限制",
            input,
            maximum / 1024
        )));
    }
    String::from_utf8(fs::read(path)?)
        .map_err(|_| OttoError::Tool(format!("文件不是有效的 UTF-8 文本：{input}")))
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

fn atomic_write(path: &Path, content: &[u8]) -> Result<()> {
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
    temporary.persist(path).map_err(|error| {
        OttoError::Tool(format!("保存文件 {} 失败：{}", path.display(), error.error))
    })?;
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
            "读取 workspace 内的 UTF-8 文本文件，可选地限定行范围。",
            json!({
                "path": {"type": "string", "description": "workspace 内相对文件路径"},
                "line_start": {"type": "integer", "minimum": 1},
                "line_end": {"type": "integer", "minimum": 1}
            }),
            &["path"],
        )
    }

    async fn execute(&self, arguments: Value, context: &mut ToolContext<'_>) -> Result<ToolOutput> {
        let arguments = arguments_object(arguments)?;
        let input = required_string(&arguments, "path")?;
        let line_start = optional_usize(&arguments, "line_start", 1, 1_000_000)?;
        let line_end = optional_usize(&arguments, "line_end", 1_000_000, 1_000_000)?;
        authorize(
            context,
            self.name(),
            Capability::Read,
            &format!("读取本地文本文件 {input}"),
        )?;
        let path = context.workspace.resolve_existing(&input)?;
        require_file(&path, &input)?;
        let content = read_utf8(&path, &input, MAX_READ_BYTES)?;
        let lines: Vec<&str> = content.lines().collect();
        if line_start > line_end {
            return Err(OttoError::Tool("line_start 不能大于 line_end".to_owned()));
        }
        let start = line_start.saturating_sub(1).min(lines.len());
        let end = line_end.min(lines.len());
        let selected = lines[start..end]
            .iter()
            .enumerate()
            .map(|(offset, line)| format!("{}: {}", start + offset + 1, line))
            .collect::<Vec<_>>()
            .join("\n");
        Ok(ToolOutput::text(format!(
            "[file: {}]\n{}",
            workspace_relative(context, &path),
            selected
        )))
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
            "在 workspace 内的 UTF-8 文本文件中进行精确文本替换，并返回 unified diff。",
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
        authorize(
            context,
            self.name(),
            Capability::Write,
            &format!("修改本地文本文件 {input}"),
        )?;
        let path = context.workspace.resolve_existing(&input)?;
        require_file(&path, &input)?;
        let original_bytes = fs::read(&path)?;
        if original_bytes.len() as u64 > MAX_EDIT_BYTES {
            return Err(OttoError::Tool(format!(
                "文件 {} 超过 {} MiB 限制",
                input,
                MAX_EDIT_BYTES / 1024 / 1024
            )));
        }
        check_expected_hash(&original_bytes, expected_sha256.as_deref())?;
        let original = String::from_utf8(original_bytes.clone())
            .map_err(|_| OttoError::Tool(format!("文件不是有效的 UTF-8 文本：{input}")))?;
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
        let updated = if replace_all {
            original.replace(&old_text, &new_text)
        } else {
            original.replacen(&old_text, &new_text, 1)
        };
        let current_bytes = fs::read(&path)?;
        if current_bytes != original_bytes {
            return Err(OttoError::Tool(format!(
                "文件 {input} 在修改前发生变化，已放弃写入"
            )));
        }
        atomic_write(&path, updated.as_bytes())?;
        let diff = edit_diff(&input, &original, &updated);
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

#[async_trait::async_trait]
impl Tool for WriteTool {
    fn name(&self) -> &'static str {
        "write"
    }

    fn definition(&self) -> Value {
        function_definition(
            self.name(),
            "在 workspace 内创建或覆盖 UTF-8 文本文件，写入前会请求用户授权。",
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
        authorize(
            context,
            self.name(),
            Capability::Write,
            &format!("创建或覆盖本地文本文件 {input}"),
        )?;
        let path = context.workspace.resolve_for_create(&input)?;
        if let Ok(existing) = fs::read(&path) {
            check_expected_hash(&existing, expected_sha256.as_deref())?;
        } else if expected_sha256.is_some() {
            return Err(OttoError::Tool(
                "文件不存在时不能使用 expected_sha256".to_owned(),
            ));
        }
        atomic_write(&path, content.as_bytes())?;
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
