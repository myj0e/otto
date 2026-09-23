use std::fs::OpenOptions;
use std::io::{self, IsTerminal, Write};

use crossterm::cursor::{MoveToColumn, MoveUp};
use crossterm::event::{self, Event, KeyCode, KeyModifiers};
use crossterm::style::{Attribute, Color, ResetColor, SetAttribute, SetForegroundColor};
use crossterm::terminal::{self, Clear, ClearType};
use crossterm::{execute, queue};

use crate::error::{OttoError, Result};

const ACTION_MAX_CHARS: usize = 4_096;
const BASH_REASON_MAX_CHARS: usize = 800;
const BASH_STEP_MAX_CHARS: usize = 400;
const MENU_MAX_VISIBLE_OPTIONS: usize = 8;
const NOTE_MAX_CHARS: usize = 4_000;

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
    let _ = write_panel_title(&mut writer, &title, width, false, styled);
    let _ = write_muted_line(&mut writer, &summary, width, styled);
    let _ = write_panel_footer(&mut writer, width, styled);
    let _ = writer.flush();
}

/// Render the assistant's short pre-tool note separately from both tool
/// activity and the final answer. This is user-facing progress, not reasoning.
pub fn print_execution_note(note: &str) {
    let note = sanitize_inline(note, NOTE_MAX_CHARS);
    if note.is_empty() {
        return;
    }

    let stderr = io::stderr();
    let interactive = stderr.is_terminal();
    let mut writer = stderr.lock();
    let width = terminal_width(interactive);
    if interactive {
        let styled = colors_enabled();
        let _ = write_panel_title(&mut writer, "执行说明", width, true, styled);
        let _ = write_muted_line(&mut writer, &note, width, styled);
        let _ = write_panel_footer(&mut writer, width, styled);
    } else {
        let _ = writeln!(writer, "[otto] 执行说明：{note}");
    }
    let _ = writer.flush();
}

/// Add a compact header before a streamed, tool-free response.
pub fn begin_final_answer() -> io::Result<bool> {
    let stdout = io::stdout();
    let interactive = stdout.is_terminal();
    if interactive {
        let mut writer = stdout.lock();
        write_panel_title(
            &mut writer,
            "OTTO · 最终答复",
            terminal_width(true),
            false,
            colors_enabled(),
        )?;
        writer.flush()?;
    }
    Ok(interactive)
}

/// Close the response section opened by [`begin_final_answer`].
pub fn end_final_answer(interactive: bool) -> io::Result<()> {
    if !interactive {
        return Ok(());
    }
    let stdout = io::stdout();
    let mut writer = stdout.lock();
    write_panel_footer(&mut writer, terminal_width(true), colors_enabled())?;
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
        write_panel_title(&mut writer, "OTTO · 最终答复", width, false, styled)?;
        writer.write_all(sanitize_multiline(answer).as_bytes())?;
        if !answer.ends_with('\n') {
            writer.write_all(b"\n")?;
        }
        write_panel_footer(&mut writer, width, styled)?;
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
            SetForegroundColor(Color::DarkCyan),
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

pub fn print_screen_header(title: &str, subtitle: &str) {
    let stdout = io::stdout();
    let interactive = stdout.is_terminal();
    let mut writer = stdout.lock();
    let width = terminal_width(interactive);
    let styled = interactive && colors_enabled();
    let _ = write_panel_title(&mut writer, title, width, false, styled);
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
    let _ = write_panel_title(&mut writer, title, width, true, styled);
    for (label, value) in fields {
        let _ = write_panel_field(&mut writer, label, value, Color::Grey, width, styled);
    }
    let _ = write_panel_footer(&mut writer, width, styled);
    let _ = writer.flush();
}

pub fn write_input_prompt(label: &str, default: Option<&str>) -> io::Result<()> {
    let stdout = io::stdout();
    let interactive = stdout.is_terminal();
    let mut writer = stdout.lock();
    if interactive && colors_enabled() {
        queue!(
            writer,
            SetForegroundColor(Color::DarkCyan),
            crossterm::style::Print(format!("  {label}")),
            ResetColor
        )?;
        if let Some(default) = default.filter(|value| !value.is_empty()) {
            queue!(
                writer,
                SetForegroundColor(Color::DarkGrey),
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
            SetForegroundColor(Color::DarkYellow),
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
        let label = sanitize_inline(label, 120);
        if colors_enabled() {
            let _ = queue!(
                writer,
                SetForegroundColor(Color::DarkCyan),
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

pub fn select_authorization(
    tool_name: &str,
    capability_label: &str,
    mode: Option<&str>,
    action: &str,
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
    write_panel_title(&mut session.writer, &title, width, true, styled)
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
            Color::DarkYellow
        } else {
            Color::Grey
        },
        width,
        styled,
    )
    .map_err(permission_io_error)?;
    let selected = select_menu(
        &mut session.writer,
        &AUTHORIZATION_OPTIONS,
        0,
        2,
        "授权选择",
    )?;
    write_panel_footer(&mut session.writer, width, styled).map_err(permission_io_error)?;
    Ok(choice_for(selected))
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
    write_panel_title(&mut session.writer, "授权 · bash", line_width, true, styled)
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
        Color::Grey,
        line_width,
        styled,
    )
    .map_err(permission_io_error)?;
    let risk_color = if risk_is_yellow {
        Color::DarkYellow
    } else {
        Color::DarkCyan
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
        Color::Grey,
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
            Color::Grey,
            line_width,
            styled,
        )
        .map_err(permission_io_error)?;
    }

    write_section_label(&mut session.writer, "脚本 · 批准后运行", line_width, styled)
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
    )?;
    write_panel_footer(&mut session.writer, line_width, styled).map_err(permission_io_error)?;
    Ok(selected == 0)
}

fn terminal_width(interactive: bool) -> usize {
    if interactive {
        terminal::size()
            .map(|(width, _)| usize::from(width.saturating_sub(1).max(1)))
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
    let prefix = format!("╭─ {} ", sanitize_inline(title, 120));
    let prefix_width = terminal_text_width(&prefix);
    if prefix_width + 1 <= width {
        format!("{prefix}{}╮", "─".repeat(width - prefix_width - 1))
    } else {
        truncate_terminal_line(&prefix, width)
    }
}

fn write_panel_title<W: Write>(
    writer: &mut W,
    title: &str,
    width: usize,
    blank_before: bool,
    styled: bool,
) -> io::Result<()> {
    if blank_before {
        writer.write_all(b"\r\n")?;
    }
    let heading = panel_heading(title, width);
    if styled {
        queue!(
            writer,
            SetForegroundColor(Color::DarkCyan),
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

fn write_panel_footer<W: Write>(writer: &mut W, width: usize, styled: bool) -> io::Result<()> {
    let width = width.max(2);
    let footer = format!("╰{}", "─".repeat(width - 1));
    if styled {
        queue!(
            writer,
            SetForegroundColor(Color::DarkGrey),
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
    let prefix = format!("  {label}: ");
    let prefix_width = terminal_text_width(&prefix);
    let value_width = width.saturating_sub(prefix_width).max(2);
    let continuation = " ".repeat(prefix_width);
    for (index, line) in wrap_terminal_text(value, value_width).iter().enumerate() {
        if styled {
            queue!(
                writer,
                SetForegroundColor(Color::DarkGrey),
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
                SetForegroundColor(Color::DarkGrey),
                crossterm::style::Print(if index == 0 { "  · " } else { "    " }),
                SetForegroundColor(Color::Grey),
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
    styled: bool,
) -> io::Result<()> {
    writer.write_all(b"\r\n")?;
    let line = truncate_terminal_line(&format!("  ◆ {}", sanitize_inline(text, 120)), width);
    if styled {
        queue!(
            writer,
            SetForegroundColor(Color::DarkCyan),
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
    let line_width = usize::from(terminal_width.saturating_sub(1).max(1));
    let visible_count = usize::from(terminal_height - 1).min(MENU_MAX_VISIBLE_OPTIONS);
    let rendered_rows = options.len().min(visible_count);
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
    )
    .map_err(permission_io_error)?;

    loop {
        let event = event::read().map_err(permission_io_error)?;
        let Event::Key(key) = event else {
            continue;
        };

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
            )
            .map_err(permission_io_error)?;
            return Ok(index);
        }

        let next = match key.code {
            KeyCode::Up => Some(selected.saturating_sub(1)),
            KeyCode::Down => Some((selected + 1).min(options.len() - 1)),
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
            )
            .map_err(permission_io_error)?;
        }
    }
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
    truncate_terminal_line(
        &format!(
            "↑↓ 移动 · Enter 确认 · Esc 取消  ·  {}/{} · {prompt}",
            selected + 1,
            option_count
        ),
        line_width,
    )
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
) -> io::Result<()> {
    let lines = option_lines(options, selected, visible_count, line_width);
    let prompt_line = menu_prompt(prompt, selected, options.len(), line_width);
    if replace_existing {
        queue!(writer, MoveUp(previous_rows as u16), MoveToColumn(0))?;
        for option in &lines {
            queue!(writer, Clear(ClearType::CurrentLine))?;
            write_menu_option(writer, option)?;
            queue!(writer, MoveToColumn(0), crossterm::cursor::MoveDown(1))?;
        }
    } else {
        for option in &lines {
            write_menu_option(writer, option)?;
            writer.write_all(b"\r\n")?;
        }
    }
    let prompt_line = if let Some(index) = confirmed {
        truncate_terminal_line(&format!("{prompt} · 已选择 {}", index + 1), line_width)
    } else {
        prompt_line
    };
    queue!(writer, Clear(ClearType::CurrentLine))?;
    if colors_enabled() {
        queue!(writer, SetForegroundColor(Color::Grey))?;
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

fn write_menu_option<W: Write>(writer: &mut W, option: &str) -> io::Result<()> {
    if option.starts_with('>') && colors_enabled() {
        queue!(
            writer,
            SetForegroundColor(Color::DarkCyan),
            SetAttribute(Attribute::Bold),
            crossterm::style::Print(option),
            SetAttribute(Attribute::Reset),
            ResetColor
        )?;
    } else if colors_enabled() {
        queue!(
            writer,
            SetForegroundColor(Color::Grey),
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
        let character_width = if character.is_ascii() { 1 } else { 2 };
        if width + character_width > maximum_width {
            truncated = true;
            break;
        }
        output.push(character);
        width += character_width;
    }

    if truncated {
        if maximum_width >= 2 {
            while width + 2 > maximum_width {
                let Some(last) = output.pop() else {
                    break;
                };
                width -= if last.is_ascii() { 1 } else { 2 };
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
            SetForegroundColor(Color::DarkGrey),
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
                SetForegroundColor(Color::DarkGrey),
                crossterm::style::Print("  │ "),
                SetForegroundColor(Color::DarkCyan),
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
        .map(|character| if character.is_ascii() { 1 } else { 2 })
        .sum()
}

fn wrap_terminal_text(text: &str, maximum_width: usize) -> Vec<String> {
    let maximum_width = maximum_width.max(2);
    let mut lines = Vec::new();
    let mut line = String::new();
    let mut width = 0usize;

    for character in text.chars() {
        let character_width = if character.is_ascii() { 1 } else { 2 };
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
    use super::{authorization_opening, option_lines, sanitize_inline, AUTHORIZATION_OPTIONS};

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
