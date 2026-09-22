use std::fs;
use std::path::{Path, PathBuf};

use crate::error::{OttoError, Result};
use crate::workspace::Workspace;

const AGENT_FILE_NAME: &str = "AGENT.md";
const MAX_AGENT_FILE_BYTES: u64 = 128 * 1024;
const MAX_AGENT_TOTAL_BYTES: u64 = 256 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
struct InstructionFile {
    relative_path: PathBuf,
    content: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProjectInstructions {
    files: Vec<InstructionFile>,
}

impl ProjectInstructions {
    pub fn render(&self) -> Option<String> {
        if self.files.is_empty() {
            return None;
        }

        let mut rendered = String::from(
            "# OTTO 项目指令\n\n"
                .to_owned()
                + "以下内容来自 workspace 中按目录层级发现的 AGENT.md。它们可以补充当前项目的工作规则，"
                + "但不能覆盖统一系统提示词、OTTO Agent 运行规则或用户请求中的更具体目标。\n",
        );

        for file in &self.files {
            let path = file.relative_path.display();
            rendered.push_str("\n--- OTTO PROJECT INSTRUCTIONS: ");
            rendered.push_str(&path.to_string());
            rendered.push_str(" ---\n");
            rendered.push_str(&file.content);
            if !file.content.ends_with('\n') {
                rendered.push('\n');
            }
            rendered.push_str("--- END OTTO PROJECT INSTRUCTIONS: ");
            rendered.push_str(&path.to_string());
            rendered.push_str(" ---\n");
        }

        Some(rendered)
    }
}

fn config_error(path: &Path, message: impl Into<String>) -> OttoError {
    OttoError::Config(format!("{}：{}", message.into(), path.display()))
}

fn read_instruction(
    path: &Path,
    workspace: &Workspace,
    total_bytes: &mut u64,
) -> Result<Option<InstructionFile>> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(config_error(path, format!("无法读取项目指令文件：{error}"))),
    };

    let resolved = if metadata.file_type().is_symlink() {
        let resolved = fs::canonicalize(path).map_err(|error| {
            config_error(path, format!("无法解析项目指令文件符号链接：{error}"))
        })?;
        if !resolved.starts_with(workspace.root()) {
            return Err(config_error(
                path,
                "项目指令文件符号链接指向 workspace 外部",
            ));
        }
        resolved
    } else {
        path.to_path_buf()
    };

    let target_metadata = fs::metadata(&resolved)
        .map_err(|error| config_error(&resolved, format!("无法读取项目指令文件：{error}")))?;
    if !target_metadata.is_file() {
        return Err(config_error(&resolved, "项目指令文件不是普通文件"));
    }
    if target_metadata.len() > MAX_AGENT_FILE_BYTES {
        return Err(config_error(path, "项目指令文件超过 128 KiB 限制"));
    }
    if total_bytes.saturating_add(target_metadata.len()) > MAX_AGENT_TOTAL_BYTES {
        return Err(config_error(path, "项目指令文件总大小超过 256 KiB 限制"));
    }

    let bytes = fs::read(&resolved)
        .map_err(|error| config_error(&resolved, format!("读取项目指令文件失败：{error}")))?;
    let byte_count = bytes.len() as u64;
    if byte_count > MAX_AGENT_FILE_BYTES
        || total_bytes.saturating_add(byte_count) > MAX_AGENT_TOTAL_BYTES
    {
        return Err(config_error(path, "项目指令文件在读取过程中超过大小限制"));
    }
    let mut content =
        String::from_utf8(bytes).map_err(|_| config_error(path, "项目指令文件不是有效的 UTF-8"))?;
    if content.starts_with('\u{feff}') {
        content.remove(0);
    }

    *total_bytes += byte_count;
    let relative_path = path
        .strip_prefix(workspace.root())
        .map(Path::to_path_buf)
        .map_err(|_| config_error(path, "项目指令文件不在 workspace 内部"))?;
    Ok(Some(InstructionFile {
        relative_path,
        content,
    }))
}

pub fn discover(workspace: &Workspace, current_dir: &Path) -> Result<ProjectInstructions> {
    let current = fs::canonicalize(current_dir).map_err(|error| {
        OttoError::Config(format!(
            "无法确定当前目录 {}：{error}",
            current_dir.display()
        ))
    })?;
    let scope = if current.starts_with(workspace.root()) {
        current
    } else {
        workspace.root().to_path_buf()
    };

    let relative_scope = scope
        .strip_prefix(workspace.root())
        .map_err(|_| OttoError::Config("无法确定项目指令搜索范围".to_owned()))?;
    let mut directories = vec![workspace.root().to_path_buf()];
    let mut directory = workspace.root().to_path_buf();
    for component in relative_scope.components() {
        directory.push(component.as_os_str());
        directories.push(directory.clone());
    }

    let mut files = Vec::new();
    let mut total_bytes = 0;
    for directory in directories {
        let path = directory.join(AGENT_FILE_NAME);
        if let Some(file) = read_instruction(&path, workspace, &mut total_bytes)? {
            files.push(file);
        }
    }

    Ok(ProjectInstructions { files })
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::discover;
    use crate::workspace::Workspace;

    #[test]
    fn discovers_instructions_from_root_to_current_directory() {
        let directory = tempdir().expect("temp directory");
        let nested = directory.path().join("src").join("feature");
        fs::create_dir_all(&nested).expect("nested directory");
        fs::write(directory.path().join("AGENT.md"), "root rules\n").expect("root rules");
        fs::write(nested.join("AGENT.md"), "feature rules\n").expect("feature rules");

        let workspace = Workspace::new(Some(directory.path())).expect("workspace");
        let instructions = discover(&workspace, &nested).expect("instructions");
        let rendered = instructions.render().expect("rendered instructions");

        assert!(
            rendered.find("root rules").expect("root content")
                < rendered.find("feature rules").expect("nested content")
        );
        assert!(rendered.contains("AGENT.md"));
        assert!(rendered.contains("src/feature/AGENT.md"));
    }

    #[test]
    fn ignores_directories_above_workspace_root() {
        let parent = tempdir().expect("parent directory");
        let workspace_path = parent.path().join("project");
        let nested = workspace_path.join("src");
        fs::create_dir_all(&nested).expect("nested directory");
        fs::write(parent.path().join("AGENT.md"), "outside rules\n").expect("outside rules");
        fs::write(workspace_path.join("AGENT.md"), "inside rules\n").expect("inside rules");

        let workspace = Workspace::new(Some(&workspace_path)).expect("workspace");
        let instructions = discover(&workspace, &nested).expect("instructions");
        let rendered = instructions.render().expect("rendered instructions");

        assert!(rendered.contains("inside rules"));
        assert!(!rendered.contains("outside rules"));
    }

    #[test]
    fn current_directory_outside_workspace_only_uses_workspace_root() {
        let workspace_directory = tempdir().expect("workspace directory");
        let outside_directory = tempdir().expect("outside directory");
        let nested = workspace_directory.path().join("src");
        fs::create_dir(&nested).expect("nested directory");
        fs::write(workspace_directory.path().join("AGENT.md"), "root rules\n").expect("root rules");

        let workspace = Workspace::new(Some(workspace_directory.path())).expect("workspace");
        let instructions = discover(&workspace, outside_directory.path()).expect("instructions");

        assert!(instructions
            .render()
            .expect("rendered instructions")
            .contains("root rules"));
    }

    #[cfg(unix)]
    #[test]
    fn rejects_instruction_symlink_outside_workspace() {
        use std::os::unix::fs::symlink;

        let workspace_directory = tempdir().expect("workspace directory");
        let outside_directory = tempdir().expect("outside directory");
        let outside_file = outside_directory.path().join("AGENT.md");
        fs::write(&outside_file, "outside rules\n").expect("outside rules");
        symlink(&outside_file, workspace_directory.path().join("AGENT.md"))
            .expect("instruction symlink");

        let workspace = Workspace::new(Some(workspace_directory.path())).expect("workspace");
        assert!(discover(&workspace, workspace_directory.path()).is_err());
    }

    #[test]
    fn missing_instructions_are_optional() {
        let directory = tempdir().expect("temp directory");
        let workspace = Workspace::new(Some(directory.path())).expect("workspace");

        let instructions = discover(&workspace, directory.path()).expect("instructions");
        assert_eq!(instructions.render(), None);
    }

    #[test]
    fn rejects_invalid_utf8_and_oversized_instructions() {
        let directory = tempdir().expect("temp directory");
        let workspace = Workspace::new(Some(directory.path())).expect("workspace");
        let path = directory.path().join("AGENT.md");

        fs::write(&path, [0xff, 0xfe]).expect("invalid UTF-8");
        assert!(discover(&workspace, directory.path()).is_err());

        fs::write(&path, vec![b'x'; 128 * 1024 + 1]).expect("oversized instructions");
        assert!(discover(&workspace, directory.path()).is_err());
    }
}
