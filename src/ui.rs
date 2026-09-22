use std::fs::OpenOptions;
use std::io::{self, IsTerminal, Write};

use crossterm::cursor::{MoveToColumn, MoveUp};
use crossterm::event::{self, Event, KeyCode, KeyModifiers};
use crossterm::terminal::{self, Clear, ClearType};
use crossterm::{execute, queue};

use crate::error::{OttoError, Result};

const ACTION_MAX_CHARS: usize = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthorizationChoice {
    Once,
    AlwaysForTool,
    Deny,
}

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
    eprintln!("[otto] ⚙ 工具调用：{summary}");
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
    let opening = authorization_opening(mode);
    let mut selected = 0usize;

    render_initial(
        &mut session.writer,
        opening,
        &tool_name,
        capability_label,
        &action,
        selected,
    )
    .map_err(permission_io_error)?;

    loop {
        let event = event::read().map_err(permission_io_error)?;
        let Event::Key(key) = event else {
            continue;
        };

        let cancel = key.code == KeyCode::Esc
            || (key.modifiers.contains(KeyModifiers::CONTROL)
                && matches!(key.code, KeyCode::Char('c') | KeyCode::Char('C')));
        if cancel {
            finish_selection(&mut session.writer, AuthorizationChoice::Deny)
                .map_err(permission_io_error)?;
            return Ok(AuthorizationChoice::Deny);
        }

        let shortcut = match key.code {
            KeyCode::Char('1') | KeyCode::Char('y') | KeyCode::Char('Y') => {
                Some(AuthorizationChoice::Once)
            }
            KeyCode::Char('2') | KeyCode::Char('a') | KeyCode::Char('A') => {
                Some(AuthorizationChoice::AlwaysForTool)
            }
            KeyCode::Char('3') | KeyCode::Char('n') | KeyCode::Char('N') => {
                Some(AuthorizationChoice::Deny)
            }
            _ => None,
        };
        if let Some(choice) = shortcut {
            finish_selection(&mut session.writer, choice).map_err(permission_io_error)?;
            return Ok(choice);
        }

        let next = match key.code {
            KeyCode::Up => Some(selected.saturating_sub(1)),
            KeyCode::Down => Some((selected + 1).min(2)),
            KeyCode::Enter | KeyCode::Char('\n') | KeyCode::Char('\r') => {
                let choice = choice_for(selected);
                finish_selection(&mut session.writer, choice).map_err(permission_io_error)?;
                return Ok(choice);
            }
            _ => None,
        };

        if let Some(next) = next {
            selected = next;
            render_options(&mut session.writer, selected).map_err(permission_io_error)?;
        }
    }
}

fn choice_for(selected: usize) -> AuthorizationChoice {
    match selected {
        0 => AuthorizationChoice::Once,
        1 => AuthorizationChoice::AlwaysForTool,
        _ => AuthorizationChoice::Deny,
    }
}

fn authorization_opening(mode: Option<&str>) -> &'static str {
    match mode {
        Some(mode) if mode.eq_ignore_ascii_case("otto") => "这波要调用本地工具了，先看清楚再点。",
        Some(mode) if mode.eq_ignore_ascii_case("jarvis") => {
            "检测到需要执行本地工具，请确认授权范围。"
        }
        _ => "需要执行本地工具，请确认授权范围。",
    }
}

fn option_lines(selected: usize) -> [String; 3] {
    [
        format!(
            "{} 1. 允许工具执行一次",
            if selected == 0 { "❯" } else { " " }
        ),
        format!(
            "{} 2. 允许该工具后续的所有执行",
            if selected == 1 { "❯" } else { " " }
        ),
        format!("{} 3. 不允许执行", if selected == 2 { "❯" } else { " " }),
    ]
}

fn render_initial<W: Write>(
    writer: &mut W,
    opening: &str,
    tool_name: &str,
    capability_label: &str,
    action: &str,
    selected: usize,
) -> io::Result<()> {
    let options = option_lines(selected);
    write!(
        writer,
        "\r\n{opening}\r\n工具：{tool_name}\r\n能力：{capability_label}\r\n操作：{action}\r\n\r\n{}\r\n{}\r\n{}\r\n授权选择 [1/2/3]: ",
        options[0],
        options[1],
        options[2]
    )?;
    writer.flush()
}

fn render_options<W: Write>(writer: &mut W, selected: usize) -> io::Result<()> {
    let options = option_lines(selected);
    queue!(writer, MoveUp(3), MoveToColumn(0))?;
    for option in &options {
        queue!(
            writer,
            Clear(ClearType::CurrentLine),
            crossterm::style::Print(option)
        )?;
        queue!(writer, MoveToColumn(0), crossterm::cursor::MoveDown(1))?;
    }
    queue!(
        writer,
        Clear(ClearType::CurrentLine),
        crossterm::style::Print("授权选择 [1/2/3]: "),
        MoveToColumn(0)
    )?;
    writer.flush()
}

fn finish_selection<W: Write>(writer: &mut W, choice: AuthorizationChoice) -> io::Result<()> {
    let selected = match choice {
        AuthorizationChoice::Once => "1",
        AuthorizationChoice::AlwaysForTool => "2",
        AuthorizationChoice::Deny => "3",
    };
    queue!(
        writer,
        Clear(ClearType::CurrentLine),
        crossterm::style::Print(format!("授权选择 [1/2/3]: {selected}\r\n"))
    )?;
    writer.flush()
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
    use super::{authorization_opening, option_lines, sanitize_inline};

    #[test]
    fn renders_three_separate_options() {
        let lines = option_lines(1);
        assert_eq!(lines.len(), 3);
        assert!(lines[0].contains("允许工具执行一次"));
        assert!(lines[1].contains("允许该工具后续的所有执行"));
        assert!(lines[2].contains("不允许执行"));
        assert!(lines[1].starts_with('❯'));
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
