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
        self.ensure_not_session_storage(&resolved)?;
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
        self.ensure_not_session_storage(&requested)?;
        if requested.exists() {
            let metadata = fs::symlink_metadata(&requested)?;
            if metadata.file_type().is_symlink() {
                return Err(OttoError::Tool(format!("拒绝写入符号链接：{input}")));
            }
            let resolved = fs::canonicalize(&requested)?;
            self.ensure_inside(&resolved)?;
            self.ensure_not_session_storage(&resolved)?;
            return Ok(resolved);
        }

        let mut parent = requested
            .parent()
            .ok_or_else(|| OttoError::Tool(format!("无法确定文件父目录：{input}")))?;
        loop {
            if parent.exists() {
                let resolved_parent = fs::canonicalize(parent)?;
                self.ensure_inside(&resolved_parent)?;
                self.ensure_not_session_storage(&resolved_parent)?;
                return Ok(requested);
            }
            parent = parent
                .parent()
                .ok_or_else(|| OttoError::Tool(format!("无法确定文件父目录：{input}")))?;
        }
    }

    fn otto_root(&self, create: bool) -> Result<Option<(PathBuf, bool)>> {
        let requested = self.root.join(".otto");
        let mut created = false;
        match fs::symlink_metadata(&requested) {
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound && !create => {
                return Ok(None)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                match fs::create_dir(&requested) {
                    Ok(()) => {
                        created = true;
                        set_private_directory_permissions(&requested)?;
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                    Err(error) => return Err(error.into()),
                }
            }
            Err(error) => return Err(error.into()),
        }

        let metadata = fs::symlink_metadata(&requested)?;
        if metadata.file_type().is_symlink() {
            return Err(OttoError::Tool(
                "拒绝使用符号链接作为 workspace 的 .otto 目录".to_owned(),
            ));
        }
        if !metadata.is_dir() {
            return Err(OttoError::Tool(
                "workspace 中的 .otto 必须是目录".to_owned(),
            ));
        }

        let resolved = fs::canonicalize(&requested)?;
        self.ensure_inside(&resolved)?;
        Ok(Some((resolved, created)))
    }

    /// Return the existing .otto directory, if present. Symlinks and non-directory
    /// entries are rejected so this remains a well-defined storage boundary.
    pub fn existing_otto_directory(&self) -> Result<Option<PathBuf>> {
        Ok(self.otto_root(false)?.map(|(path, _)| path))
    }

    /// Create the workspace-local .otto directory with private Unix permissions.
    pub fn ensure_otto_directory(&self) -> Result<(PathBuf, bool)> {
        self.otto_root(true)?
            .ok_or_else(|| OttoError::Tool("无法创建 workspace 的 .otto 目录".to_owned()))
    }

    /// If a workspace-relative path starts at .otto, return its path relative to
    /// that directory. Other workspace-relative paths return None.
    pub fn otto_relative_path(&self, input: &str) -> Result<Option<PathBuf>> {
        let relative = Self::validate_relative(input)?;
        let mut components = relative.components();
        let first = loop {
            match components.next() {
                Some(Component::CurDir) => continue,
                Some(component) => break Some(component),
                None => break None,
            }
        };
        if !matches!(first, Some(Component::Normal(name)) if name == ".otto") {
            return Ok(None);
        }

        let mut result = PathBuf::new();
        for component in components {
            match component {
                Component::Normal(name) => result.push(name),
                Component::CurDir => {}
                _ => return Err(OttoError::Tool("只允许使用 .otto 内的相对路径".to_owned())),
            }
        }
        Ok(Some(result))
    }

    fn validate_otto_relative(input: &str) -> Result<PathBuf> {
        let relative = Self::validate_relative(input)?;
        let mut normalized = PathBuf::new();
        for component in relative.components() {
            match component {
                Component::Normal(name) => normalized.push(name),
                Component::CurDir => {}
                _ => {
                    return Err(OttoError::Tool(format!(
                        "只允许使用 .otto 内的相对路径：{input}"
                    )))
                }
            }
        }
        if normalized.as_os_str().is_empty() {
            return Err(OttoError::Tool("文件路径不能为空".to_owned()));
        }
        Ok(normalized)
    }

    fn resolve_otto_relative(
        &self,
        root: &Path,
        relative: &Path,
        create_parents: bool,
        allow_missing_leaf: bool,
    ) -> Result<PathBuf> {
        if relative
            .components()
            .next()
            .is_some_and(|component| component.as_os_str() == "sessions")
        {
            return Err(OttoError::Tool(
                ".otto/sessions 是 OTTO 会话内部存储区，不能通过通用存储工具访问".to_owned(),
            ));
        }
        let components = relative.components().collect::<Vec<_>>();
        let mut current = root.to_path_buf();
        for (index, component) in components.iter().enumerate() {
            let Component::Normal(name) = component else {
                return Err(OttoError::Tool("只允许使用 .otto 内的相对路径".to_owned()));
            };
            current.push(name);
            let is_leaf = index + 1 == components.len();
            let metadata = match fs::symlink_metadata(&current) {
                Ok(metadata) => metadata,
                Err(error)
                    if error.kind() == std::io::ErrorKind::NotFound
                        && create_parents
                        && !is_leaf =>
                {
                    match fs::create_dir(&current) {
                        Ok(()) => set_private_directory_permissions(&current)?,
                        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                        Err(error) => return Err(error.into()),
                    }
                    fs::symlink_metadata(&current)?
                }
                Err(error)
                    if error.kind() == std::io::ErrorKind::NotFound
                        && is_leaf
                        && allow_missing_leaf =>
                {
                    break;
                }
                Err(error) => {
                    return Err(OttoError::Tool(format!(
                        "无法访问 .otto 内的路径 {}：{error}",
                        relative.display()
                    )))
                }
            };
            if metadata.file_type().is_symlink() {
                return Err(OttoError::Tool(format!(
                    "拒绝访问 .otto 内的符号链接：{}",
                    relative.display()
                )));
            }
            if !is_leaf && !metadata.is_dir() {
                return Err(OttoError::Tool(format!(
                    ".otto 路径中的父项不是目录：{}",
                    relative.display()
                )));
            }
        }

        let target = root.join(relative);
        let parent = target
            .parent()
            .ok_or_else(|| OttoError::Tool("无法确定 .otto 文件的父目录".to_owned()))?;
        let resolved_parent = fs::canonicalize(parent)
            .map_err(|error| OttoError::Tool(format!("无法解析 .otto 文件的父目录：{error}")))?;
        if !resolved_parent.starts_with(root) {
            return Err(OttoError::Tool(
                "路径超出 workspace 的 .otto 目录".to_owned(),
            ));
        }
        if fs::symlink_metadata(&target).is_ok() {
            let resolved = fs::canonicalize(&target).map_err(|error| {
                OttoError::Tool(format!("无法解析 .otto 内的目标文件：{error}"))
            })?;
            if !resolved.starts_with(root) {
                return Err(OttoError::Tool(
                    "路径超出 workspace 的 .otto 目录".to_owned(),
                ));
            }
            return Ok(resolved);
        }
        if allow_missing_leaf {
            Ok(target)
        } else {
            Err(OttoError::Tool(format!(
                "无法找到 .otto 内的文件：{}",
                relative.display()
            )))
        }
    }

    /// Resolve a path relative to .otto, rejecting symlinks in every path segment.
    pub fn resolve_otto_existing(&self, input: &str) -> Result<PathBuf> {
        let relative = Self::validate_otto_relative(input)?;
        let root = self
            .existing_otto_directory()?
            .ok_or_else(|| OttoError::Tool("workspace 的 .otto 目录尚未创建".to_owned()))?;
        self.resolve_otto_relative(&root, &relative, false, false)
    }

    /// Resolve a writable path relative to .otto, creating .otto and parent
    /// directories as needed while rejecting symlinks in existing path segments.
    pub fn resolve_otto_for_create(&self, input: &str) -> Result<PathBuf> {
        let relative = Self::validate_otto_relative(input)?;
        let (root, _) = self.ensure_otto_directory()?;
        self.resolve_otto_relative(&root, &relative, true, true)
    }

    /// Resolve a workspace-relative path under .otto with its stricter path checks.
    pub fn resolve_otto_workspace_existing(&self, input: &str) -> Result<Option<PathBuf>> {
        let Some(relative) = self.otto_relative_path(input)? else {
            return Ok(None);
        };
        if relative.as_os_str().is_empty() {
            let root = self
                .existing_otto_directory()?
                .ok_or_else(|| OttoError::Tool("workspace 的 .otto 目录尚未创建".to_owned()))?;
            return Ok(Some(root));
        }
        let relative_text = relative
            .to_str()
            .ok_or_else(|| OttoError::Tool(".otto 路径不是有效文本".to_owned()))?;
        self.resolve_otto_existing(relative_text).map(Some)
    }

    /// Resolve a writable workspace-relative path under .otto, creating its
    /// directory tree as needed. Paths outside .otto return None.
    pub fn resolve_otto_workspace_for_create(&self, input: &str) -> Result<Option<PathBuf>> {
        let Some(relative) = self.otto_relative_path(input)? else {
            return Ok(None);
        };
        if relative.as_os_str().is_empty() {
            return Err(OttoError::Tool(
                "目标必须是 .otto 目录中的文件路径".to_owned(),
            ));
        }
        let relative_text = relative
            .to_str()
            .ok_or_else(|| OttoError::Tool(".otto 路径不是有效文本".to_owned()))?;
        self.resolve_otto_for_create(relative_text).map(Some)
    }

    pub fn display(&self, path: &Path) -> String {
        self.relative(path)
            .map(|relative| relative.display().to_string())
            .unwrap_or_else(|_| path.display().to_string())
    }

    /// Identify the internal session cache so recursive workspace tools can
    /// prune it while searching from the workspace root.
    pub fn is_session_storage_path(&self, path: &Path) -> bool {
        path.starts_with(self.root.join(".otto").join("sessions"))
    }

    fn ensure_not_session_storage(&self, path: &Path) -> Result<()> {
        if self.is_session_storage_path(path) {
            Err(OttoError::Tool(
                ".otto/sessions 是 OTTO 会话内部存储区，不能通过通用文件工具访问".to_owned(),
            ))
        } else {
            Ok(())
        }
    }
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
