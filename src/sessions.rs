use std::fs::{self, File, OpenOptions, TryLockError};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tempfile::NamedTempFile;

use crate::error::{OttoError, Result};
use crate::workspace::Workspace;

const SESSION_DIRECTORY: &str = "sessions";
const SESSION_SCHEMA_VERSION: u32 = 1;
const MAX_SESSION_FILE_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionRecord {
    schema_version: u32,
    pub id: String,
    pub description: String,
    pub created_at: u64,
    pub updated_at: u64,
    pub messages: Vec<Value>,
    #[serde(default)]
    pub context_summary: String,
    #[serde(default)]
    pub summarized_messages: usize,
}

pub struct SessionLease {
    directory: PathBuf,
    _lock: File,
    pub record: SessionRecord,
}

impl SessionLease {
    pub fn commit(&mut self) -> Result<()> {
        validate_record(&self.record)?;
        self.record.updated_at = now();
        let path = session_path(&self.directory, &self.record.id);
        write_atomic(&path, &self.record)?;
        Ok(())
    }
}

pub fn list(workspace: &Workspace) -> Result<Vec<SessionRecord>> {
    let Some(directory) = existing_session_directory(workspace)? else {
        return Ok(Vec::new());
    };

    let mut records = Vec::new();
    for entry in fs::read_dir(&directory)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("json") {
            continue;
        }
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(storage_error(&format!(
                "会话存档不是普通文件：{}",
                path.display()
            )));
        }
        let record = read_record(&path)?;
        if path.file_stem().and_then(|stem| stem.to_str()) != Some(record.id.as_str()) {
            return Err(storage_error(&format!(
                "会话文件名与其 ID 不匹配：{}",
                path.display()
            )));
        }
        records.push(record);
    }
    records.sort_by(|left, right| right.updated_at.cmp(&left.updated_at));
    Ok(records)
}

pub fn start(workspace: &Workspace) -> Result<SessionLease> {
    let directory = ensure_session_directory(workspace)?;
    loop {
        let id = new_uuid()?;
        let lock = acquire_lock(&directory, &id)?;
        if session_path(&directory, &id).exists() {
            drop(lock);
            continue;
        }
        let timestamp = now();
        return Ok(SessionLease {
            directory,
            _lock: lock,
            record: SessionRecord {
                schema_version: SESSION_SCHEMA_VERSION,
                id,
                description: String::new(),
                created_at: timestamp,
                updated_at: timestamp,
                messages: Vec::new(),
                context_summary: String::new(),
                summarized_messages: 0,
            },
        });
    }
}

pub fn open(workspace: &Workspace, identifier: &str) -> Result<SessionLease> {
    let prefix = normalize_identifier(identifier)?;
    let Some(directory) = existing_session_directory(workspace)? else {
        return Err(OttoError::Usage(format!("找不到 session：{identifier}")));
    };
    let mut ids = Vec::new();
    for entry in fs::read_dir(&directory)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("json") {
            continue;
        }
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(storage_error(&format!(
                "会话存档不是普通文件：{}",
                path.display()
            )));
        }
        let Some(id) = path.file_stem().and_then(|stem| stem.to_str()) else {
            continue;
        };
        if is_uuid(id) && normalize_identifier(id)?.starts_with(&prefix) {
            ids.push(id.to_owned());
        }
    }
    ids.sort();

    let id = match ids.as_slice() {
        [] => return Err(OttoError::Usage(format!("找不到 session：{identifier}"))),
        [id] => id.clone(),
        ids => {
            let candidates = ids.join("\n");
            return Err(OttoError::Usage(format!(
                "session 前缀 {identifier} 不唯一，请提供更长前缀：\n{candidates}"
            )));
        }
    };

    let lock = acquire_lock(&directory, &id)?;
    let path = session_path(&directory, &id);
    let record = read_record(&path)?;
    if record.id != id {
        return Err(storage_error(&format!(
            "会话文件名与其 ID 不匹配：{}",
            path.display()
        )));
    }
    validate_record(&record)?;
    Ok(SessionLease {
        directory,
        _lock: lock,
        record,
    })
}

/// Return a complete-turn boundary that leaves at most `maximum_recent_messages`
/// in the recent history. The messages before this index are eligible for a
/// rolling summary.
pub fn summary_boundary(messages: &[Value], maximum_recent_messages: usize) -> usize {
    if messages.len() <= maximum_recent_messages {
        return 0;
    }
    let cutoff = messages.len() - maximum_recent_messages;
    (cutoff..messages.len())
        .find(|index| messages[*index].get("role").and_then(Value::as_str) == Some("user"))
        // A single unusually long turn may occupy the entire history. It is
        // still a complete turn because only completed turns are committed.
        // Summarize it whole instead of leaving it permanently outside both
        // the summary and the bounded recent context.
        .unwrap_or(messages.len())
}

fn existing_session_directory(workspace: &Workspace) -> Result<Option<PathBuf>> {
    let Some(otto_directory) = workspace.existing_otto_directory()? else {
        return Ok(None);
    };
    let directory = otto_directory.join(SESSION_DIRECTORY);
    match fs::symlink_metadata(&directory) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            Err(storage_error(".otto/sessions 不能是符号链接"))
        }
        Ok(metadata) if !metadata.is_dir() => Err(storage_error(".otto/sessions 必须是目录")),
        Ok(_) => Ok(Some(directory)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn ensure_session_directory(workspace: &Workspace) -> Result<PathBuf> {
    let (otto_directory, _) = workspace.ensure_otto_directory()?;
    let directory = otto_directory.join(SESSION_DIRECTORY);
    match fs::create_dir(&directory) {
        Ok(()) => set_private_directory_permissions(&directory)?,
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error.into()),
    }
    let metadata = fs::symlink_metadata(&directory)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(storage_error("拒绝使用非普通目录作为 .otto/sessions"));
    }
    set_private_directory_permissions(&directory)?;
    Ok(directory)
}

fn acquire_lock(directory: &Path, id: &str) -> Result<File> {
    let path = directory.join(format!("{id}.lock"));
    match fs::symlink_metadata(&path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            return Err(storage_error(&format!(
                "会话锁不是普通文件：{}",
                path.display()
            )))
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)?;
    set_private_file_permissions(&file)?;
    match file.try_lock() {
        Ok(()) => {}
        Err(TryLockError::WouldBlock) => {
            return Err(OttoError::Tool(format!(
                "session {id} 正被另一个 OTTO 请求使用，稍后重试"
            )))
        }
        Err(TryLockError::Error(error)) => return Err(error.into()),
    }
    Ok(file)
}

fn read_record(path: &Path) -> Result<SessionRecord> {
    let metadata = fs::metadata(path)?;
    if metadata.len() > MAX_SESSION_FILE_BYTES {
        return Err(storage_error(&format!(
            "会话存档超过 {} MiB：{}",
            MAX_SESSION_FILE_BYTES / 1024 / 1024,
            path.display()
        )));
    }
    let bytes = fs::read(path)?;
    let record: SessionRecord = serde_json::from_slice(&bytes)
        .map_err(|error| storage_error(&format!("无法读取会话存档 {}：{error}", path.display())))?;
    validate_record(&record)?;
    Ok(record)
}

fn validate_record(record: &SessionRecord) -> Result<()> {
    if record.schema_version != SESSION_SCHEMA_VERSION {
        return Err(storage_error(&format!(
            "session {} 使用不支持的数据版本 {}",
            record.id, record.schema_version
        )));
    }
    if !is_uuid(&record.id) {
        return Err(storage_error("会话存档包含无效的 UUID"));
    }
    if record.description.trim().is_empty() {
        return Err(storage_error(&format!(
            "session {} 缺少 description",
            record.id
        )));
    }
    if record.summarized_messages > record.messages.len() {
        return Err(storage_error(&format!(
            "session {} 的摘要边界超出消息历史",
            record.id
        )));
    }
    if record.summarized_messages > 0 && record.context_summary.trim().is_empty() {
        return Err(storage_error(&format!(
            "session {} 有摘要边界但缺少滚动摘要",
            record.id
        )));
    }
    if record.messages.iter().any(|message| {
        !matches!(
            message.get("role").and_then(Value::as_str),
            Some("user" | "assistant" | "tool")
        )
    }) {
        return Err(storage_error(&format!(
            "session {} 包含无效的消息角色",
            record.id
        )));
    }
    Ok(())
}

fn write_atomic(path: &Path, record: &SessionRecord) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(record)?;
    if bytes.len() as u64 > MAX_SESSION_FILE_BYTES {
        return Err(storage_error(&format!(
            "session {} 超过 {} MiB 存储限制",
            record.id,
            MAX_SESSION_FILE_BYTES / 1024 / 1024
        )));
    }
    let temporary = NamedTempFile::new_in(
        path.parent()
            .ok_or_else(|| storage_error("无法确定 session 存档目录"))?,
    )?;
    set_private_file_permissions(temporary.as_file())?;
    temporary.as_file().write_all(&bytes)?;
    temporary.as_file().sync_all()?;
    temporary.persist(path).map_err(|error| {
        storage_error(&format!("无法保存 session {}：{}", record.id, error.error))
    })?;
    Ok(())
}

fn session_path(directory: &Path, id: &str) -> PathBuf {
    directory.join(format!("{id}.json"))
}

fn normalize_identifier(identifier: &str) -> Result<String> {
    let normalized = identifier
        .chars()
        .filter(|character| *character != '-')
        .flat_map(char::to_lowercase)
        .collect::<String>();
    if normalized.is_empty() || !normalized.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(OttoError::Usage(
            "session ID 前缀只能包含 UUID 十六进制字符和连字符".to_owned(),
        ));
    }
    Ok(normalized)
}

fn is_uuid(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(index, byte)| match index {
            8 | 13 | 18 | 23 => byte == b'-',
            _ => byte.is_ascii_hexdigit(),
        })
}

fn new_uuid() -> Result<String> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes)
        .map_err(|error| OttoError::Io(std::io::Error::other(error.to_string())))?;
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut raw = String::with_capacity(32);
    for byte in bytes {
        raw.push(HEX[(byte >> 4) as usize] as char);
        raw.push(HEX[(byte & 0x0f) as usize] as char);
    }
    Ok(format!(
        "{}-{}-{}-{}-{}",
        &raw[0..8],
        &raw[8..12],
        &raw[12..16],
        &raw[16..20],
        &raw[20..32]
    ))
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn storage_error(message: &str) -> OttoError {
    OttoError::Tool(message.to_owned())
}

#[cfg(unix)]
fn set_private_directory_permissions(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
}

#[cfg(not(unix))]
fn set_private_directory_permissions(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

#[cfg(unix)]
fn set_private_file_permissions(file: &File) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    file.set_permissions(fs::Permissions::from_mode(0o600))
}

#[cfg(not(unix))]
fn set_private_file_permissions(_file: &File) -> std::io::Result<()> {
    Ok(())
}
