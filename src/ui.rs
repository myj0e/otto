use std::fs::OpenOptions;
use std::io::{self, IsTerminal, Write};

use crossterm::cursor::{MoveToColumn, MoveUp};
use crossterm::event::{self, Event, KeyCode, KeyModifiers};
use crossterm::style::{Attribute, Color, ResetColor, SetAttribute, SetForegroundColor};
use crossterm::terminal::{self, Clear, ClearType};
use crossterm::{execute, queue};

use crate::error::{OttoError, Result};
use crate::usage::TokenUsage;
use unicode_width::UnicodeWidthChar;

const ACTION_MAX_CHARS: usize = 4_096;
const BASH_REASON_MAX_CHARS: usize = 800;
const BASH_STEP_MAX_CHARS: usize = 400;
const MENU_MAX_VISIBLE_OPTIONS: usize = 8;
const NOTE_MAX_CHARS: usize = 4_000;

// Keep structural colors in one restrained ANSI palette so panels read as
// distinct sections without overpowering their contents.
const COLOR_BRAND: Color = Color::AnsiValue(109); // muted teal
const COLOR_TOOL_ACTIVITY: Color = Color::AnsiValue(103); // slate blue
const COLOR_FINAL_ANSWER: Color = Color::AnsiValue(108); // sage green
const COLOR_AUTHORIZATION: Color = Color::AnsiValue(144); // soft ochre
const COLOR_ERROR: Color = Color::AnsiValue(138); // muted rose
const COLOR_MUTED: Color = Color::DarkGrey;
const COLOR_BODY: Color = Color::Grey;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthorizationChoice {
    Once,
    AlwaysForTool,
    Deny,
}

struct MenuOption<'a> {
    label: &'a str,
    shortcuts: &'a [char],
}

const AUTHORIZATION_OPTIONS: [MenuOption<'static>; 3] = [
    MenuOption {
        label: "允许工具执行一次",
        shortcuts: &['y', 'Y'],
    },
    MenuOption {
        label: "允许该工具后续的所有执行",
        shortcuts: &['a', 'A'],
    },
    MenuOption {
        label: "不允许执行",
        shortcuts: &['n', 'N'],
    },
];

const WRITE_AUTHORIZATION_OPTIONS: [MenuOption<'static>; 2] = [
    MenuOption {
        label: "允许本次变更",
        shortcuts: &['y', 'Y'],
    },
    MenuOption {
        label: "拒绝变更",
        shortcuts: &['n', 'N'],
    },
];

const BASH_AUTHORIZATION_OPTIONS: [MenuOption<'static>; 2] = [
    MenuOption {
        label: "允许本次执行",
        shortcuts: &['y', 'Y'],
    },
    MenuOption {
        label: "拒绝执行",
        shortcuts: &['n', 'N'],
    },
];

pub fn print_tool_summary(names: &[String]) {
    let mut grouped: Vec<(String, usize)> = Vec::new();
    for raw_name in names {
        let name = sanitize_inline(raw_name, 80);
        if name.is_empty() {
            continue;
        }
        if let Some((_, count)) = grouped
            .iter_mut()
            .find(|(known, _)| known.as_str() == name.as_str())
        {
            *count += 1;
        } else {
            grouped.push((name, 1));
        }
    }
    if grouped.is_empty() {
        return;
    }

    let summary = grouped
        .into_iter()
        .map(|(name, count)| {
            if count == 1 {
                name
            } else {
                format!("{name} ×{count}")
            }
        })
        .collect::<Vec<_>>()
        .join(" · ");

    let stderr = io::stderr();
    let interactive = stderr.is_terminal();
    let mut writer = stderr.lock();
    if !interactive {
        let _ = writeln!(writer, "[otto] 工具 · {summary}");
        let _ = writer.flush();
        return;
    }
    let width = terminal_width(interactive);
    let title = format!("工具活动 · {} 项", names.len());
    let styled = interactive && colors_enabled();
    let _ = write_panel_title(
        &mut writer,
        &title,
        width,
        false,
        COLOR_TOOL_ACTIVITY,
        styled,
    );
    let _ = write_muted_line(&mut writer, &summary, width, styled);

    let _ = writer.flush();
}

/// Add a compact header before a streamed, tool-free response.
pub fn begin_final_answer() -> io::Result<bool> {
    begin_answer_panel("OTTO · 最终答复")
}

pub fn begin_response_stream() -> io::Result<bool> {
    begin_answer_panel("OTTO · 响应")
}

fn begin_answer_panel(title: &str) -> io::Result<bool> {
    let stdout = io::stdout();
    let interactive = stdout.is_terminal();
    if interactive {
        let mut writer = stdout.lock();
        write_panel_title(
            &mut writer,
            title,
            terminal_width(true),
            false,
            COLOR_FINAL_ANSWER,
            colors_enabled(),
        )?;
        writer.flush()?;
    }
    Ok(interactive)
}

/// Close the response section opened by [`begin_final_answer`].
pub fn end_final_answer(interactive: bool) -> io::Result<()> {
    end_answer_panel(interactive)
}

pub fn end_response_stream(interactive: bool) -> io::Result<()> {
    end_answer_panel(interactive)
}

fn end_answer_panel(interactive: bool) -> io::Result<()> {
    if !interactive {
        return Ok(());
    }
    let stdout = io::stdout();
    let mut writer = stdout.lock();
    write_panel_footer(
        &mut writer,
        terminal_width(true),
        COLOR_FINAL_ANSWER,
        colors_enabled(),
    )?;
    writer.flush()
}

/// Print a complete final answer. Piped output stays byte-for-byte plain text.
pub fn print_final_answer(answer: &str) -> io::Result<()> {
    let stdout = io::stdout();
    let interactive = stdout.is_terminal();
    let mut writer = stdout.lock();
    if interactive {
        let width = terminal_width(true);
        let styled = colors_enabled();
        write_panel_title(
            &mut writer,
            "OTTO · 最终答复",
            width,
            false,
            COLOR_FINAL_ANSWER,
            styled,
        )?;
        writer.write_all(sanitize_multiline(answer).as_bytes())?;
        if !answer.ends_with('\n') {
            writer.write_all(b"\n")?;
        }
        write_panel_footer(&mut writer, width, COLOR_FINAL_ANSWER, styled)?;
    } else {
        writer.write_all(answer.as_bytes())?;
        if !answer.ends_with('\n') {
            writer.write_all(b"\n")?;
        }
    }
    writer.flush()
}

pub fn print_status(message: &str) {
    let stdout = io::stdout();
    let interactive = stdout.is_terminal();
    let mut writer = stdout.lock();
    let message = sanitize_inline(message, NOTE_MAX_CHARS);
    if interactive && colors_enabled() {
        let _ = queue!(
            writer,
            SetForegroundColor(COLOR_BRAND),
            crossterm::style::Print("◆ "),
            ResetColor,
            crossterm::style::Print(message),
            crossterm::style::Print("\r\n")
        );
    } else {
        let _ = writeln!(writer, "{message}");
    }
    let _ = writer.flush();
}

/// Report request progress and session state without contaminating answer stdout.
pub fn print_notice(message: &str) {
    let stderr = io::stderr();
    let interactive = stderr.is_terminal();
    let mut writer = stderr.lock();
    let message = sanitize_inline(message, NOTE_MAX_CHARS);
    if interactive && colors_enabled() {
        let _ = queue!(
            writer,
            SetForegroundColor(COLOR_BRAND),
            crossterm::style::Print("◆ "),
            ResetColor,
            crossterm::style::Print(message),
            crossterm::style::Print("\r\n")
        );
    } else {
        let _ = writeln!(writer, "{message}");
    }
    let _ = writer.flush();
}

fn grouped_count(value: u64) -> String {
    let digits = value.to_string();
    let mut output = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, character) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index) % 3 == 0 {
            output.push(',');
        }
        output.push(character);
    }
    output
}

/// Print provider-reported token usage on stderr so scripts that consume the
/// answer from stdout keep a clean response stream.
pub fn print_usage(usage: &TokenUsage, session: bool, context_window: Option<u64>) {
    let input = if usage.input_reports == 0 {
        "—".to_owned()
    } else {
        grouped_count(usage.input_tokens)
    };
    let output = if usage.output_reports == 0 {
        "—".to_owned()
    } else {
        grouped_count(usage.output_tokens)
    };
    let cache = match usage.cache_hit_percent() {
        Some(percent) => format!(
            "{percent}% ({}/{})",
            grouped_count(usage.cache_read_input_tokens),
            grouped_count(usage.cache_read_total_input_tokens)
        ),
        None if usage.requests_with_cache_usage > 0 => "命中数未提供".to_owned(),
        None => "不可用".to_owned(),
    };
    let cache_coverage = if usage.requests_with_cache_usage < usage.requests_with_usage {
        format!(
            " · 缓存数据 {}/{} 次用量请求",
            usage.requests_with_cache_usage, usage.requests_with_usage
        )
    } else {
        String::new()
    };
    let cache_write = if usage.cache_write_input_tokens > 0 {
        format!(
            " · 缓存写入 {}",
            grouped_count(usage.cache_write_input_tokens)
        )
    } else {
        String::new()
    };
    let coverage = if usage.requests_with_usage < usage.requests {
        format!(
            " · 用量数据 {}/{} 次请求",
            usage.requests_with_usage, usage.requests
        )
    } else {
        String::new()
    };
    let field_coverage = if usage.input_reports < usage.requests_with_usage
        || usage.output_reports < usage.requests_with_usage
    {
        format!(
            " · 输入/输出字段 {}/{}、{}/{}",
            usage.input_reports,
            usage.requests_with_usage,
            usage.output_reports,
            usage.requests_with_usage
        )
    } else {
        String::new()
    };
    let context = match (usage.latest_input_tokens, context_window) {
        (Some(input), Some(maximum)) => {
            let tenths = input
                .saturating_mul(1000)
                .checked_div(maximum)
                .unwrap_or_default();
            format!(
                " · 最近请求上下文 {}/{} ({:.1}%)",
                grouped_count(input),
                grouped_count(maximum),
                tenths as f64 / 10.0
            )
        }
        (Some(input), None) => format!(" · 最近请求输入 {}", grouped_count(input)),
        _ => String::new(),
    };
    print_notice(&format!(
        "Token 用量 · {}输入 {input} · 输出 {output} · 缓存命中 {cache}{cache_write}{cache_coverage} · {} 次请求{coverage}{field_coverage}{context}",
        if session { "会话累计" } else { "本轮" },
        usage.requests
    ));
}

pub fn print_screen_header(title: &str, subtitle: &str) {
    let stdout = io::stdout();
    let interactive = stdout.is_terminal();
    let mut writer = stdout.lock();
    let width = terminal_width(interactive);
    let styled = interactive && colors_enabled();
    let _ = write_panel_title(&mut writer, title, width, false, COLOR_BRAND, styled);
    if !subtitle.trim().is_empty() {
        let _ = write_muted_line(&mut writer, subtitle, width, styled);
    }
    let _ = writer.flush();
}

pub fn print_summary(title: &str, fields: &[(&str, &str)]) {
    let stdout = io::stdout();
    let interactive = stdout.is_terminal();
    let mut writer = stdout.lock();
    let width = terminal_width(interactive);
    let styled = interactive && colors_enabled();
    let _ = write_panel_title(&mut writer, title, width, true, COLOR_BRAND, styled);
    for (label, value) in fields {
        let _ = write_panel_field(&mut writer, label, value, COLOR_BODY, width, styled);
    }
    let _ = write_panel_footer(&mut writer, width, COLOR_BRAND, styled);
    let _ = writer.flush();
}

/// Keep redirected session listings stable; use stacked rows in the terminal so
/// long descriptions never compete with the identifier for horizontal space.
pub fn print_sessions(records: &[crate::sessions::SessionRecord]) -> io::Result<()> {
    let stdout = io::stdout();
    let interactive = stdout.is_terminal();
    let mut writer = stdout.lock();
    if records.is_empty() {
        writeln!(writer, "当前 workspace 没有已保存的会话。")?;
    } else if !interactive {
        writeln!(
            writer,
            "SESSION ID                              DESCRIPTION"
        )?;
        for record in records {
            writeln!(
                writer,
                "{}  {}",
                record.id,
                sanitize_terminal_output(&record.description)
            )?;
        }
    } else {
        let width = terminal_width(true);
        let styled = colors_enabled();
        write_panel_title(
            &mut writer,
            &format!("OTTO · 会话 · {} 条", records.len()),
            width,
            false,
            COLOR_BRAND,
            styled,
        )?;
        for record in records {
            write_section_label(&mut writer, &record.id, width, COLOR_BRAND, styled)?;
            let description = sanitize_inline(&record.description, NOTE_MAX_CHARS);
            write_muted_line(
                &mut writer,
                if description.is_empty() {
                    "暂无描述"
                } else {
                    &description
                },
                width,
                styled,
            )?;
        }
        writer.write_all(b"\r\n")?;
    }
    writer.flush()
}

pub fn write_input_prompt(label: &str, default: Option<&str>) -> io::Result<()> {
    let stdout = io::stdout();
    let interactive = stdout.is_terminal();
    let mut writer = stdout.lock();
    if interactive && colors_enabled() {
        queue!(
            writer,
            SetForegroundColor(COLOR_BRAND),
            crossterm::style::Print(format!("  {label}")),
            ResetColor
        )?;
        if let Some(default) = default.filter(|value| !value.is_empty()) {
            queue!(
                writer,
                SetForegroundColor(COLOR_MUTED),
                crossterm::style::Print(format!(" [{default}]")),
                ResetColor
            )?;
        }
        writer.write_all(b": ")?;
    } else if let Some(default) = default.filter(|value| !value.is_empty()) {
        write!(writer, "{label} [{default}]: ")?;
    } else {
        write!(writer, "{label}: ")?;
    }
    writer.flush()
}

pub fn sanitize_terminal_output(value: &str) -> String {
    sanitize_multiline(value)
}

pub fn print_error(message: &str) {
    let stderr = io::stderr();
    let interactive = stderr.is_terminal();
    let mut writer = stderr.lock();
    let message = sanitize_inline(message, NOTE_MAX_CHARS);
    if interactive && colors_enabled() {
        let _ = queue!(
            writer,
            SetForegroundColor(COLOR_ERROR),
            crossterm::style::Print("! "),
            ResetColor,
            crossterm::style::Print("otto: "),
            crossterm::style::Print(message),
            crossterm::style::Print("\r\n")
        );
    } else {
        let _ = writeln!(writer, "otto: {message}");
    }
    let _ = writer.flush();
}

pub struct ModelProgress {
    active: bool,
}

impl ModelProgress {
    pub fn start(label: &str) -> Self {
        let stderr = io::stderr();
        if !stderr.is_terminal() {
            return Self { active: false };
        }
        let mut writer = stderr.lock();
        let label = truncate_terminal_line(
            &sanitize_inline(label, 120),
            terminal_width(true).saturating_sub(9),
        );
        if colors_enabled() {
            let _ = queue!(
                writer,
                SetForegroundColor(COLOR_BRAND),
                crossterm::style::Print("◌ "),
                ResetColor,
                crossterm::style::Print("OTTO · "),
                crossterm::style::Print(label)
            );
        } else {
            let _ = write!(writer, "◌ OTTO · {label}");
        }
        let _ = writer.flush();
        Self { active: true }
    }

    pub fn finish(&mut self) {
        if !self.active {
            return;
        }
        let stderr = io::stderr();
        let mut writer = stderr.lock();
        let _ = execute!(writer, MoveToColumn(0), Clear(ClearType::CurrentLine));
        let _ = writer.flush();
        self.active = false;
    }
}

impl Drop for ModelProgress {
    fn drop(&mut self) {
        self.finish();
    }
}

pub fn select_authorization_with_preview(
    tool_name: &str,
    capability_label: &str,
    mode: Option<&str>,
    action: &str,
    preview: Option<&str>,
) -> Result<AuthorizationChoice> {
    let mut session = TerminalSession::new()?;
    let action = sanitize_inline(action, ACTION_MAX_CHARS);
    let tool_name = sanitize_inline(tool_name, 80);
    let width = terminal_width(true);
    let styled = colors_enabled();
    let title = if capability_label == "写入" {
        format!("授权 · {tool_name} · 修改")
    } else {
        format!("授权 · {tool_name}")
    };
    write_panel_title(
        &mut session.writer,
        &title,
        width,
        true,
        COLOR_AUTHORIZATION,
        styled,
    )
    .map_err(permission_io_error)?;
    if let Some(mode) = mode.filter(|mode| !mode.trim().is_empty()) {
        write_muted_line(
            &mut session.writer,
            &format!("模式 · {}", sanitize_inline(mode, 80)),
            width,
            styled,
        )
        .map_err(permission_io_error)?;
    }
    write_panel_field(
        &mut session.writer,
        "操作",
        &action,
        if capability_label == "写入" {
            COLOR_AUTHORIZATION
        } else {
            COLOR_BODY
        },
        width,
        styled,
    )
    .map_err(permission_io_error)?;
    if let Some(preview) = preview.filter(|value| !value.is_empty()) {
        write_section_label(
            &mut session.writer,
            "变更预览 · 尚未写入",
            width,
            COLOR_AUTHORIZATION,
            styled,
        )
        .map_err(permission_io_error)?;
        for line in sanitize_multiline(preview).lines() {
            let color = if line.starts_with('+') {
                COLOR_FINAL_ANSWER
            } else if line.starts_with('-') {
                COLOR_ERROR
            } else {
                COLOR_BODY
            };
            for part in wrap_terminal_text(line, width.saturating_sub(6).max(2)) {
                if styled {
                    queue!(
                        session.writer,
                        SetForegroundColor(COLOR_MUTED),
                        crossterm::style::Print("  │ "),
                        SetForegroundColor(color),
                        crossterm::style::Print(part),
                        ResetColor
                    )
                    .map_err(permission_io_error)?;
                } else {
                    write!(session.writer, "  │ {part}").map_err(permission_io_error)?;
                }
                session
                    .writer
                    .write_all(b"\r\n")
                    .map_err(permission_io_error)?;
            }
        }
    }
    let options = if capability_label == "写入" {
        &WRITE_AUTHORIZATION_OPTIONS[..]
    } else {
        &AUTHORIZATION_OPTIONS[..]
    };
    let selected = select_menu(
        &mut session.writer,
        options,
        0,
        options.len() - 1,
        "授权选择",
        COLOR_AUTHORIZATION,
    )?;
    write_panel_footer(&mut session.writer, width, COLOR_AUTHORIZATION, styled)
        .map_err(permission_io_error)?;
    if capability_label == "写入" {
        Ok(if selected == 0 {
            AuthorizationChoice::Once
        } else {
            AuthorizationChoice::Deny
        })
    } else {
        Ok(choice_for(selected))
    }
}

/// Show the exact Bash script and require a fresh, per-invocation decision.
/// The menu starts on denial, so Enter confirms denial unless the user moves it.
pub fn select_bash_authorization(
    command: &str,
    working_directory: &str,
    risk: &str,
    risk_is_yellow: bool,
    reason: &str,
    breakdown: &[String],
    mode: Option<&str>,
) -> Result<bool> {
    let mut session = TerminalSession::new()?;
    let (terminal_width, _) = terminal::size().map_err(permission_io_error)?;
    let line_width = usize::from(terminal_width.saturating_sub(1).max(1));
    let styled = colors_enabled();
    write_panel_title(
        &mut session.writer,
        "授权 · bash",
        line_width,
        true,
        COLOR_AUTHORIZATION,
        styled,
    )
    .map_err(permission_io_error)?;
    write_muted_line(
        &mut session.writer,
        "尚未执行 · 模型判断仅供参考",
        line_width,
        styled,
    )
    .map_err(permission_io_error)?;
    if let Some(mode) = mode.filter(|mode| !mode.trim().is_empty()) {
        write_muted_line(
            &mut session.writer,
            &format!("模式 · {}", sanitize_inline(mode, 80)),
            line_width,
            styled,
        )
        .map_err(permission_io_error)?;
    }
    write_panel_field(
        &mut session.writer,
        "目录",
        &sanitize_inline(working_directory, 4096),
        COLOR_BODY,
        line_width,
        styled,
    )
    .map_err(permission_io_error)?;
    let risk_color = if risk_is_yellow {
        COLOR_AUTHORIZATION
    } else {
        COLOR_BRAND
    };
    write_panel_field(
        &mut session.writer,
        "风险",
        &sanitize_inline(risk, 120),
        risk_color,
        line_width,
        styled,
    )
    .map_err(permission_io_error)?;
    write_panel_field(
        &mut session.writer,
        "说明",
        &sanitize_inline(reason, BASH_REASON_MAX_CHARS),
        COLOR_BODY,
        line_width,
        styled,
    )
    .map_err(permission_io_error)?;

    if !breakdown.is_empty() {
        write_panel_field(
            &mut session.writer,
            "结构",
            &breakdown
                .iter()
                .map(|step| sanitize_inline(step, BASH_STEP_MAX_CHARS))
                .collect::<Vec<_>>()
                .join(" · "),
            COLOR_BODY,
            line_width,
            styled,
        )
        .map_err(permission_io_error)?;
    }

    write_section_label(
        &mut session.writer,
        "脚本 · 批准后运行",
        line_width,
        COLOR_BRAND,
        styled,
    )
    .map_err(permission_io_error)?;
    write_muted_line(
        &mut session.writer,
        "判断是模型摘要；批准覆盖整段脚本。",
        line_width,
        styled,
    )
    .map_err(permission_io_error)?;
    write_bash_code_frame(&mut session.writer, "bash", line_width, styled)
        .map_err(permission_io_error)?;
    for line in sanitize_multiline(command).split('\n') {
        write_bash_command_line(&mut session.writer, line, line_width, styled)
            .map_err(permission_io_error)?;
    }
    write_bash_code_frame(&mut session.writer, "", line_width, styled)
        .map_err(permission_io_error)?;
    let selected = select_menu(
        &mut session.writer,
        &BASH_AUTHORIZATION_OPTIONS,
        1,
        1,
        "授权选择",
        COLOR_AUTHORIZATION,
    )?;
    write_panel_footer(&mut session.writer, line_width, COLOR_AUTHORIZATION, styled)
        .map_err(permission_io_error)?;
    Ok(selected == 0)
}

fn terminal_width(interactive: bool) -> usize {
    if interactive {
        terminal::size()
            .map(|(width, _)| usize::from(width.saturating_sub(1).max(1)).min(100))
            .unwrap_or(80)
    } else {
        80
    }
}

fn colors_enabled() -> bool {
    std::env::var_os("NO_COLOR").is_none()
        && !matches!(std::env::var("TERM").as_deref(), Ok("dumb"))
}

fn panel_heading(title: &str, width: usize) -> String {
    let prefix = format!("── {} ", sanitize_inline(title, 120));
    let prefix_width = terminal_text_width(&prefix);
    if prefix_width + 1 <= width {
        format!("{prefix}{}", "─".repeat(width - prefix_width))
    } else {
        truncate_terminal_line(&prefix, width)
    }
}

fn write_panel_title<W: Write>(
    writer: &mut W,
    title: &str,
    width: usize,
    blank_before: bool,
    accent: Color,
    styled: bool,
) -> io::Result<()> {
    if blank_before {
        writer.write_all(b"\r\n")?;
    }
    let heading = panel_heading(title, width);
    if styled {
        queue!(
            writer,
            SetForegroundColor(accent),
            SetAttribute(Attribute::Bold),
            crossterm::style::Print(heading),
            SetAttribute(Attribute::Reset),
            ResetColor
        )?;
    } else {
        writer.write_all(heading.as_bytes())?;
    }
    writer.write_all(b"\r\n")
}

fn write_panel_footer<W: Write>(
    writer: &mut W,
    width: usize,
    accent: Color,
    styled: bool,
) -> io::Result<()> {
    let footer = "─".repeat(width.min(24));
    if styled {
        queue!(
            writer,
            SetForegroundColor(accent),
            crossterm::style::Print(footer),
            ResetColor
        )?;
    } else {
        writer.write_all(footer.as_bytes())?;
    }
    writer.write_all(b"\r\n")
}

fn write_panel_field<W: Write>(
    writer: &mut W,
    label: &str,
    value: &str,
    value_color: Color,
    width: usize,
    styled: bool,
) -> io::Result<()> {
    let label = truncate_terminal_line(
        &sanitize_inline(label, 120),
        terminal_width(true).saturating_sub(9),
    );
    let padding = if width >= 40 {
        10usize.saturating_sub(terminal_text_width(&label))
    } else {
        0
    };
    let prefix = format!("  {label}{}  ", " ".repeat(padding));
    let prefix_width = terminal_text_width(&prefix);
    let value_width = width.saturating_sub(prefix_width).max(2);
    let continuation = " ".repeat(prefix_width);
    for (index, line) in wrap_terminal_text(value, value_width).iter().enumerate() {
        if styled {
            queue!(
                writer,
                SetForegroundColor(COLOR_MUTED),
                crossterm::style::Print(if index == 0 {
                    prefix.as_str()
                } else {
                    continuation.as_str()
                }),
                SetForegroundColor(value_color),
                crossterm::style::Print(line),
                ResetColor
            )?;
        } else {
            write!(
                writer,
                "{}{}",
                if index == 0 {
                    prefix.as_str()
                } else {
                    continuation.as_str()
                },
                line
            )?;
        }
        writer.write_all(b"\r\n")?;
    }
    Ok(())
}

fn write_muted_line<W: Write>(
    writer: &mut W,
    text: &str,
    width: usize,
    styled: bool,
) -> io::Result<()> {
    for (index, line) in wrap_terminal_text(text, width.saturating_sub(5).max(2))
        .iter()
        .enumerate()
    {
        if styled {
            queue!(
                writer,
                SetForegroundColor(COLOR_MUTED),
                crossterm::style::Print(if index == 0 { "  · " } else { "    " }),
                SetForegroundColor(COLOR_BODY),
                crossterm::style::Print(line),
                ResetColor
            )?;
        } else {
            write!(
                writer,
                "{}{}",
                if index == 0 { "  · " } else { "    " },
                line
            )?;
        }
        writer.write_all(b"\r\n")?;
    }
    Ok(())
}

fn write_section_label<W: Write>(
    writer: &mut W,
    text: &str,
    width: usize,
    accent: Color,
    styled: bool,
) -> io::Result<()> {
    writer.write_all(b"\r\n")?;
    let line = truncate_terminal_line(&format!("  ◆ {}", sanitize_inline(text, 120)), width);
    if styled {
        queue!(
            writer,
            SetForegroundColor(accent),
            crossterm::style::Print(line),
            ResetColor
        )?;
    } else {
        writer.write_all(line.as_bytes())?;
    }
    writer.write_all(b"\r\n")
}

fn choice_for(selected: usize) -> AuthorizationChoice {
    match selected {
        0 => AuthorizationChoice::Once,
        1 => AuthorizationChoice::AlwaysForTool,
        _ => AuthorizationChoice::Deny,
    }
}

#[cfg(test)]
fn authorization_opening(mode: Option<&str>) -> &'static str {
    match mode {
        Some(mode) if mode.eq_ignore_ascii_case("otto") => "这波要调用本地工具了，先看清楚再点。",
        Some(mode) if mode.eq_ignore_ascii_case("jarvis") => {
            "检测到需要执行本地工具，请确认授权范围。"
        }
        _ => "需要执行本地工具，请确认授权范围。",
    }
}

fn option_lines(
    options: &[MenuOption<'_>],
    selected: usize,
    visible_count: usize,
    line_width: usize,
) -> Vec<String> {
    let count = options.len().min(visible_count);
    let start = selected
        .saturating_sub(count / 2)
        .min(options.len().saturating_sub(count));

    options
        .iter()
        .enumerate()
        .skip(start)
        .take(count)
        .map(|(index, option)| {
            truncate_terminal_line(
                &format!(
                    "{} {}  {}{}",
                    if selected == index { ">" } else { " " },
                    index + 1,
                    sanitize_inline(option.label, 256),
                    option
                        .shortcuts
                        .first()
                        .map(|shortcut| format!("  ({shortcut})"))
                        .unwrap_or_default()
                ),
                line_width,
            )
        })
        .collect()
}

fn select_menu<W: Write>(
    writer: &mut W,
    options: &[MenuOption<'_>],
    default_index: usize,
    cancel_index: usize,
    prompt: &str,
    accent: Color,
) -> Result<usize> {
    if options.is_empty() || default_index >= options.len() || cancel_index >= options.len() {
        return Err(OttoError::Permission(
            "授权选项配置无效，已拒绝本地工具操作".to_owned(),
        ));
    }

    let mut selected = default_index;
    let (terminal_width, terminal_height) = terminal::size().map_err(permission_io_error)?;
    if terminal_height < 2 {
        return Err(OttoError::Permission(
            "终端高度不足以显示授权选项，已拒绝本地工具操作".to_owned(),
        ));
    }
    let mut line_width = usize::from(terminal_width.saturating_sub(1).max(1));
    let mut visible_count = usize::from(terminal_height - 1).min(MENU_MAX_VISIBLE_OPTIONS);
    let mut rendered_rows = options.len().min(visible_count);
    render_menu(
        writer,
        options,
        selected,
        prompt,
        line_width,
        visible_count,
        None,
        0,
        false,
        accent,
    )
    .map_err(permission_io_error)?;

    loop {
        let event = event::read().map_err(permission_io_error)?;
        if let Event::Resize(new_width, new_height) = event {
            if new_height < 2 {
                return Err(OttoError::Permission(
                    "终端缩放后高度不足，已取消授权并拒绝本地工具操作".to_owned(),
                ));
            }
            let old_options = option_lines(options, selected, visible_count, line_width);
            let old_prompt = menu_prompt(prompt, selected, options.len(), line_width);
            let columns = usize::from(new_width.max(1));
            let wrapped_rows = old_options
                .iter()
                .map(|line| rows_at_width(line, columns))
                .sum::<usize>()
                .saturating_add(rows_at_width(&old_prompt, columns).saturating_sub(1));
            queue!(
                writer,
                MoveUp(wrapped_rows.min(u16::MAX as usize) as u16),
                MoveToColumn(0),
                Clear(ClearType::FromCursorDown)
            )
            .map_err(permission_io_error)?;
            line_width = usize::from(new_width.saturating_sub(1).max(1));
            visible_count = usize::from(new_height - 1).min(MENU_MAX_VISIBLE_OPTIONS);
            rendered_rows = options.len().min(visible_count);
            render_menu(
                writer,
                options,
                selected,
                prompt,
                line_width,
                visible_count,
                None,
                0,
                false,
                accent,
            )
            .map_err(permission_io_error)?;
            continue;
        }
        let Event::Key(key) = event else {
            continue;
        };
        if key.kind == crossterm::event::KeyEventKind::Release {
            continue;
        }

        if key.code == KeyCode::Esc
            || (key.modifiers.contains(KeyModifiers::CONTROL)
                && matches!(key.code, KeyCode::Char('c') | KeyCode::Char('C')))
        {
            render_menu(
                writer,
                options,
                cancel_index,
                prompt,
                line_width,
                visible_count,
                Some(cancel_index),
                rendered_rows,
                true,
                accent,
            )
            .map_err(permission_io_error)?;
            return Ok(cancel_index);
        }

        if let Some(index) = choice_index_for_key(key.code, options) {
            render_menu(
                writer,
                options,
                index,
                prompt,
                line_width,
                visible_count,
                Some(index),
                rendered_rows,
                true,
                accent,
            )
            .map_err(permission_io_error)?;
            return Ok(index);
        }

        let next = match key.code {
            KeyCode::Home => Some(0),
            KeyCode::End => Some(options.len() - 1),
            KeyCode::Up | KeyCode::BackTab => Some(selected.saturating_sub(1)),
            KeyCode::Down | KeyCode::Tab => Some((selected + 1).min(options.len() - 1)),
            KeyCode::Enter | KeyCode::Char('\n') | KeyCode::Char('\r') => {
                render_menu(
                    writer,
                    options,
                    selected,
                    prompt,
                    line_width,
                    visible_count,
                    Some(selected),
                    rendered_rows,
                    true,
                    accent,
                )
                .map_err(permission_io_error)?;
                return Ok(selected);
            }
            _ => None,
        };

        if let Some(next) = next {
            selected = next;
            render_menu(
                writer,
                options,
                selected,
                prompt,
                line_width,
                visible_count,
                None,
                rendered_rows,
                true,
                accent,
            )
            .map_err(permission_io_error)?;
        }
    }
}

fn rows_at_width(text: &str, columns: usize) -> usize {
    let width = terminal_text_width(text);
    width.max(1).saturating_add(columns.max(1) - 1) / columns.max(1)
}

fn choice_index_for_key(code: KeyCode, options: &[MenuOption<'_>]) -> Option<usize> {
    let KeyCode::Char(character) = code else {
        return None;
    };

    if let Some(index) = character
        .to_digit(10)
        .and_then(|number| number.checked_sub(1))
        .map(|number| number as usize)
        .filter(|index| *index < options.len())
    {
        return Some(index);
    }

    options.iter().position(|option| {
        option
            .shortcuts
            .iter()
            .any(|shortcut| *shortcut == character)
    })
}

fn menu_prompt(prompt: &str, selected: usize, option_count: usize, line_width: usize) -> String {
    let hint = if line_width < 48 {
        "↑↓ 选择 · Enter 确认 · Esc 取消".to_owned()
    } else {
        format!(
            "↑↓ 选择 · Enter 确认 · 数字 / 字母快捷选择 · Esc 取消  {}/{} · {prompt}",
            selected + 1,
            option_count
        )
    };
    truncate_terminal_line(&hint, line_width)
}

fn render_menu<W: Write>(
    writer: &mut W,
    options: &[MenuOption<'_>],
    selected: usize,
    prompt: &str,
    line_width: usize,
    visible_count: usize,
    confirmed: Option<usize>,
    previous_rows: usize,
    replace_existing: bool,
    accent: Color,
) -> io::Result<()> {
    let lines = option_lines(options, selected, visible_count, line_width);
    let prompt_line = menu_prompt(prompt, selected, options.len(), line_width);
    if replace_existing {
        queue!(writer, MoveUp(previous_rows as u16), MoveToColumn(0))?;
        for option in &lines {
            queue!(writer, Clear(ClearType::CurrentLine))?;
            write_menu_option(writer, option, accent)?;
            queue!(writer, MoveToColumn(0), crossterm::cursor::MoveDown(1))?;
        }
    } else {
        for option in &lines {
            write_menu_option(writer, option, accent)?;
            writer.write_all(b"\r\n")?;
        }
    }
    let prompt_line = if let Some(index) = confirmed {
        truncate_terminal_line(&format!("{prompt} · {}", options[index].label), line_width)
    } else {
        prompt_line
    };
    queue!(writer, Clear(ClearType::CurrentLine))?;
    if colors_enabled() {
        queue!(writer, SetForegroundColor(COLOR_BODY))?;
    }
    crossterm::queue!(writer, crossterm::style::Print(prompt_line))?;
    if colors_enabled() {
        queue!(writer, ResetColor)?;
    }
    if confirmed.is_some() {
        queue!(writer, crossterm::style::Print("\r\n"))?;
    } else {
        queue!(writer, MoveToColumn(0))?;
    }
    writer.flush()
}

fn write_menu_option<W: Write>(writer: &mut W, option: &str, accent: Color) -> io::Result<()> {
    if option.starts_with('>') && colors_enabled() {
        queue!(
            writer,
            SetForegroundColor(accent),
            SetAttribute(Attribute::Bold),
            SetAttribute(Attribute::Reverse),
            crossterm::style::Print(option),
            SetAttribute(Attribute::Reset),
            ResetColor
        )?;
    } else if colors_enabled() {
        queue!(
            writer,
            SetForegroundColor(COLOR_BODY),
            crossterm::style::Print(option),
            ResetColor
        )?;
    } else {
        writer.write_all(option.as_bytes())?;
    }
    Ok(())
}

fn truncate_terminal_line(value: &str, maximum_width: usize) -> String {
    let maximum_width = maximum_width.max(1);
    let mut output = String::new();
    let mut width = 0usize;
    let mut truncated = false;

    for character in value.chars() {
        let character_width = character.width().unwrap_or(0);
        if width + character_width > maximum_width {
            truncated = true;
            break;
        }
        output.push(character);
        width += character_width;
    }

    if truncated {
        if maximum_width >= 2 {
            while width + 1 > maximum_width {
                let Some(last) = output.pop() else {
                    break;
                };
                width -= last.width().unwrap_or(0);
            }
            output.push('…');
        } else if output.is_empty() {
            output.push('>');
        }
    }

    output
}

fn sanitize_inline(value: &str, maximum: usize) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .take(maximum)
        .collect::<String>()
        .trim()
        .to_owned()
}

fn sanitize_multiline(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '\n' => output.push('\n'),
            '\t' => output.push_str("\\t"),
            '\r' => output.push_str("\\r"),
            '\u{1b}' => output.push_str("\\x1b"),
            value if value.is_control() => {
                output.push_str(&format!("\\u{{{:x}}}", value as u32));
            }
            value => output.push(value),
        }
    }
    output
}

fn write_bash_code_frame<W: Write>(
    writer: &mut W,
    text: &str,
    line_width: usize,
    styled: bool,
) -> io::Result<()> {
    let line = if text.is_empty() {
        format!("  └{}", "─".repeat(line_width.saturating_sub(3).min(12)))
    } else {
        truncate_terminal_line(&format!("  ┌─ {text} ─"), line_width)
    };
    if styled {
        queue!(
            writer,
            SetForegroundColor(COLOR_MUTED),
            crossterm::style::Print(line),
            ResetColor
        )?;
    } else {
        writer.write_all(line.as_bytes())?;
    }
    writer.write_all(b"\r\n")
}

fn write_bash_command_line<W: Write>(
    writer: &mut W,
    line: &str,
    line_width: usize,
    styled: bool,
) -> io::Result<()> {
    let content_width = line_width.saturating_sub(6).max(2);
    for segment in wrap_terminal_text(line, content_width) {
        if styled {
            queue!(
                writer,
                SetForegroundColor(COLOR_MUTED),
                crossterm::style::Print("  │ "),
                SetForegroundColor(COLOR_BRAND),
                crossterm::style::Print(segment),
                ResetColor
            )?;
        } else {
            write!(writer, "  │ {segment}")?;
        }
        writer.write_all(b"\r\n")?;
    }
    Ok(())
}

fn terminal_text_width(text: &str) -> usize {
    text.chars()
        .map(|character| character.width().unwrap_or(0))
        .sum()
}

fn wrap_terminal_text(text: &str, maximum_width: usize) -> Vec<String> {
    let maximum_width = maximum_width.max(2);
    let mut lines = Vec::new();
    let mut line = String::new();
    let mut width = 0usize;

    for character in text.chars() {
        if character == '\n' {
            lines.push(std::mem::take(&mut line));
            width = 0;
            continue;
        }
        let character_width = character.width().unwrap_or(0);
        if !line.is_empty() && width + character_width > maximum_width {
            lines.push(std::mem::take(&mut line));
            width = 0;
        }
        line.push(character);
        width += character_width;
    }

    if !line.is_empty() || lines.is_empty() {
        lines.push(line);
    }
    lines
}

fn permission_io_error(error: impl std::fmt::Display) -> OttoError {
    OttoError::Permission(format!("终端授权交互失败：{error}"))
}

fn ui_writer() -> io::Result<Box<dyn Write>> {
    let stderr = io::stderr();
    if stderr.is_terminal() {
        return Ok(Box::new(stderr));
    }

    #[cfg(unix)]
    {
        return OpenOptions::new()
            .read(true)
            .write(true)
            .open("/dev/tty")
            .map(|file| Box::new(file) as Box<dyn Write>);
    }

    #[cfg(not(unix))]
    {
        Err(io::Error::new(
            io::ErrorKind::NotConnected,
            "没有可用的控制终端",
        ))
    }
}

struct TerminalSession {
    writer: Box<dyn Write>,
}

impl TerminalSession {
    fn new() -> Result<Self> {
        let mut writer = ui_writer().map_err(|error| {
            OttoError::Permission(format!("当前终端不可交互，已拒绝本地工具操作：{error}"))
        })?;
        terminal::enable_raw_mode().map_err(permission_io_error)?;
        if let Err(error) = execute!(writer, crossterm::cursor::Hide) {
            let _ = terminal::disable_raw_mode();
            return Err(permission_io_error(error));
        }
        if let Err(error) = writer.flush() {
            let _ = terminal::disable_raw_mode();
            return Err(permission_io_error(error));
        }
        Ok(Self { writer })
    }
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        let _ = execute!(
            self.writer,
            crossterm::cursor::Show,
            crossterm::style::ResetColor
        );
        let _ = self.writer.flush();
        let _ = terminal::disable_raw_mode();
    }
}

#[cfg(test)]
mod tests {
    use super::{
        authorization_opening, option_lines, rows_at_width, sanitize_inline, AUTHORIZATION_OPTIONS,
        WRITE_AUTHORIZATION_OPTIONS,
    };

    #[test]
    fn wraps_by_display_columns_and_preserves_line_breaks() {
        assert_eq!(super::terminal_text_width("中文 · e\u{301}"), 8);
        assert_eq!(super::wrap_terminal_text("中文ab", 4), vec!["中文", "ab"]);
        assert_eq!(
            super::wrap_terminal_text("one\ntwo", 20),
            vec!["one", "two"]
        );
        assert_eq!(
            super::wrap_terminal_text("e\u{301}ab", 2),
            vec!["e\u{301}a", "b"]
        );
    }

    #[test]
    fn truncation_and_headings_fit_available_columns() {
        for width in 1..100 {
            assert!(
                super::terminal_text_width(&super::truncate_terminal_line(
                    "中文 · hello world",
                    width
                )) <= width
            );
            assert!(
                super::terminal_text_width(&super::panel_heading("OTTO · 配置", width)) <= width
            );
        }
    }

    #[test]
    fn renders_three_separate_options() {
        let lines = option_lines(&AUTHORIZATION_OPTIONS, 1, 8, 80);
        assert_eq!(lines.len(), 3);
        assert!(lines[0].contains("允许工具执行一次"));
        assert!(lines[1].contains("允许该工具后续的所有执行"));
        assert!(lines[2].contains("不允许执行"));
        assert!(lines[1].starts_with('>'));
    }

    #[test]
    fn resize_row_count_uses_display_columns() {
        assert_eq!(rows_at_width("中文标题", 4), 2);
        assert_eq!(rows_at_width("中文标题", 80), 1);
    }

    #[test]
    fn write_authorization_requires_a_fresh_decision_for_each_preview() {
        assert_eq!(WRITE_AUTHORIZATION_OPTIONS.len(), 2);
        assert!(WRITE_AUTHORIZATION_OPTIONS
            .iter()
            .all(|option| !option.label.contains("后续")));
    }

    #[test]
    fn keeps_custom_mode_opening_neutral() {
        assert_eq!(
            authorization_opening(Some("custom-mode")),
            "需要执行本地工具，请确认授权范围。"
        );
    }

    #[test]
    fn removes_control_characters_from_inline_text() {
        assert_eq!(
            sanitize_inline("hello\nworld\u{1b}[31m", 100),
            "hello world [31m"
        );
    }
}
