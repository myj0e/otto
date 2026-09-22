use std::env;
use std::fs;
use std::path::{Component, Path, PathBuf};

use crate::error::{OttoError, Result};

#[derive(Debug, Clone)]
pub struct Workspace {
    root: PathBuf,
}

impl Workspace {
    pub fn new(root: Option<&Path>) -> Result<Self> {
        let requested = match root {
            Some(path) if path.is_absolute() => path.to_path_buf(),
            Some(path) => env::current_dir()?.join(path),
            None => env::current_dir()?,
        };
        let root = fs::canonicalize(&requested).map_err(|error| {
            OttoError::Config(format!(
                "无法使用 workspace 根目录 {}：{error}",
                requested.display()
            ))
        })?;
        if !root.is_dir() {
            return Err(OttoError::Config(format!(
                "workspace 根目录不是目录：{}",
                root.display()
            )));
        }
        Ok(Self { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn relative(&self, path: &Path) -> Result<PathBuf> {
        path.strip_prefix(&self.root)
            .map(Path::to_path_buf)
            .map_err(|_| OttoError::Tool(format!("路径超出 workspace 范围：{}", path.display())))
    }

    fn validate_relative(input: &str) -> Result<&Path> {
        if input.trim().is_empty() {
            return Err(OttoError::Tool("路径不能为空".to_owned()));
        }
        let path = Path::new(input);
        if path.is_absolute()
            || path.components().any(|component| {
                matches!(
                    component,
                    Component::Prefix(_) | Component::RootDir | Component::ParentDir
                )
            })
        {
            return Err(OttoError::Tool(format!(
                "只允许使用 workspace 内的相对路径：{input}"
            )));
        }
        Ok(path)
    }

    fn ensure_inside(&self, path: &Path) -> Result<()> {
        if path.starts_with(&self.root) {
            Ok(())
        } else {
            Err(OttoError::Tool(format!(
                "路径超出 workspace 范围：{}",
                path.display()
            )))
        }
    }

    pub fn resolve_existing(&self, input: &str) -> Result<PathBuf> {
        let relative = Self::validate_relative(input)?;
        let requested = self.root.join(relative);
        let resolved = fs::canonicalize(&requested)
            .map_err(|error| OttoError::Tool(format!("无法找到文件 {}：{error}", input)))?;
        self.ensure_inside(&resolved)?;
        Ok(resolved)
    }

    pub fn resolve_directory(&self, input: Option<&str>) -> Result<PathBuf> {
        match input.filter(|value| !value.trim().is_empty()) {
            Some(value) => {
                let path = self.resolve_existing(value)?;
                if !path.is_dir() {
                    return Err(OttoError::Tool(format!("搜索范围不是目录：{value}")));
                }
                Ok(path)
            }
            None => Ok(self.root.clone()),
        }
    }

    pub fn resolve_for_create(&self, input: &str) -> Result<PathBuf> {
        let relative = Self::validate_relative(input)?;
        let requested = self.root.join(relative);
        if requested.exists() {
            let metadata = fs::symlink_metadata(&requested)?;
            if metadata.file_type().is_symlink() {
                return Err(OttoError::Tool(format!("拒绝写入符号链接：{input}")));
            }
            let resolved = fs::canonicalize(&requested)?;
            self.ensure_inside(&resolved)?;
            return Ok(resolved);
        }

        let mut parent = requested
            .parent()
            .ok_or_else(|| OttoError::Tool(format!("无法确定文件父目录：{input}")))?;
        loop {
            if parent.exists() {
                let resolved_parent = fs::canonicalize(parent)?;
                self.ensure_inside(&resolved_parent)?;
                return Ok(requested);
            }
            parent = parent
                .parent()
                .ok_or_else(|| OttoError::Tool(format!("无法确定文件父目录：{input}")))?;
        }
    }

    pub fn display(&self, path: &Path) -> String {
        self.relative(path)
            .map(|relative| relative.display().to_string())
            .unwrap_or_else(|_| path.display().to_string())
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::Workspace;

    #[test]
    fn resolves_files_inside_workspace() {
        let directory = tempdir().expect("temp directory");
        fs::write(directory.path().join("note.txt"), "hello").expect("file");
        let workspace = Workspace::new(Some(directory.path())).expect("workspace");
        let path = workspace.resolve_existing("note.txt").expect("file path");
        assert_eq!(workspace.display(&path), "note.txt");
    }

    #[test]
    fn rejects_parent_traversal() {
        let directory = tempdir().expect("temp directory");
        let workspace = Workspace::new(Some(directory.path())).expect("workspace");
        assert!(workspace.resolve_existing("../outside.txt").is_err());
    }

    #[test]
    fn permits_new_nested_file_inside_workspace() {
        let directory = tempdir().expect("temp directory");
        let workspace = Workspace::new(Some(directory.path())).expect("workspace");
        let path = workspace
            .resolve_for_create("generated/note.txt")
            .expect("new path");
        assert!(path.starts_with(fs::canonicalize(directory.path()).expect("canonical root")));
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlink_escape() {
        use std::os::unix::fs::symlink;

        let directory = tempdir().expect("temp directory");
        let outside = tempdir().expect("outside directory");
        fs::write(outside.path().join("secret.txt"), "secret").expect("secret");
        symlink(
            outside.path().join("secret.txt"),
            directory.path().join("link.txt"),
        )
        .expect("symlink");
        let workspace = Workspace::new(Some(directory.path())).expect("workspace");
        assert!(workspace.resolve_existing("link.txt").is_err());
    }
}
