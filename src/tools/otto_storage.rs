use std::fs;
use std::io::Write;
use std::path::Path;

use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tempfile::NamedTempFile;
use walkdir::WalkDir;

use super::{
    arguments_object, function_definition, optional_string, optional_usize, required_string, Tool,
    ToolContext, ToolOutput,
};
use crate::error::{OttoError, Result};

const MAX_STORAGE_READ_BYTES: u64 = 512 * 1024;
const MAX_STORAGE_WRITE_BYTES: usize = 4 * 1024 * 1024;
const MAX_LIST_RESULTS: usize = 1000;

pub struct OttoStorageTool;

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

fn set_private_file_permissions(file: &fs::File) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
    }
    #[cfg(not(unix))]
    {
        let _ = file;
    }
    Ok(())
}

fn write_atomic(path: &Path, content: &[u8], replace: bool) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| OttoError::Tool("无法确定 .otto 文件的父目录".to_owned()))?;
    let existing_permissions = fs::metadata(path)
        .ok()
        .map(|metadata| metadata.permissions());
    let temporary = NamedTempFile::new_in(parent)?;
    if let Some(permissions) = existing_permissions {
        temporary.as_file().set_permissions(permissions)?;
    } else {
        set_private_file_permissions(temporary.as_file())?;
    }
    temporary.as_file().write_all(content)?;
    temporary.as_file().sync_all()?;

    if replace {
        temporary
            .persist(path)
            .map_err(|error| OttoError::Tool(format!("保存 .otto 文件失败：{}", error.error)))?;
    } else {
        temporary.persist_noclobber(path).map_err(|error| {
            if error.error.kind() == std::io::ErrorKind::AlreadyExists {
                OttoError::Tool("文件已存在；请使用 update 操作修改".to_owned())
            } else {
                OttoError::Tool(format!("创建 .otto 文件失败：{}", error.error))
            }
        })?;
    }
    Ok(())
}

fn read_file(path: &Path, display_path: &str) -> Result<String> {
    let metadata = fs::metadata(path)?;
    if !metadata.is_file() {
        return Err(OttoError::Tool(format!(
            ".otto 目标不是普通文件：{display_path}"
        )));
    }
    if metadata.len() > MAX_STORAGE_READ_BYTES {
        return Err(OttoError::Tool(format!(
            ".otto 文件 {display_path} 超过 {} KiB 读取限制",
            MAX_STORAGE_READ_BYTES / 1024
        )));
    }
    String::from_utf8(fs::read(path)?)
        .map_err(|_| OttoError::Tool(format!(".otto 文件不是有效的 UTF-8 文本：{display_path}")))
}

fn storage_path(path: &str) -> Result<&str> {
    let path = path.trim();
    if path.is_empty() {
        return Err(OttoError::Tool(".otto 文件路径不能为空".to_owned()));
    }
    Ok(path)
}

fn list_files(
    context: &ToolContext<'_>,
    path: Option<&str>,
    max_results: usize,
) -> Result<ToolOutput> {
    let root = match context.workspace.existing_otto_directory()? {
        Some(root) => root,
        None if path.is_none() || path == Some(".") => {
            return Ok(ToolOutput::text(
                "{\"directory\":\".otto\",\"files\":[],\"truncated\":false}".to_owned(),
            ))
        }
        None => {
            return Err(OttoError::Tool(
                "workspace 的 .otto 目录尚未创建".to_owned(),
            ))
        }
    };
    let target = match path.filter(|value| *value != ".") {
        Some(path) => context
            .workspace
            .resolve_otto_existing(storage_path(path)?)?,
        None => root.clone(),
    };
    if !target.is_dir() {
        return Err(OttoError::Tool(".otto 列表范围不是目录".to_owned()));
    }

    let mut files = Vec::new();
    let mut truncated = false;
    for entry in WalkDir::new(&target)
        .follow_links(false)
        .max_depth(32)
        .into_iter()
        .filter_entry(|entry| !context.workspace.is_session_storage_path(entry.path()))
        .filter_map(std::result::Result::ok)
    {
        if !entry.file_type().is_file() {
            continue;
        }
        let relative = entry
            .path()
            .strip_prefix(&root)
            .map_err(|_| OttoError::Tool("列表结果超出 .otto 目录".to_owned()))?;
        files.push(relative.display().to_string());
        if files.len() == max_results {
            truncated = true;
            break;
        }
    }
    files.sort();
    Ok(ToolOutput::text(
        serde_json::to_string_pretty(&json!({
            "directory": ".otto",
            "files": files,
            "truncated": truncated
        }))
        .unwrap_or_else(|_| "{\"error\":\"无法序列化 .otto 列表\"}".to_owned()),
    ))
}

fn read_storage_file(context: &ToolContext<'_>, input: &str) -> Result<ToolOutput> {
    let input = storage_path(input)?;
    let path = context.workspace.resolve_otto_existing(input)?;
    let content = read_file(&path, input)?;
    Ok(ToolOutput::text(format!("[.otto/{input}]\n{content}")))
}

fn create_storage_file(
    context: &ToolContext<'_>,
    input: &str,
    content: &str,
) -> Result<ToolOutput> {
    let input = storage_path(input)?;
    if content.len() > MAX_STORAGE_WRITE_BYTES {
        return Err(OttoError::Tool(format!(
            ".otto 写入内容超过 {} MiB 限制",
            MAX_STORAGE_WRITE_BYTES / 1024 / 1024
        )));
    }
    let path = context.workspace.resolve_otto_for_create(input)?;
    write_atomic(&path, content.as_bytes(), false)?;
    Ok(ToolOutput::text(
        serde_json::to_string_pretty(&json!({
            "path": format!(".otto/{input}"),
            "bytes": content.len(),
            "sha256": sha256(content.as_bytes())
        }))
        .unwrap_or_else(|_| "{\"error\":\"无法序列化写入结果\"}".to_owned()),
    ))
}

fn update_storage_file(
    context: &ToolContext<'_>,
    input: &str,
    content: &str,
    expected_sha256: Option<&str>,
) -> Result<ToolOutput> {
    let input = storage_path(input)?;
    if content.len() > MAX_STORAGE_WRITE_BYTES {
        return Err(OttoError::Tool(format!(
            ".otto 写入内容超过 {} MiB 限制",
            MAX_STORAGE_WRITE_BYTES / 1024 / 1024
        )));
    }
    let path = context.workspace.resolve_otto_existing(input)?;
    let metadata = fs::metadata(&path)?;
    if !metadata.is_file() {
        return Err(OttoError::Tool(format!(".otto 目标不是普通文件：{input}")));
    }
    if metadata.len() > MAX_STORAGE_WRITE_BYTES as u64 {
        return Err(OttoError::Tool(format!(
            ".otto 文件 {input} 超过 {} MiB 更新限制",
            MAX_STORAGE_WRITE_BYTES / 1024 / 1024
        )));
    }
    let original = fs::read(&path)?;
    check_expected_hash(&original, expected_sha256)?;
    if fs::read(&path)? != original {
        return Err(OttoError::Tool(format!(
            ".otto 文件 {input} 在更新前发生变化，已放弃写入"
        )));
    }
    write_atomic(&path, content.as_bytes(), true)?;
    Ok(ToolOutput::text(
        serde_json::to_string_pretty(&json!({
            "path": format!(".otto/{input}"),
            "bytes": content.len(),
            "sha256": sha256(content.as_bytes())
        }))
        .unwrap_or_else(|_| "{\"error\":\"无法序列化更新结果\"}".to_owned()),
    ))
}

fn delete_storage_file(context: &ToolContext<'_>, input: &str) -> Result<ToolOutput> {
    let input = storage_path(input)?;
    let path = context.workspace.resolve_otto_existing(input)?;
    if !fs::metadata(&path)?.is_file() {
        return Err(OttoError::Tool(format!(
            "只允许删除 .otto 中的普通文件：{input}"
        )));
    }
    fs::remove_file(&path)?;
    Ok(ToolOutput::text(format!("已删除 .otto/{input}")))
}

fn required_content(arguments: &serde_json::Map<String, Value>) -> Result<&str> {
    arguments
        .get("content")
        .and_then(Value::as_str)
        .ok_or_else(|| OttoError::Tool("缺少字符串参数：content".to_owned()))
}

#[async_trait::async_trait]
impl Tool for OttoStorageTool {
    fn name(&self) -> &'static str {
        "otto_storage"
    }

    fn definition(&self) -> Value {
        function_definition(
            self.name(),
            "管理当前 workspace 根目录下 .otto 中的普通 OTTO 持久化数据。所有 path 都相对于 .otto；仅操作该目录内的文件，不请求授权。.otto/sessions 是会话运行时保留区，不能通过此工具访问。",
            json!({
                "action": {
                    "type": "string",
                    "enum": ["init", "list", "read", "create", "update", "delete"]
                },
                "path": {
                    "type": "string",
                    "description": "相对于 .otto 的文件路径；list 可选，使用 . 表示根目录"
                },
                "content": {"type": "string", "description": "create 或 update 时写入的 UTF-8 文本"},
                "expected_sha256": {"type": "string", "description": "update 时可选的原文件哈希校验"},
                "max_results": {"type": "integer", "minimum": 1, "maximum": 1000}
            }),
            &["action"],
        )
    }

    async fn execute(&self, arguments: Value, context: &mut ToolContext<'_>) -> Result<ToolOutput> {
        let arguments = arguments_object(arguments)?;
        let action = required_string(&arguments, "action")?;
        match action.as_str() {
            "init" => {
                let (path, created) = context.workspace.ensure_otto_directory()?;
                Ok(ToolOutput::text(format!(
                    "{} .otto 目录 {}",
                    if created { "已创建" } else { "已存在" },
                    path.display()
                )))
            }
            "list" => {
                let path = optional_string(&arguments, "path")?;
                let max_results = optional_usize(&arguments, "max_results", 100, MAX_LIST_RESULTS)?;
                list_files(context, path.as_deref(), max_results)
            }
            "read" => read_storage_file(context, &required_string(&arguments, "path")?),
            "create" => create_storage_file(
                context,
                &required_string(&arguments, "path")?,
                required_content(&arguments)?,
            ),
            "update" => update_storage_file(
                context,
                &required_string(&arguments, "path")?,
                required_content(&arguments)?,
                optional_string(&arguments, "expected_sha256")?.as_deref(),
            ),
            "delete" => delete_storage_file(context, &required_string(&arguments, "path")?),
            _ => Err(OttoError::Tool(format!(
                "未知的 otto_storage 操作：{action}"
            ))),
        }
    }
}
