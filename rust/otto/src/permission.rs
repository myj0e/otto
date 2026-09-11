use std::collections::HashSet;

use crate::error::{OttoError, Result};
use crate::ui::{self, AuthorizationChoice};

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
        let tool_name = tool_name.to_ascii_lowercase();
        if self.allowed_tools.contains(&tool_name) {
            return Ok(());
        }

        let choice = ui::select_authorization(&tool_name, capability.label(), mode, action)?;
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
    use super::PermissionManager;

    #[test]
    fn remembers_only_the_specific_tool() {
        let mut permissions = PermissionManager::default();
        permissions.remember_tool("read");
        assert!(permissions.is_allowed("read"));
        assert!(!permissions.is_allowed("grep"));
    }
}
