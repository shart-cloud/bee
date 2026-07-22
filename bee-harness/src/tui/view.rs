//! `view(&App, &mut Frame)` — the full-screen layout (008-grid-tui, US1 T016 + the `?` help overlay
//! T016a; the too-small message from US3 T034 is handled here too).
//!
//! Header · chat (scrollable) · input · footer hint bar. Regions are in fixed positions (FR-007). All
//! color comes from the theme bridge, so `NO_COLOR` degrades to monochrome (FR-014). Panels (US2)
//! render into a right-hand column added in T028.

use ratatui::layout::{Alignment, Constraint, Layout, Position, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};
use ratatui::Frame;

use super::app::{App, Focus, LayoutMode, TurnState};
use super::chat::{Body, Role};
use super::theme_bridge::{dim_style, role_style, role_style_bold};
use crate::viz::theme::Role as ThemeRole;

/// Draw the whole UI for the current model state.
pub fn view(app: &App, frame: &mut Frame<'_>) {
    let area = frame.area();

    if app.layout_mode == LayoutMode::TooSmall {
        too_small(frame, area);
        return;
    }

    // header (1) · chat (fill) · input (3, bordered) · footer (1)
    let [header, chat, input, footer] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(3),
        Constraint::Length(1),
    ])
    .areas(area);

    render_header(app, frame, header);
    // In TwoPane with live panels, split a right-hand panel column off the chat area (FR-012); with
    // no panels (US1) chat keeps the full width, so the US1 layout is untouched.
    if app.layout_mode == LayoutMode::TwoPane && !app.panels.is_empty() {
        let panel_w = (chat.width / 3)
            .clamp(30, 50)
            .min(chat.width.saturating_sub(20));
        let [chat_area, panel_area] =
            Layout::horizontal([Constraint::Min(20), Constraint::Length(panel_w)]).areas(chat);
        render_chat(app, frame, chat_area);
        render_panels(app, frame, panel_area);
    } else {
        render_chat(app, frame, chat);
    }
    render_input(app, frame, input);
    render_footer(frame, footer);

    if app.help_open {
        render_help(frame, area);
    }
}

fn render_header(app: &App, frame: &mut Frame<'_>, area: Rect) {
    let mut spans = vec![
        Span::styled("bee", role_style_bold(ThemeRole::Info)),
        Span::styled("  full-screen", dim_style()),
    ];
    if app.turn == TurnState::Streaming {
        spans.push(Span::styled("  ● working…", role_style(ThemeRole::Accent)));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn render_footer(frame: &mut Frame<'_>, area: Rect) {
    // The 5 most useful keys (contracts/keybindings.md). Dim so it recedes.
    let hint = " Enter send   ↑↓/PgUp·Dn scroll   Tab focus   ? help   q quit ";
    frame.render_widget(Paragraph::new(Line::styled(hint, dim_style())), area);
}

fn render_input(app: &App, frame: &mut Frame<'_>, area: Rect) {
    let focused = app.focus == Focus::Input;
    let border = if focused {
        role_style(ThemeRole::Accent)
    } else {
        dim_style()
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(border)
        .title(Span::styled(" input ", dim_style()));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let text = app.input.text();
    frame.render_widget(
        Paragraph::new(text.as_str()).wrap(Wrap { trim: false }),
        inner,
    );

    // Place the cursor at the end of the buffer when the input is focused (a single-line
    // approximation for the MVP; multi-line cursor tracking is a refinement).
    if focused {
        let last = text.rsplit('\n').next().unwrap_or("");
        let x = inner.x + (last.chars().count() as u16).min(inner.width.saturating_sub(1));
        let y = inner.y + (text.matches('\n').count() as u16).min(inner.height.saturating_sub(1));
        frame.set_cursor_position(Position::new(x, y));
    }
}

fn render_chat(app: &App, frame: &mut Frame<'_>, area: Rect) {
    let width = area.width.max(1);
    let lines = chat_lines(app, width);
    let total = lines.len() as u16;
    let height = area.height;

    // Bottom-anchored virtualized scroll: `scroll` counts lines up from the newest.
    let max_offset = total.saturating_sub(height);
    let scroll_up = app.scroll.min(max_offset as usize) as u16;
    let top = max_offset.saturating_sub(scroll_up);

    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .scroll((top, 0)),
        area,
    );
}

/// Render the right-hand panel column (008-grid-tui, US2 T028): each live panel a bordered block
/// titled with its id, stacked in insertion order and given an equal share of the column height. The
/// panel's widget is drawn richly (truecolor) into the block's inner rect via `buffer_render`, which
/// clips any overflow to the region (FR-012).
fn render_panels(app: &App, frame: &mut Frame<'_>, area: Rect) {
    let n = app.panels.len() as u16;
    if n == 0 {
        return;
    }
    // Equal vertical slices; `Layout` distributes any remainder across the top chunks.
    let slices = Layout::vertical(vec![Constraint::Ratio(1, n as u32); n as usize]).split(area);
    for ((id, spec), rect) in app.panels.iter().zip(slices.iter()) {
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(dim_style())
            .title(Span::styled(
                format!(" {id} "),
                role_style(ThemeRole::Accent),
            ));
        let inner = block.inner(*rect);
        frame.render_widget(block, *rect);
        if inner.width > 0 && inner.height > 0 {
            crate::viz::buffer_render::render_into(spec, inner, frame.buffer_mut());
        }
    }
}

/// Flatten the chat into styled, wrapped lines (the virtualization unit). Inline widgets render as
/// their ASCII form here in the chat flow; the rich truecolor rendering is the panel's job (US2).
fn chat_lines(app: &App, width: u16) -> Vec<Line<'static>> {
    let mut out: Vec<Line<'static>> = Vec::new();
    let wrap_w = width.max(8) as usize;
    for msg in &app.chat {
        match &msg.body {
            Body::Text(t) => {
                let style = line_style(msg.role);
                let prefix = role_prefix(msg.role);
                for (i, wl) in textwrap::wrap(t, wrap_w).into_iter().enumerate() {
                    let content = if i == 0 && !prefix.is_empty() {
                        format!("{prefix}{wl}")
                    } else {
                        wl.into_owned()
                    };
                    out.push(Line::styled(content, style));
                }
            }
            Body::Widget(spec) => {
                for l in spec.to_ascii().lines() {
                    out.push(Line::styled(l.to_string(), dim_style()));
                }
            }
        }
        out.push(Line::raw("")); // blank between messages
    }
    out
}

fn line_style(role: Role) -> Style {
    match role {
        Role::User => role_style_bold(ThemeRole::Accent),
        Role::Assistant => role_style(ThemeRole::Text),
        Role::System => dim_style(),
        Role::Tool => dim_style(),
    }
}

fn role_prefix(role: Role) -> &'static str {
    match role {
        Role::User => "you  ",
        _ => "",
    }
}

fn too_small(frame: &mut Frame<'_>, area: Rect) {
    let msg = Paragraph::new("terminal too small (min 40×10)")
        .alignment(Alignment::Center)
        .style(dim_style());
    frame.render_widget(msg, center(area, 32, 1));
}

fn render_help(frame: &mut Frame<'_>, area: Rect) {
    let keys = [
        ("Enter", "send message (Shift+Enter: newline)"),
        ("↑ ↓", "scroll history / input history"),
        ("PgUp PgDn", "page the chat"),
        ("gg G", "top / bottom"),
        ("Tab", "cycle focus (input · chat)"),
        ("y", "yank current message"),
        ("? Esc", "close this help"),
        ("q  Ctrl-C", "quit"),
    ];
    let mut lines = vec![Line::styled(
        "keybindings",
        role_style_bold(ThemeRole::Info),
    )];
    lines.push(Line::raw(""));
    for (k, desc) in keys {
        lines.push(Line::from(vec![
            Span::styled(format!(" {k:<11}"), role_style(ThemeRole::Accent)),
            Span::styled(desc.to_string(), Style::default()),
        ]));
    }
    let popup = center(area, 46, (lines.len() as u16) + 2);
    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(role_style(ThemeRole::Info))
                .title(" help "),
        ),
        popup,
    );
}

/// A `w×h` rectangle centered in `area` (clamped to it).
fn center(area: Rect, w: u16, h: u16) -> Rect {
    let w = w.min(area.width);
    let h = h.min(area.height);
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    Rect::new(x, y, w, h)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render_spec::RenderSpec;
    use crate::tui::chat::{ChatMessage, Role};
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use ratatui::Terminal;

    // Render the view onto a fixed-size headless backend and return its symbols as text. Symbols are
    // color-independent, so these snapshots are deterministic without touching NO_COLOR (research D12).
    fn render(app: &App, w: u16, h: u16) -> String {
        let mut term = Terminal::new(TestBackend::new(w, h)).expect("test backend");
        term.draw(|f| view(app, f)).expect("draw");
        symbols(term.backend().buffer())
    }

    fn symbols(buf: &Buffer) -> String {
        let mut s = String::new();
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                s.push_str(buf[(x, y)].symbol());
            }
            s.push('\n');
        }
        s
    }

    #[test]
    fn renders_header_chat_input_and_footer() {
        let mut app = App::new(80, 20);
        app.chat.push(ChatMessage::text(Role::User, "hello bee"));
        app.chat
            .push(ChatMessage::text(Role::Assistant, "hi there"));
        let out = render(&app, 80, 20);
        assert!(out.contains("bee"), "header missing:\n{out}");
        assert!(out.contains("you  hello bee"), "user line missing:\n{out}");
        assert!(out.contains("hi there"), "assistant line missing:\n{out}");
        assert!(out.contains("input"), "input border title missing:\n{out}");
        assert!(out.contains("q quit"), "footer hint missing:\n{out}");
    }

    #[test]
    fn too_small_terminal_shows_a_message_not_a_broken_layout() {
        let app = App::new(30, 8); // below the 40×10 floor
        let out = render(&app, 30, 8);
        assert!(out.contains("terminal too small"), "got:\n{out}");
    }

    #[test]
    fn help_overlay_lists_keybindings() {
        let mut app = App::new(80, 20);
        app.help_open = true;
        let out = render(&app, 80, 20);
        assert!(out.contains("keybindings"), "help title missing:\n{out}");
        assert!(out.contains("quit"), "help body missing:\n{out}");
    }

    // --- US2 (T030): a live panel renders beside chat, not over it ------------------------------

    #[test]
    fn panel_renders_beside_chat_in_two_pane() {
        // TwoPane needs ≥120 cols; a panel then splits off a right-hand column (FR-012).
        let mut app = App::new(120, 24);
        app.chat.push(ChatMessage::text(Role::User, "show metrics"));
        app.panels.upsert(
            "metrics",
            RenderSpec::Table {
                title: "Latency".into(),
                headers: vec!["p50".into(), "p99".into()],
                rows: vec![crate::render_spec::Row {
                    cells: vec!["12ms".into(), "88ms".into()],
                    color: None,
                }],
            },
        );
        let out = render(&app, 120, 24);
        assert!(out.contains("metrics"), "panel title missing:\n{out}");
        assert!(out.contains("Latency"), "panel content missing:\n{out}");
        assert!(
            out.contains("you  show metrics"),
            "chat still present beside the panel:\n{out}"
        );
    }

    #[test]
    fn panel_column_appears_only_when_populated() {
        // Same app, same size: the right-hand column exists only once a panel is upserted, so a US1
        // session (no panels) renders exactly as before — chat keeps the full width.
        let base = || {
            let mut a = App::new(120, 24);
            a.chat.push(ChatMessage::text(Role::Assistant, "hi"));
            a
        };
        let empty = render(&base(), 120, 24);
        assert!(
            !empty.contains("metrics"),
            "no panel title when empty:\n{empty}"
        );

        let mut with = base();
        with.panels.upsert("metrics", RenderSpec::Separator);
        let populated = render(&with, 120, 24);
        assert!(
            populated.contains("metrics"),
            "panel appears once populated:\n{populated}"
        );
        assert_ne!(empty, populated, "the panel column changes the layout");
    }
}
