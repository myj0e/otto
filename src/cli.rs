use std::path::PathBuf;

use crate::error::{OttoError, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    Ask,
    SessionList,
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
    pub new_session: bool,
    pub session: Option<String>,
    pub session_list: bool,
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
            new_session: false,
            session: None,
            session_list: false,
        }
    }
}

fn usage_error(message: impl Into<String>) -> OttoError {
    OttoError::Usage(message.into())
}

fn take_option_value(
    arguments: &[String],
    index: &mut usize,
    options: &[&str],
) -> Result<Option<String>> {
    let argument = &arguments[*index];
    let Some(option) = options
        .iter()
        .copied()
        .find(|option| argument == option || argument.starts_with(&format!("{option}=")))
    else {
        return Ok(None);
    };

    let value = if argument == option {
        *index += 1;
        if *index >= arguments.len() {
            return Err(usage_error(format!("选项 {} 缺少值", options.join("/"))));
        }
        arguments[*index].clone()
    } else if let Some(value) = argument.strip_prefix(&format!("{option}=")) {
        value.to_owned()
    } else {
        unreachable!("option was matched above");
    };

    if value.is_empty() || value.starts_with("--") {
        return Err(usage_error(format!(
            "选项 {} 缺少有效值",
            options.join("/")
        )));
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

        if let Some(value) = take_option_value(arguments, &mut index, &["--name", "-n"])? {
            options.name = Some(value);
        } else if let Some(value) =
            take_option_value(arguments, &mut index, &["--baseurl", "--base-url", "-b"])?
        {
            options.baseurl = Some(value);
        } else if let Some(value) =
            take_option_value(arguments, &mut index, &["--apikey", "--api-key", "-k"])?
        {
            options.apikey = Some(value);
        } else if let Some(value) = take_option_value(arguments, &mut index, &["--model", "-M"])? {
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

    if let Some(value) = arguments[0]
        .strip_prefix("--mode=")
        .or_else(|| arguments[0].strip_prefix("-m="))
    {
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
        "--" | "--no-agent"
            | "-A"
            | "--no-stdin"
            | "-S"
            | "--root"
            | "-r"
            | "--new-session"
            | "--session"
            | "--session-list"
    ) || arguments[1].starts_with("--root=")
        || arguments[1].starts_with("-r=")
        || arguments[1].starts_with("--session=")
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
            return finish_question_options(options);
        }
        if arguments[index] == "--no-agent" || arguments[index] == "-A" {
            options.no_agent = true;
            index += 1;
            continue;
        }
        if arguments[index] == "--no-stdin" || arguments[index] == "-S" {
            options.no_stdin = true;
            index += 1;
            continue;
        }
        if arguments[index] == "--new-session" {
            options.new_session = true;
            index += 1;
            continue;
        }
        if arguments[index] == "--session-list" {
            options.session_list = true;
            index += 1;
            continue;
        }
        if arguments[index] == "--session" {
            if index + 1 < arguments.len() && !arguments[index + 1].starts_with('-') {
                options.session = Some(arguments[index + 1].clone());
                index += 2;
            } else {
                options.session_list = true;
                index += 1;
            }
            continue;
        }
        if let Some(value) = arguments[index].strip_prefix("--session=") {
            if value.is_empty() {
                options.session_list = true;
            } else {
                options.session = Some(value.to_owned());
            }
            index += 1;
            continue;
        }
        if let Some(value) = take_option_value(arguments, &mut index, &["--root", "-r"])? {
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
        return finish_question_options(options);
    }
    finish_question_options(options)
}

fn finish_question_options(mut options: CliOptions) -> Result<CliOptions> {
    if options.new_session && options.session.is_some() {
        return Err(usage_error("--new-session 与 --session 不能同时使用"));
    }
    if options.session_list
        && (options.new_session || options.session.is_some() || !options.prompt.is_empty())
    {
        return Err(usage_error("列出会话时不能同时指定会话操作或问题"));
    }
    if options.session_list {
        options.command = Command::SessionList;
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
        "--config" | "-c" => parse_config(&arguments),
        argument
            if argument == "--mode"
                || argument.starts_with("--mode=")
                || argument == "-m"
                || argument.starts_with("-m=") =>
        {
            parse_mode(&arguments)
        }
        "--" => Ok(CliOptions {
            prompt: arguments[1..].to_vec(),
            ..CliOptions::default()
        }),
        "--no-agent" | "-A" | "--no-stdin" | "-S" | "--root" | "-r" | "--new-session"
        | "--session" | "--session-list" => {
            parse_question_tail(&arguments, 0, CliOptions::default())
        }
        argument
            if argument.starts_with("--root=")
                || argument.starts_with("-r=")
                || argument.starts_with("--session=") =>
        {
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
    println!("OTTO (One-time.Talk once) - CLI one-shot LLM Agent");
    println!();
    println!("Usage:");
    println!("  otto <question...>");
    println!("  otto --mode <name>");
    println!("  otto -m <name>");
    println!("  otto --mode");
    println!("  otto --mode <name> <question...>");
    println!("  otto --mode -- <question...>");
    println!("  otto --root <directory> <question...>");
    println!("  otto -r <directory> <question...>");
    println!("  otto --new-session <question...>");
    println!("  otto --session <id-or-prefix> <question...>");
    println!("  otto --session");
    println!("  otto --session-list");
    println!("  otto --no-agent <question...>");
    println!("  otto -A <question...>");
    println!("  otto --no-stdin <question...>");
    println!("  otto -S <question...>");
    println!("  otto --config");
    println!("  otto -c");
    println!("  otto --config --name ... --baseurl ... --apikey ... [--model ...]");
    println!("  otto --help");
    println!("  otto --version");
    println!();
    println!("Short options:");
    println!("  -c, --config       Enter configuration");
    println!("  -m, --mode         Set, clear, or use a response mode");
    println!("  -n, --name         Configure the service name");
    println!("  -b, --baseurl      Configure the service Base URL");
    println!("  -k, --apikey       Configure the API key");
    println!("  -M, --model        Configure the model name");
    println!("  -r, --root         Set the workspace root");
    println!("      --new-session  Start and save a new conversation session");
    println!("      --session      Resume a session, or list sessions without an ID");
    println!("      --session-list List saved conversation sessions");
    println!("  -A, --no-agent     Disable Agent tool calls");
    println!("  -S, --no-stdin     Ignore standard input");
    println!("  -h, --help         Show help");
    println!("  -V, --version      Show the version");
    println!();
    println!("Notes:");
    println!("  Question arguments are joined with spaces; quoting is usually optional.");
    println!("  Options must come before the question; every argument after the first question argument is question content.");
    println!("  If the question starts with -, use otto -- <question...>.");
    println!("  Piped stdin is read as additional context; use --no-stdin to disable it.");
    println!("  Normal requests load system.md and the currently selected mode.");
    println!("  Responses use SSE streaming by default.");
    println!("  Agent is enabled by default; --no-agent sends an ordinary Chat request only.");
    println!("  Sessions are stored in the workspace .otto/sessions directory.");
    println!("  Set OTTO_MAX_AGENT_ROUNDS to control the Agent limit (default 8, range 0-255, 0 means unlimited).");
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
    fn parses_short_mode_option() {
        let options = parse(arguments(&["-m", "otto"])).expect("options");
        assert_eq!(options.command, Command::Mode);
        assert_eq!(options.mode.as_deref(), Some("otto"));
        assert!(!options.raw_mode);

        let options = parse(arguments(&["-m=jarvis", "你好"])).expect("options");
        assert_eq!(options.mode.as_deref(), Some("jarvis"));
        assert_eq!(options.prompt, arguments(&["你好"]));
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
    fn parses_short_config_options() {
        let options = parse(arguments(&[
            "-c",
            "-n",
            "SiliconFlow",
            "-b=https://example.test/v1",
            "-k",
            "secret",
            "-M",
            "test-model",
        ]))
        .expect("options");
        assert_eq!(options.command, Command::Config);
        assert_eq!(options.name.as_deref(), Some("SiliconFlow"));
        assert_eq!(options.baseurl.as_deref(), Some("https://example.test/v1"));
        assert_eq!(options.apikey.as_deref(), Some("secret"));
        assert_eq!(options.model.as_deref(), Some("test-model"));
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

        let options =
            parse(arguments(&["-r", "workspace", "-A", "-S", "你好"])).expect("short options");
        assert_eq!(options.root, Some(std::path::PathBuf::from("workspace")));
        assert!(options.no_agent);
        assert!(options.no_stdin);
        assert_eq!(options.prompt, arguments(&["你好"]));
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
        assert!(parse(arguments(&["-x", "这个参数是什么意思"])).is_err());
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
