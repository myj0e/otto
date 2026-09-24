use std::env;
use std::fs::{self, File};
use std::io::{self, IsTerminal, Read, Write};
use std::path::{Path, PathBuf};

use tempfile::NamedTempFile;

use crate::error::{OttoError, Result};
use crate::ui;

pub const DEFAULT_MODEL: &str = "gpt-4o-mini";
pub const DEFAULT_NAME: &str = "Openai";
pub const DEFAULT_BASEURL: &str = "https://api.openai.com";
pub const DEFAULT_MAX_AGENT_ROUNDS: usize = 8;
pub const MAX_CONFIGURED_AGENT_ROUNDS: usize = 255;
const MAX_AGENT_ROUNDS_ENV: &str = "OTTO_MAX_AGENT_ROUNDS";

#[derive(Clone, Default)]
pub struct SearchConfig {
    /// Search is native-first by default. Set this to `third-party` to keep
    /// the legacy provider as the only search route.
    pub mode: Option<String>,
    /// Optional native protocol override. `auto` is inferred from the model
    /// service endpoint when it is not set.
    pub native_protocol: Option<String>,
    pub provider: Option<String>,
    pub endpoint: Option<String>,
    pub api_key: Option<String>,
    pub tavily_api_key: Option<String>,
    pub brave_api_key: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub name: Option<String>,
    pub baseurl: Option<String>,
    pub apikey: Option<String>,
    pub model: String,
    /// Optional context size for displaying recent request utilization.
    pub context_window_tokens: Option<u64>,
}

fn parse_max_agent_rounds(raw_value: &str) -> Result<Option<usize>> {
    let value = raw_value.trim().parse::<usize>().map_err(|_| {
        OttoError::Config(format!(
            "环境变量 {MAX_AGENT_ROUNDS_ENV} 必须是 0 到 {MAX_CONFIGURED_AGENT_ROUNDS} 之间的整数"
        ))
    })?;
    if value > MAX_CONFIGURED_AGENT_ROUNDS {
        return Err(OttoError::Config(format!(
            "环境变量 {MAX_AGENT_ROUNDS_ENV} 必须是 0 到 {MAX_CONFIGURED_AGENT_ROUNDS} 之间的整数"
        )));
    }
    Ok((value != 0).then_some(value))
}

pub fn max_agent_rounds() -> Result<Option<usize>> {
    match env::var(MAX_AGENT_ROUNDS_ENV) {
        Ok(value) => parse_max_agent_rounds(&value),
        Err(env::VarError::NotPresent) => Ok(Some(DEFAULT_MAX_AGENT_ROUNDS)),
        Err(env::VarError::NotUnicode(_)) => Err(OttoError::Config(format!(
            "环境变量 {MAX_AGENT_ROUNDS_ENV} 不是有效的 UTF-8 文本"
        ))),
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            name: None,
            baseurl: None,
            apikey: None,
            model: DEFAULT_MODEL.to_owned(),
            context_window_tokens: None,
        }
    }
}

fn path_parent(path: &Path) -> &Path {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
}

pub fn config_dir() -> Result<PathBuf> {
    if let Ok(override_path) = env::var("OTTO_CONFIG") {
        if !override_path.is_empty() {
            return Ok(Path::new(&override_path)
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
                .unwrap_or_else(|| Path::new("."))
                .to_path_buf());
        }
    }

    if let Ok(config_home) = env::var("XDG_CONFIG_HOME") {
        if !config_home.is_empty() {
            return Ok(PathBuf::from(config_home).join("otto"));
        }
    }

    let home = env::var_os("HOME")
        .or_else(|| env::var_os("USERPROFILE"))
        .ok_or_else(|| OttoError::Config("无法确定当前用户的 home 目录".to_owned()))?;
    Ok(PathBuf::from(home).join(".config").join("otto"))
}

pub fn config_path() -> Result<PathBuf> {
    if let Ok(override_path) = env::var("OTTO_CONFIG") {
        if !override_path.is_empty() {
            return Ok(PathBuf::from(override_path));
        }
    }
    Ok(config_dir()?.join("config"))
}

pub fn search_config_path() -> Result<PathBuf> {
    if let Ok(override_path) = env::var("OTTO_SEARCH_CONFIG") {
        if !override_path.is_empty() {
            return Ok(PathBuf::from(override_path));
        }
    }
    Ok(config_dir()?.join("search.env"))
}

pub fn load(path: &Path) -> Result<(Config, bool)> {
    let mut config = Config::default();
    let mut file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok((config, false)),
        Err(error) => {
            return Err(OttoError::Config(format!(
                "无法读取配置文件 {}：{error}",
                path.display()
            )))
        }
    };

    let mut content = String::new();
    file.read_to_string(&mut content).map_err(|error| {
        OttoError::Config(format!("读取配置文件 {} 失败：{error}", path.display()))
    })?;

    for (line_number, line) in content.lines().enumerate() {
        let entry = line.trim();
        if entry.is_empty() || entry.starts_with('#') {
            continue;
        }

        let (key, value) = entry.split_once('=').ok_or_else(|| {
            OttoError::Config(format!(
                "配置文件 {} 第 {} 行格式错误",
                path.display(),
                line_number + 1
            ))
        })?;
        let key = key.trim();
        let value = value.trim().to_owned();

        match key {
            "name" => config.name = Some(value),
            "baseurl" => config.baseurl = Some(value),
            "apikey" => config.apikey = Some(value),
            "model" => config.model = value,
            "context_window_tokens" => {
                config.context_window_tokens = if value.is_empty() {
                    None
                } else {
                    Some(parse_context_window(&value, path, line_number + 1)?)
                };
            }
            _ => {}
        }
    }

    Ok((config, true))
}

fn search_value(raw_value: &str, path: &Path, line_number: usize) -> Result<String> {
    let value = raw_value.trim();
    if value.len() >= 2 {
        let first = value.as_bytes()[0];
        let last = value.as_bytes()[value.len() - 1];
        if (first == b'"' && last == b'"') || (first == b'\'' && last == b'\'') {
            return Ok(value[1..value.len() - 1].to_owned());
        }
        if first == b'"' || first == b'\'' {
            return Err(OttoError::Config(format!(
                "搜索配置文件 {} 第 {} 行引号不匹配",
                path.display(),
                line_number
            )));
        }
    }
    Ok(value.to_owned())
}

pub fn load_search(path: &Path) -> Result<(SearchConfig, bool)> {
    let mut config = SearchConfig::default();
    let mut file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok((config, false)),
        Err(error) => {
            return Err(OttoError::Config(format!(
                "无法读取搜索配置文件 {}：{error}",
                path.display()
            )))
        }
    };

    let mut content = String::new();
    file.read_to_string(&mut content).map_err(|error| {
        OttoError::Config(format!("读取搜索配置文件 {} 失败：{error}", path.display()))
    })?;

    for (line_index, line) in content.lines().enumerate() {
        let line_number = line_index + 1;
        let entry = line.trim();
        if entry.is_empty() || entry.starts_with('#') {
            continue;
        }
        let entry = entry.strip_prefix("export ").unwrap_or(entry).trim();
        let (key, raw_value) = entry.split_once('=').ok_or_else(|| {
            OttoError::Config(format!(
                "搜索配置文件 {} 第 {} 行格式错误",
                path.display(),
                line_number
            ))
        })?;
        let value = search_value(raw_value, path, line_number)?;
        let value = (!value.is_empty()).then_some(value);

        match key.trim() {
            "OTTO_SEARCH_MODE" | "OTTO_NATIVE_SEARCH" => config.mode = value,
            "OTTO_NATIVE_SEARCH_PROTOCOL" | "OTTO_NATIVE_SEARCH_PROVIDER" => {
                config.native_protocol = value
            }
            "OTTO_SEARCH_PROVIDER" => config.provider = value,
            "OTTO_SEARCH_URL" | "OTTO_SEARCH_ENDPOINT" => config.endpoint = value,
            "OTTO_SEARCH_API_KEY" => config.api_key = value,
            "OTTO_TAVILY_API_KEY" => config.tavily_api_key = value,
            "OTTO_BRAVE_API_KEY" => config.brave_api_key = value,
            // `OTTO_BIN` was used by the removed shell launcher. It is
            // intentionally ignored by the native application config.
            _ => {}
        }
    }

    Ok((config, true))
}

fn valid_baseurl(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    let host = if lower.starts_with("https://") {
        &value[8..]
    } else if lower.starts_with("http://") {
        &value[7..]
    } else {
        return false;
    };

    !host.is_empty()
        && !host.starts_with('/')
        && !value.contains('?')
        && !value.contains('#')
        && !value.chars().any(char::is_whitespace)
}

pub fn validate(config: &Config) -> Result<()> {
    if config.name.as_deref().unwrap_or_default().is_empty() {
        return Err(OttoError::Config("name 不能为空".to_owned()));
    }
    if !valid_baseurl(config.baseurl.as_deref().unwrap_or_default()) {
        return Err(OttoError::Config(
            "baseurl 不是有效的 HTTP/HTTPS 地址".to_owned(),
        ));
    }
    if config.apikey.as_deref().unwrap_or_default().is_empty() {
        return Err(OttoError::Config("apikey 不能为空".to_owned()));
    }
    if config.model.is_empty() {
        return Err(OttoError::Config("model 不能为空".to_owned()));
    }
    if let Some(value) = config.context_window_tokens {
        if !(1..=10_000_000).contains(&value) {
            return Err(OttoError::Config(
                "context_window_tokens 必须在 1 到 10000000 之间".to_owned(),
            ));
        }
    }
    if [
        config.name.as_deref().unwrap_or_default(),
        config.baseurl.as_deref().unwrap_or_default(),
        config.apikey.as_deref().unwrap_or_default(),
        config.model.as_str(),
    ]
    .iter()
    .any(|value| value.contains('\n') || value.contains('\r'))
    {
        return Err(OttoError::Config("配置项不能包含换行符".to_owned()));
    }
    Ok(())
}

fn parse_context_window(raw: &str, path: &Path, line_number: usize) -> Result<u64> {
    let value = raw.parse::<u64>().map_err(|_| {
        OttoError::Config(format!(
            "配置文件 {} 第 {} 行 context_window_tokens 格式错误",
            path.display(),
            line_number
        ))
    })?;
    if !(1..=10_000_000).contains(&value) {
        return Err(OttoError::Config(format!(
            "配置文件 {} 第 {} 行 context_window_tokens 必须在 1 到 10000000 之间",
            path.display(),
            line_number
        )));
    }
    Ok(value)
}

fn set_private_permissions(file: &File) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

pub fn write_private_atomic(path: &Path, content: &str) -> Result<()> {
    let directory = path_parent(path);
    fs::create_dir_all(directory).map_err(|error| {
        OttoError::Config(format!("无法创建配置目录 {}：{error}", directory.display()))
    })?;

    let temporary = NamedTempFile::new_in(directory)
        .map_err(|error| OttoError::Config(format!("无法创建临时配置文件：{error}")))?;
    set_private_permissions(temporary.as_file())?;
    temporary.as_file().write_all(content.as_bytes())?;
    temporary.as_file().sync_all()?;
    temporary.persist(path).map_err(|error| {
        OttoError::Config(format!("保存文件 {} 失败：{}", path.display(), error.error))
    })?;
    Ok(())
}

pub fn save_atomic(path: &Path, config: &Config) -> Result<()> {
    validate(config)?;
    let content = format!(
        "# OTTO (One-time.Talk once) configuration\nname={}\nbaseurl={}\napikey={}\nmodel={}\ncontext_window_tokens={}\n",
        config.name.as_deref().unwrap_or_default(),
        config.baseurl.as_deref().unwrap_or_default(),
        config.apikey.as_deref().unwrap_or_default(),
        config.model,
        config.context_window_tokens.map_or_else(String::new, |value| value.to_string())
    );
    write_private_atomic(path, &content)
}

pub fn save_values(
    path: &Path,
    name: &str,
    baseurl: &str,
    apikey: &str,
    model: Option<&str>,
    context_window_tokens: Option<Option<u64>>,
) -> Result<()> {
    let context_window_tokens = match context_window_tokens {
        Some(value) => value,
        None => load(path)?.0.context_window_tokens,
    };
    let config = Config {
        name: Some(name.to_owned()),
        baseurl: Some(baseurl.to_owned()),
        apikey: Some(apikey.to_owned()),
        model: model.unwrap_or(DEFAULT_MODEL).to_owned(),
        context_window_tokens,
    };
    save_atomic(path, &config)
}

fn read_line(prompt: &str, default: Option<&str>) -> Result<String> {
    ui::write_input_prompt(prompt, default)?;

    let mut line = String::new();
    io::stdin().read_line(&mut line)?;
    if line.is_empty() {
        return Err(OttoError::Config("配置输入已结束".to_owned()));
    }
    let trimmed = line.trim();
    if trimmed.is_empty() {
        Ok(default.unwrap_or_default().to_owned())
    } else {
        Ok(trimmed.to_owned())
    }
}

#[cfg(unix)]
fn read_secret_from_terminal() -> io::Result<String> {
    use std::os::fd::AsRawFd;

    let stdin = io::stdin();
    let fd = stdin.as_raw_fd();
    let mut original = std::mem::MaybeUninit::<libc::termios>::uninit();
    if unsafe { libc::tcgetattr(fd, original.as_mut_ptr()) } != 0 {
        return Err(io::Error::last_os_error());
    }
    let original = unsafe { original.assume_init() };
    let mut hidden = original;
    hidden.c_lflag &= !(libc::ECHO | libc::ECHONL);
    if unsafe { libc::tcsetattr(fd, libc::TCSANOW, &hidden) } != 0 {
        return Err(io::Error::last_os_error());
    }

    // Configure the terminal before printing the prompt so fast pseudo-
    // terminal writers cannot leak the secret between those two operations.
    let read_result = (|| {
        crate::ui::write_input_prompt("API Key", None)?;
        let mut line = String::new();
        stdin.read_line(&mut line)?;
        Ok(line)
    })();
    let restore_result = unsafe { libc::tcsetattr(fd, libc::TCSANOW, &original) };
    if restore_result != 0 {
        return Err(io::Error::last_os_error());
    }
    println!();
    read_result.map(|line| line.trim_end_matches(['\n', '\r']).to_owned())
}

fn read_secret(default: Option<&str>) -> Result<String> {
    let secret = {
        #[cfg(unix)]
        {
            read_secret_from_terminal()
        }
        #[cfg(not(unix))]
        {
            rpassword::prompt_password("API Key: ")
        }
    }
    .map_err(|error| OttoError::Config(format!("读取 API Key 失败：{error}")))?;
    if secret.is_empty() {
        Ok(default.unwrap_or_default().to_owned())
    } else {
        Ok(secret)
    }
}

fn confirm(prompt: &str) -> Result<bool> {
    loop {
        ui::write_input_prompt(prompt, Some("Y/n"))?;
        let mut line = String::new();
        io::stdin().read_line(&mut line)?;
        match line.trim().to_ascii_lowercase().as_str() {
            "" | "y" | "yes" => return Ok(true),
            "n" | "no" => return Ok(false),
            _ => ui::print_error("请输入 y 或 n。"),
        }
    }
}

fn masked_key(apikey: &str) -> String {
    if apikey.is_empty() {
        "<empty>".to_owned()
    } else if apikey.chars().count() <= 8 {
        "********".to_owned()
    } else {
        let characters: Vec<char> = apikey.chars().collect();
        let prefix: String = characters.iter().take(4).collect();
        let suffix: String = characters.iter().rev().take(4).rev().collect();
        format!("{prefix}****{suffix}")
    }
}

pub fn interactive(path: &Path) -> Result<()> {
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        return Err(OttoError::Config(
            "--config 需要在交互式终端中运行".to_owned(),
        ));
    }

    let (mut config, _) = load(path)?;
    ui::print_screen_header(
        "OTTO · 配置",
        "Enter 保留方括号中的当前值；API Key 输入时不回显。",
    );
    let name = read_line("Name", Some(config.name.as_deref().unwrap_or(DEFAULT_NAME)))?;
    let baseurl = read_line(
        "Base URL",
        Some(config.baseurl.as_deref().unwrap_or(DEFAULT_BASEURL)),
    )?;

    let apikey = loop {
        let value = read_secret(config.apikey.as_deref())?;
        if !value.is_empty() {
            break value;
        }
        ui::print_error("API Key 不能为空，请重新输入。");
    };
    let model = read_line("Model", Some(&config.model))?;

    config.name = Some(name);
    config.baseurl = Some(baseurl);
    config.apikey = Some(apikey);
    config.model = model;
    validate(&config)?;

    let masked_key = masked_key(config.apikey.as_deref().unwrap_or_default());
    let summary = [
        ("服务", config.name.as_deref().unwrap_or_default()),
        ("地址", config.baseurl.as_deref().unwrap_or_default()),
        ("API Key", masked_key.as_str()),
        ("模型", config.model.as_str()),
    ];
    ui::print_summary("配置预览", &summary);

    if confirm("保存配置")? {
        save_atomic(path, &config)?;
        ui::print_status(&format!("配置已保存 · {}", path.display()));
    } else {
        ui::print_status("配置未保存");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::{load_search, parse_max_agent_rounds};

    #[test]
    fn validates_configured_agent_rounds() {
        assert_eq!(parse_max_agent_rounds("0").expect("unlimited"), None);
        assert_eq!(parse_max_agent_rounds("1").expect("minimum"), Some(1));
        assert_eq!(parse_max_agent_rounds(" 16 ").expect("trimmed"), Some(16));
        assert_eq!(parse_max_agent_rounds("255").expect("maximum"), Some(255));
        assert!(parse_max_agent_rounds("256").is_err());
        assert!(parse_max_agent_rounds("many").is_err());
    }

    #[test]
    fn loads_native_search_env_without_executing_shell() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let path = directory.path().join("search.env");
        fs::write(
            &path,
            "export OTTO_SEARCH_PROVIDER=tavily\n\
             OTTO_SEARCH_MODE=auto\n\
             OTTO_NATIVE_SEARCH_PROTOCOL=openai-chat\n\
             OTTO_TAVILY_API_KEY=\"test-key\"\n\
             OTTO_BIN=/tmp/legacy-launcher-target\n",
        )
        .expect("search config");

        let (config, found) = load_search(&path).expect("search config loads");
        assert!(found);
        assert_eq!(config.mode.as_deref(), Some("auto"));
        assert_eq!(config.native_protocol.as_deref(), Some("openai-chat"));
        assert_eq!(config.provider.as_deref(), Some("tavily"));
        assert_eq!(config.tavily_api_key.as_deref(), Some("test-key"));
        assert!(config.api_key.is_none());
    }
}
