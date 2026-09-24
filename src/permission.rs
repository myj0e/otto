use std::collections::HashSet;

use crate::error::{OttoError, Result};
use crate::ui::{self, AuthorizationChoice};

const DEFAULT_ALLOWED_READ_TOOLS: [&str; 3] = ["glob", "grep", "read"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Capability {
    Read,
    Write,
}

#[derive(Debug, Default)]
pub struct PermissionManager {
    allowed_tools: HashSet<String>,
}

impl PermissionManager {
    pub fn authorize(
        &mut self,
        tool_name: &str,
        capability: Capability,
        mode: Option<&str>,
        action: &str,
    ) -> Result<()> {
        self.authorize_with_preview(tool_name, capability, mode, action, None)
    }

    pub fn authorize_with_preview(
        &mut self,
        tool_name: &str,
        capability: Capability,
        mode: Option<&str>,
        action: &str,
        preview: Option<&str>,
    ) -> Result<()> {
        let tool_name = tool_name.to_ascii_lowercase();
        if capability == Capability::Read
            && DEFAULT_ALLOWED_READ_TOOLS.contains(&tool_name.as_str())
        {
            return Ok(());
        }
        if self.allowed_tools.contains(&tool_name) {
            return Ok(());
        }

        let choice = ui::select_authorization_with_preview(
            &tool_name,
            capability.label(),
            mode,
            action,
            preview,
        )?;
        match choice {
            AuthorizationChoice::Once => Ok(()),
            AuthorizationChoice::AlwaysForTool => {
                self.allowed_tools.insert(tool_name);
                Ok(())
            }
            AuthorizationChoice::Deny => Err(OttoError::Permission(format!(
                "用户拒绝了本地{}操作：{action}",
                capability.label()
            ))),
        }
    }

    #[cfg(test)]
    fn remember_tool(&mut self, tool_name: &str) {
        self.allowed_tools.insert(tool_name.to_ascii_lowercase());
    }

    #[cfg(test)]
    fn is_allowed(&self, tool_name: &str) -> bool {
        self.allowed_tools.contains(&tool_name.to_ascii_lowercase())
    }
}

impl Capability {
    pub fn label(self) -> &'static str {
        match self {
            Self::Read => "读取",
            Self::Write => "写入",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Capability, PermissionManager};

    #[test]
    fn allows_default_read_tools_without_authorization() {
        let mut permissions = PermissionManager::default();
        for tool_name in ["glob", "grep", "read"] {
            permissions
                .authorize(tool_name, Capability::Read, None, "读取 workspace")
                .expect("default read tool should be allowed");
        }
    }

    #[test]
    fn remembers_only_the_specific_tool() {
        let mut permissions = PermissionManager::default();
        permissions.remember_tool("read");
        assert!(permissions.is_allowed("read"));
        assert!(!permissions.is_allowed("grep"));
    }
}
