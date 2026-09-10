use std::io::{self, IsTerminal, Write};

use crate::error::{OttoError, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Capability {
    Read,
    Write,
}

#[derive(Debug, Default)]
pub struct PermissionManager {
    read_session: bool,
    write_session: bool,
}

impl PermissionManager {
    pub fn authorize(
        &mut self,
        capability: Capability,
        mode: Option<&str>,
        action: &str,
    ) -> Result<()> {
        let session_allowed = match capability {
            Capability::Read => self.read_session,
            Capability::Write => self.write_session,
        };
        if session_allowed {
            return Ok(());
        }

        if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
            return Err(OttoError::Permission(format!(
                "当前终端不可交互，已拒绝本地{}操作：{action}",
                capability.label()
            )));
        }

        print!("{}", authorization_message(mode, capability, action));
        io::stdout().flush()?;

        loop {
            let mut answer = String::new();
            if io::stdin().read_line(&mut answer)? == 0 {
                return Err(OttoError::Permission(
                    "授权输入已结束，已拒绝本地操作".to_owned(),
                ));
            }
            match answer.trim().to_ascii_lowercase().as_str() {
                "1" | "y" | "yes" => return Ok(()),
                "2" | "a" | "always" => {
                    match capability {
                        Capability::Read => self.read_session = true,
                        Capability::Write => self.write_session = true,
                    }
                    return Ok(());
                }
                "3" | "n" | "no" => {
                    return Err(OttoError::Permission(format!(
                        "用户拒绝了本地{}操作：{action}",
                        capability.label()
                    )))
                }
                _ => {
                    print!("请输入 1、2 或 3：");
                    io::stdout().flush()?;
                }
            }
        }
    }
}

impl Capability {
    fn label(self) -> &'static str {
        match self {
            Self::Read => "读取",
            Self::Write => "写入",
        }
    }
}

fn authorization_message(mode: Option<&str>, capability: Capability, action: &str) -> String {
    let safe_action: String = action
        .chars()
        .filter(|character| !character.is_control())
        .take(200)
        .collect();
    let mode = mode.unwrap_or_default().to_ascii_lowercase();
    let label = capability.label();
    let (opening, closing) = match mode.as_str() {
        "otto" => (
            "这波要碰你本地文件了，",
            "你确认就输入 1；想让本轮同类操作都放行输入 2；不让碰输入 3，别稀里糊涂点确认。",
        ),
        "jarvis" => (
            "检测到需要访问本地文件，",
            "请确认授权范围：1 仅此次，2 本轮同类操作总是允许，3 拒绝。",
        ),
        _ => (
            "otto 需要访问本地文件，",
            "请确认授权范围：1 仅此次，2 本轮同类操作总是允许，3 拒绝。",
        ),
    };
    format!("\n{opening}即将{label}：{safe_action}\n{closing}\n授权选择 [1/2/3]: ")
}
