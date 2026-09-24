use std::fs::{self, File, OpenOptions, TryLockError};
use std::io::{BufReader, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tempfile::NamedTempFile;

use crate::error::{OttoError, Result};
use crate::usage::TokenUsage;
use crate::workspace::Workspace;

const SESSION_DIRECTORY: &str = "sessions";
const SESSION_SCHEMA_VERSION: u32 = 1;
const MAX_SESSION_FILE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_SESSION_LIST_ENTRIES: usize = 5_000;
const MAX_SESSION_LIST_BYTES: u64 = 256 * 1024 * 1024;

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
    /// Message boundary known to belong to the last closed turn. `None` means
    /// a legacy archive in which all stored messages predate checkpoints.
    #[serde(default)]
    pub completed_messages: Option<usize>,
    #[serde(default)]
    pub usage: TokenUsage,
}

pub struct SessionLease {
    directory: PathBuf,
    _lock: File,
    pub record: SessionRecord,
}

impl SessionLease {
    fn update_turn_messages(
        &mut self,
        completed_history: &[Value],
        current_turn: &[Value],
        completed: bool,
    ) -> Result<()> {
        let base = completed_history.len();
        if self.record.messages.len() < base || self.record.messages[..base] != *completed_history {
            return Err(storage_error("会话检查点与已保存历史不一致"));
        }
        self.record.messages.truncate(base);
        self.record.messages.extend_from_slice(current_turn);
        if completed {
            self.record.completed_messages = Some(self.record.messages.len());
        }
        Ok(())
    }

    pub fn checkpoint_turn_with_usage(
        &mut self,
        completed_history: &[Value],
        current_turn: &[Value],
        completed: bool,
        usage: &TokenUsage,
        usage_before_turn: &TokenUsage,
    ) -> Result<()> {
        self.update_turn_messages(completed_history, current_turn, completed)?;
        self.record.usage = usage_before_turn.clone();
        self.record.usage.add_assign(usage);
        self.commit()
    }

    pub fn commit(&mut self) -> Result<()> {
        validate_record(&self.record)?;
        self.record.updated_at = now();
        let path = session_path(&self.directory, &self.record.id);
        write_atomic(&path, &self.record)?;
        Ok(())
    }
}

impl SessionRecord {
    pub fn completed_message_count(&self) -> usize {
        self.completed_messages
            .unwrap_or(self.messages.len())
            .min(self.messages.len())
    }
}

pub fn list(workspace: &Workspace) -> Result<Vec<SessionRecord>> {
    let Some(directory) = existing_session_directory(workspace)? else {
        return Ok(Vec::new());
    };

    let mut records = Vec::new();
    let mut scanned_bytes = 0u64;
    let mut inspected_entries = 0usize;
    for entry in fs::read_dir(&directory)? {
        if inspected_entries >= MAX_SESSION_LIST_ENTRIES || scanned_bytes >= MAX_SESSION_LIST_BYTES
        {
            eprintln!("会话列表达到扫描上限，已停止读取其余存档。");
            break;
        }
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("json") {
            continue;
        }
        inspected_entries += 1;
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) => {
                eprintln!("会话存档无法检查，已跳过：{error}");
                continue;
            }
        };
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            eprintln!("会话存档不是普通文件，已跳过：{}", safe_file_name(&path));
            continue;
        }
        if metadata.len() > MAX_SESSION_FILE_BYTES
            || scanned_bytes.saturating_add(metadata.len()) > MAX_SESSION_LIST_BYTES
        {
            eprintln!(
                "会话存档超过列表读取预算，已跳过：{}",
                safe_file_name(&path)
            );
            continue;
        }
        scanned_bytes = scanned_bytes.saturating_add(metadata.len());
        let record: SessionListRecord =
            match serde_json::from_reader(BufReader::new(File::open(&path)?)) {
                Ok(record) => record,
                Err(error) => {
                    eprintln!("会话存档损坏，已跳过：{}（{error}）", safe_file_name(&path));
                    continue;
                }
            };
        if record.schema_version != SESSION_SCHEMA_VERSION
            || !is_uuid(&record.id)
            || record.description.trim().is_empty()
        {
            eprintln!("会话存档元数据无效，已跳过：{}", safe_file_name(&path));
            continue;
        }
        if path.file_stem().and_then(|stem| stem.to_str()) != Some(record.id.as_str()) {
            eprintln!("会话文件名与 ID 不匹配，已跳过：{}", safe_file_name(&path));
            continue;
        }
        records.push(SessionRecord {
            schema_version: record.schema_version,
            id: record.id,
            description: record.description,
            created_at: 0,
            updated_at: record.updated_at,
            messages: Vec::new(),
            context_summary: String::new(),
            summarized_messages: 0,
            completed_messages: None,
            usage: TokenUsage::default(),
        });
    }
    records.sort_by(|left, right| right.updated_at.cmp(&left.updated_at));
    Ok(records)
}

#[derive(Deserialize)]
struct SessionListRecord {
    schema_version: u32,
    id: String,
    description: String,
    updated_at: u64,
}

fn safe_file_name(path: &Path) -> String {
    path.file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect()
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
                completed_messages: Some(0),
                usage: TokenUsage::default(),
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

/// Pick a complete-turn boundary that respects both a message cap and a
/// conservative serialized-size token estimate. A single oversized latest
/// turn is summarized as a whole rather than left outside the rolling memory.
pub fn summary_boundary_for_budget(
    messages: &[Value],
    maximum_recent_messages: usize,
    maximum_recent_tokens: usize,
) -> usize {
    let Some(mut boundary) = messages
        .iter()
        .rposition(|message| message.get("role").and_then(Value::as_str) == Some("user"))
    else {
        return 0;
    };
    let mut recent_tokens = estimated_tokens(&messages[boundary..]);
    if recent_tokens > maximum_recent_tokens
        || messages.len().saturating_sub(boundary) > maximum_recent_messages
    {
        return messages.len();
    }
    loop {
        let Some(previous_user) = messages[..boundary]
            .iter()
            .rposition(|message| message.get("role").and_then(Value::as_str) == Some("user"))
        else {
            return 0;
        };
        let combined_tokens =
            recent_tokens.saturating_add(estimated_tokens(&messages[previous_user..boundary]));
        if messages.len().saturating_sub(previous_user) > maximum_recent_messages
            || combined_tokens > maximum_recent_tokens
        {
            return boundary;
        }
        boundary = previous_user;
        recent_tokens = combined_tokens;
    }
}

fn estimated_tokens(messages: &[Value]) -> usize {
    let bytes = messages
        .iter()
        .map(|message| serde_json::to_vec(message).map_or(usize::MAX, |json| json.len()))
        .fold(0usize, usize::saturating_add);
    bytes.saturating_add(1) / 2
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
    if record
        .completed_messages
        .is_some_and(|count| count > record.messages.len())
    {
        return Err(storage_error(&format!(
            "session {} 的完成边界超出消息历史",
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

#[cfg(test)]
mod tests {
    use std::fs;

    use serde_json::json;
    use tempfile::tempdir;

    use super::{list, open, start};
    use crate::workspace::Workspace;

    #[test]
    fn persists_open_and_completed_turn_boundaries() {
        let directory = tempdir().expect("workspace");
        let workspace = Workspace::new(Some(directory.path())).expect("workspace root");
        let mut lease = start(&workspace).expect("new session");
        lease.record.description = "checkpoint test".to_owned();
        let user = json!({"role":"user","content":"question"});
        lease
            .checkpoint_turn(&[], std::slice::from_ref(&user), false)
            .expect("pending checkpoint");
        assert_eq!(lease.record.completed_message_count(), 0);
        let answer = json!({"role":"assistant","content":"answer"});
        lease
            .checkpoint_turn(&[], &[user, answer], true)
            .expect("completed checkpoint");
        let id = lease.record.id.clone();
        drop(lease);

        let reopened = open(&workspace, &id).expect("reopen session");
        assert_eq!(reopened.record.completed_message_count(), 2);
        assert_eq!(reopened.record.messages.len(), 2);
    }

    #[test]
    fn lists_lightweight_metadata_and_skips_corrupt_records() {
        let directory = tempdir().expect("workspace");
        let workspace = Workspace::new(Some(directory.path())).expect("workspace root");
        let (otto, _) = workspace.ensure_otto_directory().expect(".otto directory");
        let sessions = otto.join("sessions");
        fs::create_dir(&sessions).expect("sessions directory");
        let id = "123e4567-e89b-42d3-a456-426614174000";
        fs::write(
            sessions.join(format!("{id}.json")),
            json!({
                "schema_version": 1,
                "id": id,
                "description": "kept",
                "created_at": 1,
                "updated_at": 2,
                "messages": [{"role":"user","content":"large history is ignored"}]
            })
            .to_string(),
        )
        .expect("valid session");
        fs::write(sessions.join("broken.json"), "{").expect("broken session");

        let records = list(&workspace).expect("session list");
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].id, id);
        assert!(records[0].messages.is_empty());
    }
}
