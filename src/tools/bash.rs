use std::io;
use std::process::ExitStatus;
use std::process::Stdio;
use std::time::Duration;

use serde_json::{json, Map, Value};
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::{Child, Command};
use tokio::sync::mpsc;

use super::{arguments_object, function_definition, Tool, ToolContext, ToolOutput};
use crate::error::{OttoError, Result};
use crate::ui;

const MAX_COMMAND_BYTES: usize = 64 * 1024;
const MAX_CAPTURED_STREAM_BYTES: usize = 64 * 1024;
const COMMAND_TIMEOUT: Duration = Duration::from_secs(120);
const CHILD_CLEANUP_TIMEOUT: Duration = Duration::from_secs(3);

pub struct BashTool;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Risk {
    ReadOnly,
    MayModify,
    Uncertain,
}

#[derive(Debug)]
struct Assessment {
    risk: Risk,
    reason: String,
    breakdown: Vec<String>,
}

impl Assessment {
    fn uncertain(reason: impl Into<String>) -> Self {
        Self {
            risk: Risk::Uncertain,
            reason: reason.into(),
            breakdown: Vec::new(),
        }
    }

    fn ui_risk(&self) -> &'static str {
        match self.risk {
            Risk::ReadOnly => "只读判断",
            Risk::MayModify => "可能修改或产生外部影响",
            Risk::Uncertain => "无法确定，按可能产生影响处理",
        }
    }

    fn risk_key(&self) -> &'static str {
        match self.risk {
            Risk::ReadOnly => "read_only",
            Risk::MayModify => "may_modify",
            Risk::Uncertain => "uncertain",
        }
    }
}

fn parse_assessment(arguments: &Map<String, Value>) -> Assessment {
    let (Some(risk), Some(reason), Some(breakdown)) = (
        arguments.get("risk").and_then(Value::as_str),
        arguments.get("reason").and_then(Value::as_str),
        arguments.get("breakdown").and_then(Value::as_array),
    ) else {
        return Assessment::uncertain("模型风险判断缺少必要字段");
    };
    if reason.trim().is_empty() {
        return Assessment::uncertain("模型风险判断说明为空");
    }
    if breakdown.iter().any(|step| !step.is_string()) {
        return Assessment::uncertain("模型命令拆解格式无效");
    }

    let risk = match risk {
        "read_only" => Risk::ReadOnly,
        "may_modify" => Risk::MayModify,
        "uncertain" => Risk::Uncertain,
        _ => return Assessment::uncertain("模型返回了未知的风险类型"),
    };
    Assessment {
        risk,
        reason: reason.to_owned(),
        breakdown: breakdown
            .iter()
            .filter_map(Value::as_str)
            .take(12)
            .map(ToOwned::to_owned)
            .collect(),
    }
}

async fn capture_stream<R>(
    mut stream: R,
    overflow: mpsc::UnboundedSender<()>,
) -> io::Result<(Vec<u8>, bool)>
where
    R: AsyncRead + Unpin + Send + 'static,
{
    let mut captured = Vec::new();
    let mut buffer = [0u8; 8192];
    let mut exceeded = false;
    loop {
        let count = stream.read(&mut buffer).await?;
        if count == 0 {
            break;
        }
        let available = MAX_CAPTURED_STREAM_BYTES.saturating_sub(captured.len());
        let keep = count.min(available);
        captured.extend_from_slice(&buffer[..keep]);
        if keep < count && !exceeded {
            exceeded = true;
            let _ = overflow.send(());
        }
    }
    Ok((captured, exceeded))
}

enum ProcessEnd {
    Exited(io::Result<ExitStatus>),
    TimedOut,
    OutputLimit,
}

#[cfg(unix)]
fn kill_process_group(pid: u32) {
    if let Ok(pid) = i32::try_from(pid) {
        // The child is placed in its own process group before it starts.
        unsafe {
            libc::kill(-pid, libc::SIGKILL);
        }
    }
}

async fn stop_child(child: &mut Child) {
    #[cfg(unix)]
    if let Some(pid) = child.id() {
        kill_process_group(pid);
    }
    let _ = child.start_kill();
    let _ = tokio::time::timeout(CHILD_CLEANUP_TIMEOUT, child.wait()).await;
}

async fn join_capture(
    mut task: tokio::task::JoinHandle<io::Result<(Vec<u8>, bool)>>,
) -> (Vec<u8>, bool) {
    match tokio::time::timeout(CHILD_CLEANUP_TIMEOUT, &mut task).await {
        Ok(Ok(Ok(result))) => result,
        _ => {
            task.abort();
            (Vec::new(), true)
        }
    }
}

async fn run_command(command_text: &str, workspace: &std::path::Path) -> Result<ToolOutput> {
    let mut command = Command::new("bash");
    command
        .arg("--noprofile")
        .arg("--norc")
        .arg("-c")
        .arg(command_text)
        .current_dir(workspace)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .env_remove("BASH_ENV")
        .env_remove("ENV")
        .env_remove("SHELLOPTS")
        .env_remove("BASHOPTS");

    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("BASH_FUNC_") {
            command.env_remove(key);
        }
    }

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.as_std_mut().process_group(0);
    }

    let mut child = command
        .spawn()
        .map_err(|error| OttoError::Tool(format!("无法启动 Bash：{error}")))?;
    let process_group_id = child.id();
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| OttoError::Tool("无法捕获 Bash 标准输出".to_owned()))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| OttoError::Tool("无法捕获 Bash 标准错误".to_owned()))?;

    let (overflow_tx, mut overflow_rx) = mpsc::unbounded_channel();
    let stdout_task = tokio::spawn(capture_stream(stdout, overflow_tx.clone()));
    let stderr_task = tokio::spawn(capture_stream(stderr, overflow_tx.clone()));
    drop(overflow_tx);

    let end = tokio::select! {
        result = child.wait() => ProcessEnd::Exited(result),
        _ = tokio::time::sleep(COMMAND_TIMEOUT) => ProcessEnd::TimedOut,
        Some(()) = overflow_rx.recv() => ProcessEnd::OutputLimit,
    };

    let (status, stop_reason) = match end {
        ProcessEnd::Exited(result) => {
            let status = result
                .map_err(|error| OttoError::Tool(format!("等待 Bash 进程结束失败：{error}")))?;
            #[cfg(unix)]
            if let Some(pid) = process_group_id {
                // Bash can leave asynchronous background jobs behind after exiting.
                kill_process_group(pid);
            }
            (Some(status), None)
        }
        ProcessEnd::TimedOut => {
            stop_child(&mut child).await;
            (None, Some("命令超过 120 秒时限，已尝试终止进程"))
        }
        ProcessEnd::OutputLimit => {
            stop_child(&mut child).await;
            (None, Some("命令输出超过每路 64 KiB 限制，已尝试终止进程"))
        }
    };

    let (stdout, stdout_exceeded) = join_capture(stdout_task).await;
    let (stderr, stderr_exceeded) = join_capture(stderr_task).await;
    let stdout = String::from_utf8_lossy(&stdout);
    let stderr = String::from_utf8_lossy(&stderr);
    let mut result = String::new();
    if !stdout.is_empty() {
        result.push_str("标准输出：\n");
        result.push_str(&stdout);
        if !stdout.ends_with('\n') {
            result.push('\n');
        }
    }
    if !stderr.is_empty() {
        result.push_str("标准错误：\n");
        result.push_str(&stderr);
        if !stderr.ends_with('\n') {
            result.push('\n');
        }
    }
    if let Some(reason) = stop_reason {
        result.push_str(reason);
        result.push('\n');
    } else if stdout_exceeded || stderr_exceeded {
        result.push_str("部分输出已截断。\n");
    }
    if let Some(status) = status {
        result.push_str(&format!(
            "退出状态：{}\n",
            status
                .code()
                .map(|code| code.to_string())
                .unwrap_or_else(|| "被信号终止".to_owned())
        ));
    }
    if result.is_empty() {
        result.push_str("命令已执行，没有标准输出或标准错误。\n");
    }
    Ok(ToolOutput::text(result))
}

#[async_trait::async_trait]
impl Tool for BashTool {
    fn name(&self) -> &'static str {
        "bash"
    }

    fn definition(&self) -> Value {
        function_definition(
            self.name(),
            "仅在专用工具无法完成任务时调用。在当前 workspace 根目录中执行一段非交互式 Bash 脚本。生成工具调用时，必须对 command 整段脚本评估风险，并同时填写 risk、reason 和 breakdown。风险判断仅供警告；命令每次执行前仍须用户单次授权，一次授权覆盖脚本中的全部子命令。",
            json!({
                "command": {
                    "type": "string",
                    "description": "完整 Bash 脚本，最多 64 KiB；复合命令作为一段脚本展示和执行"
                },
                "risk": {
                    "type": "string",
                    "enum": ["read_only", "may_modify", "uncertain"],
                    "description": "判断整段脚本的实际效果：read_only=有把握只读；may_modify=可能修改状态或产生外部影响；uncertain=无法可靠判断，拿不准时选此项"
                },
                "reason": {
                    "type": "string",
                    "description": "简短说明风险判断的依据"
                },
                "breakdown": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "按执行顺序说明脚本片段、管道和条件关系；没有复合结构时可返回空数组"
                }
            }),
            &["command", "risk", "reason", "breakdown"],
        )
    }

    async fn execute(&self, arguments: Value, context: &mut ToolContext<'_>) -> Result<ToolOutput> {
        let arguments = arguments_object(arguments)?;
        let command_text = arguments
            .get("command")
            .and_then(Value::as_str)
            .ok_or_else(|| OttoError::Tool("缺少字符串参数：command".to_owned()))?;
        if command_text.trim().is_empty() {
            return Err(OttoError::Tool("Bash 命令不能为空".to_owned()));
        }
        if command_text.len() > MAX_COMMAND_BYTES {
            return Err(OttoError::Tool("Bash 命令超过 64 KiB 限制".to_owned()));
        }
        if command_text.contains('\0') {
            return Err(OttoError::Tool("Bash 命令不能包含 NUL 字符".to_owned()));
        }

        let working_directory = context.workspace.root().display().to_string();
        let assessment = parse_assessment(&arguments);

        if !ui::select_bash_authorization(
            command_text,
            &working_directory,
            assessment.ui_risk(),
            assessment.risk != Risk::ReadOnly,
            &assessment.reason,
            &assessment.breakdown,
            context.mode,
        )? {
            return Err(OttoError::Permission("用户拒绝执行 Bash 命令".to_owned()));
        }

        let output = run_command(command_text, context.workspace.root()).await?;
        Ok(output.with_display_name(format!("bash ({})", assessment.risk_key())))
    }
}
