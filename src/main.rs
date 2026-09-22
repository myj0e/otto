mod agent;
mod api;
mod cli;
mod config;
mod error;
mod http;
mod input;
mod permission;
mod prompt;
mod tools;
mod ui;
mod workspace;

use std::io::{self, Write};
use std::process::ExitCode;
use std::time::Duration;

use cli::{CliOptions, Command};
use error::{OttoError, Result};

const VERSION: &str = "1.0.0";

fn handle_mode(options: &CliOptions) -> Result<()> {
    if options.mode.is_none() {
        prompt::set_active(None)?;
        println!("已清除当前附加模式，之后的 otto <问题> 将只使用统一系统提示词。");
        return Ok(());
    }

    let base = prompt::load("system")?
        .ok_or_else(|| OttoError::Config("缺少统一系统提示词文件 system.md".to_owned()))?;
    let mode = options.mode.as_deref().unwrap_or_default();
    let mode_prompt = prompt::load(mode)?
        .ok_or_else(|| OttoError::Config("指定模式没有可用的提示词文件".to_owned()))?;
    let _combined = prompt::combine(&base, Some(&mode_prompt));
    prompt::set_active(Some(mode))?;
    println!("已切换到模式：{mode}");
    Ok(())
}

fn handle_config(options: &CliOptions) -> Result<()> {
    let path = config::config_path()?;
    if !options.has_config_options {
        return config::interactive(&path);
    }

    let name = options.name.as_deref().ok_or_else(|| {
        OttoError::Usage("参数式配置必须同时提供 --name、--baseurl 和 --apikey".to_owned())
    })?;
    let baseurl = options.baseurl.as_deref().ok_or_else(|| {
        OttoError::Usage("参数式配置必须同时提供 --name、--baseurl 和 --apikey".to_owned())
    })?;
    let apikey = options.apikey.as_deref().ok_or_else(|| {
        OttoError::Usage("参数式配置必须同时提供 --name、--baseurl 和 --apikey".to_owned())
    })?;

    config::save_values(&path, name, baseurl, apikey, options.model.as_deref())?;
    println!("配置已保存到 {}", path.display());
    Ok(())
}

fn load_prompts(options: &CliOptions) -> Result<(String, Option<String>)> {
    let base_prompt = prompt::load("system")?
        .ok_or_else(|| OttoError::Config("缺少统一系统提示词文件 system.md".to_owned()))?;
    let (mode_name, mode_prompt) = if options.raw_mode {
        (None, None)
    } else if let Some(mode) = options.mode.as_deref() {
        (
            Some(mode.to_owned()),
            Some(
                prompt::load(mode)?
                    .ok_or_else(|| OttoError::Config("指定模式没有可用的提示词文件".to_owned()))?,
            ),
        )
    } else {
        match prompt::get_active()? {
            Some(mode) => {
                (
                    Some(mode.clone()),
                    Some(prompt::load(&mode)?.ok_or_else(|| {
                        OttoError::Config("当前模式没有可用的提示词文件".to_owned())
                    })?),
                )
            }
            None => (None, None),
        }
    };
    Ok((
        prompt::combine(&base_prompt, mode_prompt.as_deref()),
        mode_name,
    ))
}

async fn ask(options: &CliOptions) -> Result<()> {
    let question = input::build_question(&options.prompt, options.no_stdin)?;
    let path = config::config_path()?;
    let (config, found) = config::load(&path)?;
    if !found {
        return Err(OttoError::Config(
            "尚未配置 API，请先运行 otto --config".to_owned(),
        ));
    }
    config::validate(&config)?;

    let (system_prompt, mode_name) = load_prompts(options)?;
    let adapter = api::adapter_for(&config);
    let endpoint = adapter.endpoint(&config)?;

    if !options.no_agent {
        let search_config_path = config::search_config_path()?;
        let (search_config, _) = config::load_search(&search_config_path)?;
        return agent::run(
            &config,
            &search_config,
            &endpoint,
            &system_prompt,
            &question,
            options.root.as_deref(),
            mode_name.as_deref(),
        )
        .await;
    }

    let messages = vec![
        serde_json::json!({"role": "system", "content": system_prompt}),
        serde_json::json!({"role": "user", "content": question}),
    ];
    let body = adapter.request_body(&config, &messages, &[])?;

    let mut wrote_anything = false;
    let mut last_was_newline = false;
    let mut stdout = io::stdout();
    http::chat_stream_events(
        http::ChatRequest {
            endpoint: &endpoint,
            auth: adapter.auth(config.apikey.as_deref().unwrap_or_default()),
            accept: "text/event-stream",
            body: &body,
            connect_timeout: Duration::from_secs(15),
            timeout: Duration::from_secs(120),
            max_response_bytes: 16 * 1024 * 1024,
        },
        |event| match event {
            http::ChatEvent::Data(value) => {
                if let Some(normalized) = adapter.normalize_event(&value) {
                    if let Some(content) = http::event_content(&normalized) {
                        stdout.write_all(content.as_bytes())?;
                        stdout.flush()?;
                        wrote_anything = true;
                        last_was_newline = content.ends_with('\n');
                    }
                }
                Ok(())
            }
            http::ChatEvent::Done => Ok(()),
        },
    )
    .await?;

    if !wrote_anything {
        return Err(OttoError::Api("API 响应中没有回答内容".to_owned()));
    }
    if !last_was_newline {
        stdout.write_all(b"\n")?;
        stdout.flush()?;
    }
    Ok(())
}

async fn run() -> Result<()> {
    let options = cli::parse(std::env::args().skip(1))?;
    match options.command {
        Command::Help => cli::print_help(),
        Command::Version => println!("otto {VERSION}"),
        Command::Config => handle_config(&options)?,
        Command::Mode => handle_mode(&options)?,
        Command::Ask => ask(&options).await?,
    }
    Ok(())
}

#[tokio::main]
async fn main() -> ExitCode {
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("otto: {error}");
            ExitCode::from(error.exit_code())
        }
    }
}
