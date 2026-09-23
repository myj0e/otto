mod agent;
mod api;
mod cli;
mod config;
mod error;
mod http;
mod input;
mod instructions;
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

const VERSION: &str = "1.5.0";

fn handle_mode(options: &CliOptions) -> Result<()> {
    if options.mode.is_none() {
        prompt::set_active(None)?;
        ui::print_status("已清除附加模式 · 后续请求使用统一系统提示词");
        return Ok(());
    }

    let base = prompt::load("system")?
        .ok_or_else(|| OttoError::Config("缺少统一系统提示词文件 system.md".to_owned()))?;
    let mode = options.mode.as_deref().unwrap_or_default();
    let mode_prompt = prompt::load(mode)?
        .ok_or_else(|| OttoError::Config("指定模式没有可用的提示词文件".to_owned()))?;
    let _combined = prompt::combine(&base, Some(&mode_prompt));
    prompt::set_active(Some(mode))?;
    ui::print_status(&format!("已切换模式 · {mode}"));
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
    ui::print_status(&format!("配置已保存 · {}", path.display()));
    Ok(())
}

fn load_prompts(options: &CliOptions) -> Result<(String, Option<String>)> {
    let base_prompt = prompt::load("system")?
        .ok_or_else(|| OttoError::Config("缺少统一系统提示词文件 system.md".to_owned()))?;
    let workspace = workspace::Workspace::new(options.root.as_deref())?;
    let current_dir = std::env::current_dir()?;
    let project_instructions = instructions::discover(&workspace, &current_dir)?;
    let project_prompt = project_instructions.render();
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
        prompt::combine_layers(
            &base_prompt,
            project_prompt.as_deref(),
            mode_prompt.as_deref(),
        ),
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
    let mut answer_panel = None;
    let mut stdout = io::stdout();
    let mut progress = ui::ModelProgress::start("正在生成响应");
    let stream_result = http::chat_stream_events(
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
                        if answer_panel.is_none() {
                            progress.finish();
                            answer_panel = Some(ui::begin_final_answer()?);
                        }
                        let display_content = if answer_panel == Some(true) {
                            ui::sanitize_terminal_output(&content)
                        } else {
                            content.to_owned()
                        };
                        stdout.write_all(display_content.as_bytes())?;
                        stdout.flush()?;
                        wrote_anything = true;
                        last_was_newline = display_content.ends_with('\n');
                    }
                }
                Ok(())
            }
            http::ChatEvent::Done => Ok(()),
        },
    )
    .await;
    progress.finish();
    if let Err(error) = stream_result {
        if wrote_anything && !last_was_newline {
            let _ = stdout.write_all(b"\n");
            let _ = stdout.flush();
        }
        if let Some(interactive) = answer_panel {
            let _ = ui::end_final_answer(interactive);
        }
        return Err(error);
    }

    if !wrote_anything {
        return Err(OttoError::Api("API 响应中没有回答内容".to_owned()));
    }
    if !last_was_newline {
        stdout.write_all(b"\n")?;
        stdout.flush()?;
    }
    if let Some(interactive) = answer_panel {
        ui::end_final_answer(interactive)?;
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
            ui::print_error(&error.to_string());
            ExitCode::from(error.exit_code())
        }
    }
}
