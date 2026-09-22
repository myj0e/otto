use std::path::PathBuf;

use crate::error::{OttoError, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    Ask,
    Config,
    Mode,
    Help,
    Version,
}

#[derive(Debug)]
pub struct CliOptions {
    pub command: Command,
    pub prompt: Vec<String>,
    pub raw_mode: bool,
    pub mode: Option<String>,
    pub name: Option<String>,
    pub baseurl: Option<String>,
    pub apikey: Option<String>,
    pub model: Option<String>,
    pub has_config_options: bool,
    pub root: Option<PathBuf>,
    pub no_agent: bool,
    pub no_stdin: bool,
}

impl Default for CliOptions {
    fn default() -> Self {
        Self {
            command: Command::Ask,
            prompt: Vec::new(),
            raw_mode: false,
            mode: None,
            name: None,
            baseurl: None,
            apikey: None,
            model: None,
            has_config_options: false,
            root: None,
            no_agent: false,
            no_stdin: false,
        }
    }
}

fn usage_error(message: impl Into<String>) -> OttoError {
    OttoError::Usage(message.into())
}

fn take_option_value(
    arguments: &[String],
    index: &mut usize,
    option: &str,
) -> Result<Option<String>> {
    let argument = &arguments[*index];
    let value = if argument == option {
        *index += 1;
        if *index >= arguments.len() {
            return Err(usage_error(format!("选项 {option} 缺少值")));
        }
        arguments[*index].clone()
    } else if let Some(value) = argument.strip_prefix(&format!("{option}=")) {
        value.to_owned()
    } else {
        return Ok(None);
    };

    if value.is_empty() || value.starts_with("--") {
        return Err(usage_error(format!("选项 {option} 缺少有效值")));
    }

    Ok(Some(value))
}

fn parse_config(arguments: &[String]) -> Result<CliOptions> {
    let mut options = CliOptions {
        command: Command::Config,
        ..CliOptions::default()
    };
    let mut index = 1;

    while index < arguments.len() {
        if arguments[index] == "--help" || arguments[index] == "-h" {
            options.command = Command::Help;
            return Ok(options);
        }

        if let Some(value) = take_option_value(arguments, &mut index, "--name")? {
            options.name = Some(value);
        } else if let Some(value) = take_option_value(arguments, &mut index, "--baseurl")? {
            options.baseurl = Some(value);
        } else if let Some(value) = take_option_value(arguments, &mut index, "--base-url")? {
            options.baseurl = Some(value);
        } else if let Some(value) = take_option_value(arguments, &mut index, "--apikey")? {
            options.apikey = Some(value);
        } else if let Some(value) = take_option_value(arguments, &mut index, "--api-key")? {
            options.apikey = Some(value);
        } else if let Some(value) = take_option_value(arguments, &mut index, "--model")? {
            options.model = Some(value);
        } else {
            return Err(usage_error(format!("未知配置选项：{}", arguments[index])));
        }
        options.has_config_options = true;
        index += 1;
    }

    Ok(options)
}

fn parse_mode(arguments: &[String]) -> Result<CliOptions> {
    let mut options = CliOptions::default();
    let index;

    if let Some(value) = arguments[0].strip_prefix("--mode=") {
        options.raw_mode = value.is_empty();
        if !value.is_empty() {
            options.mode = Some(value.to_owned());
        }
        index = 1;
    } else if arguments.len() == 1 {
        options.raw_mode = true;
        options.command = Command::Mode;
        return Ok(options);
    } else if matches!(
        arguments[1].as_str(),
        "--" | "--no-agent" | "--no-stdin" | "--root"
    ) || arguments[1].starts_with("--root=")
    {
        options.raw_mode = true;
        index = 1;
    } else {
        options.mode = Some(arguments[1].clone());
        index = 2;
    }

    if index >= arguments.len() {
        options.command = Command::Mode;
        return Ok(options);
    }

    parse_question_tail(arguments, index, options)
}

fn parse_question_tail(
    arguments: &[String],
    mut index: usize,
    mut options: CliOptions,
) -> Result<CliOptions> {
    while index < arguments.len() {
        if arguments[index] == "--" {
            options
                .prompt
                .extend(arguments[index + 1..].iter().cloned());
            return Ok(options);
        }
        if arguments[index] == "--no-agent" {
            options.no_agent = true;
            index += 1;
            continue;
        }
        if arguments[index] == "--no-stdin" {
            options.no_stdin = true;
            index += 1;
            continue;
        }
        if let Some(value) = take_option_value(arguments, &mut index, "--root")? {
            options.root = Some(PathBuf::from(value));
            index += 1;
            continue;
        }
        if arguments[index].starts_with('-') {
            return Err(usage_error(format!("未知选项：{}", arguments[index])));
        }

        // Once the first positional argument is the question, every remaining
        // argument belongs to that question. In particular, strings such as
        // `-m` or `--no-agent` may be part of the question and must not be
        // parsed as Otto options.
        options.prompt.extend(arguments[index..].iter().cloned());
        return Ok(options);
    }
    Ok(options)
}

pub fn parse(arguments: impl IntoIterator<Item = String>) -> Result<CliOptions> {
    let arguments: Vec<String> = arguments.into_iter().collect();

    if arguments.is_empty() {
        return Ok(CliOptions::default());
    }

    match arguments[0].as_str() {
        "--help" | "-h" => Ok(CliOptions {
            command: Command::Help,
            ..CliOptions::default()
        }),
        "--version" | "-V" => Ok(CliOptions {
            command: Command::Version,
            ..CliOptions::default()
        }),
        "--config" => parse_config(&arguments),
        argument if argument == "--mode" || argument.starts_with("--mode=") => {
            parse_mode(&arguments)
        }
        "--" => Ok(CliOptions {
            prompt: arguments[1..].to_vec(),
            ..CliOptions::default()
        }),
        "--no-agent" | "--no-stdin" | "--root" => {
            parse_question_tail(&arguments, 0, CliOptions::default())
        }
        argument if argument.starts_with("--root=") => {
            parse_question_tail(&arguments, 0, CliOptions::default())
        }
        argument if argument.starts_with('-') => Err(usage_error(format!(
            "未知选项：{argument}\notto: 如果问题以 - 开头，请使用 otto -- <问题>"
        ))),
        _ => parse_question_tail(&arguments, 0, CliOptions::default()),
    }
}

pub fn join_prompt(arguments: &[String]) -> Result<String> {
    let prompt = arguments.join(" ");
    if prompt.len() > 1024 * 1024 {
        return Err(usage_error("问题内容超过 1 MiB 限制"));
    }
    Ok(prompt)
}

pub fn print_help() {
    println!("OTTO (One-time.Talk once) - CLI 单轮大模型 Agent 工具");
    println!();
    println!("用法:");
    println!("  otto <问题内容...>");
    println!("  otto --mode <模式>");
    println!("  otto --mode");
    println!("  otto --mode <模式> <问题内容...>");
    println!("  otto --mode -- <问题内容...>");
    println!("  otto --root <目录> <问题内容...>");
    println!("  otto --no-agent <问题内容...>");
    println!("  otto --no-stdin <问题内容...>");
    println!("  otto --config");
    println!("  otto --config --name ... --baseurl ... --apikey ... [--model ...]");
    println!("  otto --help");
    println!("  otto --version");
    println!();
    println!("说明:");
    println!("  问题参数会自动用空格拼接，通常不需要加引号。");
    println!("  选项必须位于问题之前；第一个问题参数之后的所有参数均属于问题内容。");
    println!("  如果问题以 - 开头，请使用 otto -- <问题内容...>。");
    println!("  stdin 是管道时会作为附加上下文读取；--no-stdin 可关闭此行为。");
    println!("  普通请求默认加载 system.md，并附加当前保存的模式。");
    println!("  请求默认使用 SSE 流式输出。");
    println!("  Agent 默认启用；--no-agent 可仅发送普通 Chat 请求。");
    println!("  Agent 最大执行轮数可通过 OTTO_MAX_AGENT_ROUNDS 设置（默认 8，范围 0-255，0 表示不限制）。");
}

#[cfg(test)]
mod tests {
    use super::{join_prompt, parse, Command};

    fn arguments(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    #[test]
    fn joins_arguments_with_spaces() {
        let result = join_prompt(&arguments(&["你好", "世界"])).expect("prompt");
        assert_eq!(result, "你好 世界");
    }

    #[test]
    fn parses_persistent_mode() {
        let options = parse(arguments(&["--mode", "otto"])).expect("options");
        assert_eq!(options.command, Command::Mode);
        assert_eq!(options.mode.as_deref(), Some("otto"));
        assert!(!options.raw_mode);
    }

    #[test]
    fn parses_one_shot_raw_mode() {
        let options = parse(arguments(&["--mode", "--", "你好"])).expect("options");
        assert_eq!(options.command, Command::Ask);
        assert!(options.raw_mode);
        assert_eq!(options.prompt, arguments(&["你好"]));
    }

    #[test]
    fn parses_config_values() {
        let options = parse(arguments(&[
            "--config",
            "--name",
            "SiliconFlow",
            "--baseurl=https://example.test/v1",
            "--apikey",
            "secret",
        ]))
        .expect("options");
        assert_eq!(options.command, Command::Config);
        assert!(options.has_config_options);
        assert_eq!(options.name.as_deref(), Some("SiliconFlow"));
        assert_eq!(options.baseurl.as_deref(), Some("https://example.test/v1"));
        assert_eq!(options.apikey.as_deref(), Some("secret"));
    }

    #[test]
    fn parses_workspace_and_agent_flags() {
        let options = parse(arguments(&[
            "--root",
            "workspace",
            "--no-agent",
            "你好",
            "世界",
        ]))
        .expect("options");
        assert_eq!(options.root, Some(std::path::PathBuf::from("workspace")));
        assert!(options.no_agent);
        assert_eq!(options.prompt, arguments(&["你好", "世界"]));
    }

    #[test]
    fn treats_all_arguments_after_the_first_question_argument_as_prompt() {
        let options = parse(arguments(&["python", "-m", "这个指令是什么意思"])).expect("options");
        assert_eq!(
            options.prompt,
            arguments(&["python", "-m", "这个指令是什么意思"])
        );
    }

    #[test]
    fn does_not_parse_options_after_the_question_starts() {
        let options = parse(arguments(&[
            "解释这段命令",
            "--no-agent",
            "--root",
            "workspace",
        ]))
        .expect("options");
        assert!(!options.no_agent);
        assert_eq!(options.root, None);
        assert_eq!(
            options.prompt,
            arguments(&["解释这段命令", "--no-agent", "--root", "workspace"])
        );
    }

    #[test]
    fn accepts_a_dash_prefixed_question_after_the_separator() {
        let options = parse(arguments(&["--", "-m", "这个参数是什么意思"])).expect("options");
        assert_eq!(options.prompt, arguments(&["-m", "这个参数是什么意思"]));
    }

    #[test]
    fn still_rejects_a_dash_prefixed_argument_before_the_question() {
        assert!(parse(arguments(&["-m", "这个参数是什么意思"])).is_err());
    }

    #[test]
    fn parses_raw_mode_with_agent_flags() {
        let options =
            parse(arguments(&["--mode", "--root", "workspace", "你好"])).expect("options");
        assert!(options.raw_mode);
        assert_eq!(options.root, Some(std::path::PathBuf::from("workspace")));
        assert_eq!(options.prompt, arguments(&["你好"]));
    }

    #[test]
    fn parses_no_stdin_flag() {
        let options = parse(arguments(&["--no-stdin", "你好"])).expect("options");
        assert!(options.no_stdin);
        assert_eq!(options.prompt, arguments(&["你好"]));
    }

    #[test]
    fn permits_empty_arguments_for_stdin_only_requests() {
        let options = parse(Vec::<String>::new()).expect("options");
        assert_eq!(options.command, Command::Ask);
        assert!(options.prompt.is_empty());
    }

    #[test]
    fn permits_empty_separator_for_stdin_only_requests() {
        let options = parse(arguments(&["--"])).expect("options");
        assert_eq!(options.command, Command::Ask);
        assert!(options.prompt.is_empty());
    }
}
