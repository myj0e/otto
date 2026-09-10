use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use crate::config;
use crate::error::{OttoError, Result};

const MAX_MODE_BYTES: u64 = 256 * 1024;

fn valid_mode_name(mode: &str) -> bool {
    !mode.is_empty()
        && mode.len() <= 64
        && mode != "."
        && mode != ".."
        && mode
            .bytes()
            .all(|value| value.is_ascii_alphanumeric() || matches!(value, b'_' | b'-' | b'.'))
}

fn read_prompt(path: &Path) -> Result<Option<String>> {
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(OttoError::Config(format!(
                "无法读取提示词文件 {}：{error}",
                path.display()
            )))
        }
    };
    if !metadata.is_file() {
        return Err(OttoError::Config(format!(
            "提示词文件不是普通文件：{}",
            path.display()
        )));
    }
    if metadata.len() > MAX_MODE_BYTES {
        return Err(OttoError::Config(format!(
            "提示词文件超过 256 KiB 限制：{}",
            path.display()
        )));
    }

    let mut content = String::from_utf8(fs::read(path).map_err(|error| {
        OttoError::Config(format!("读取提示词文件 {} 失败：{error}", path.display()))
    })?)
    .map_err(|_| OttoError::Config(format!("提示词文件不是有效的 UTF-8：{}", path.display())))?;

    if content.starts_with('\u{feff}') {
        content.remove(0);
    }
    Ok(Some(content))
}

fn mode_directory() -> Result<(PathBuf, bool)> {
    if let Ok(directory) = env::var("OTTO_MODE_DIR") {
        if !directory.is_empty() {
            return Ok((PathBuf::from(directory), true));
        }
    }
    Ok((config::config_dir()?, false))
}

pub fn load(mode: &str) -> Result<Option<String>> {
    if !valid_mode_name(mode) {
        return Err(OttoError::Usage(format!("模式名称无效：{mode}")));
    }

    let (directory, explicit) = mode_directory()?;
    let path = directory.join(format!("{mode}.md"));
    let prompt = read_prompt(&path)?;
    if prompt.is_some() || explicit || !matches!(mode, "system" | "otto") {
        return Ok(prompt);
    }

    read_prompt(Path::new(".").join(format!("{mode}.md")).as_path())
}

fn active_mode_path() -> Result<PathBuf> {
    Ok(config::config_dir()?.join("active_mode"))
}

pub fn get_active() -> Result<Option<String>> {
    let path = active_mode_path()?;
    let content = match fs::read_to_string(&path) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(OttoError::Config(format!(
                "无法读取当前模式文件 {}：{error}",
                path.display()
            )))
        }
    };
    let value = content.trim();
    if value.is_empty() {
        return Ok(None);
    }
    if !valid_mode_name(value) {
        return Err(OttoError::Config(format!("当前模式名称无效：{value}")));
    }
    Ok(Some(value.to_owned()))
}

pub fn set_active(mode: Option<&str>) -> Result<()> {
    if let Some(mode) = mode {
        if !valid_mode_name(mode) {
            return Err(OttoError::Usage(format!("模式名称无效：{mode}")));
        }
    }
    let path = active_mode_path()?;
    let content = mode.map(|value| format!("{value}\n")).unwrap_or_default();
    config::write_private_atomic(&path, &content)
}

pub fn combine(base: &str, mode: Option<&str>) -> String {
    match mode {
        Some(mode) => format!("{base}\n\n{mode}"),
        None => base.to_owned(),
    }
}
