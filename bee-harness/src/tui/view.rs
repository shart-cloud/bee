//! `view(&App, &mut Frame)` — the full-screen layout (008-grid-tui, US1 T016 + the `?` help overlay
//! T016a; the too-small message from US3 T034 is handled here too).
//!
//! Header · chat (scrollable) · input · footer hint bar. Regions are in fixed positions (FR-007). All
//! color comes from the theme bridge, so `NO_COLOR` degrades to monochrome (FR-014). Panels (US2)
//! render into a right-hand column added in T028.

use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Constraint, Layout, Position, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};
use ratatui::Frame;

use super::app::{App, Focus, LayoutMode, TurnState};
use super::chat::{Body, Role};
use super::panels::{self, PanelArea};
use super::theme_bridge::{dim_style, role_style, role_style_bold};
use crate::render_spec::RenderSpec;
use crate::viz::theme::Role as ThemeRole;

/// Draw the whole UI for the current model state.
///
/// Takes `&mut App` because the frame is where the effects pipeline lives (009 FR-001): panel
/// transitions can only be registered once the layout has assigned each panel a `Rect`, and the
/// effects pass mutates the buffer after every widget has drawn into it.
pub fn view(app: &mut App, frame: &mut Frame<'_>) {
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
        let panel_w = panel_column_width(chat.width, app.visual.level);
        let [chat_area, panel_area] =
            Layout::horizontal([Constraint::Min(20), Constraint::Length(panel_w)]).areas(chat);
        render_chat(app, frame, chat_area);
        render_panels(app, frame, panel_area);
    } else {
        render_chat(app, frame, chat);
    }
    render_input(app, frame, input);
    render_footer(app, frame, footer);

    // Single-pane layouts have no panel column, so `p` overlays the panels above chat (US3 T036).
    if app.overlay_open() {
        render_panel_overlay(app, frame, area);
    }
    if app.help_open {
        render_help(frame, area);
    }

    // Effects transform cells that are already drawn (FR-001), so this is the last thing the frame
    // does: every widget above has painted, and ratatui flushes as soon as we return.
    app.effects.process(app.dt, frame.buffer_mut(), area);
}

/// The single-pane panel overlay (US3 T036): panels stacked in a bordered popup over the chat, so a
/// narrow terminal can still see model-owned output. Toggled with `p`, closed with `p`/`Esc`.
fn render_panel_overlay(app: &mut App, frame: &mut Frame<'_>, area: Rect) {
    let w = area.width.saturating_sub(4).clamp(20, 72);
    let h = area.height.saturating_sub(4).max(5);
    let popup = center(area, w, h);
    frame.render_widget(Clear, popup);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(role_style(ThemeRole::Info))
        .title(Span::styled(
            format!(" panels ({}) — p/Esc to close ", app.panels.len()),
            role_style(ThemeRole::Info),
        ));
    let inner = block.inner(popup);
    frame.render_widget(block, popup);
    if inner.width > 0 && inner.height > 0 {
        render_panels(app, frame, inner);
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

fn render_footer(app: &App, frame: &mut Frame<'_>, area: Rect) {
    // The most useful keys (contracts/keybindings.md), trimmed to what actually fits so a narrow
    // terminal never truncates mid-hint. `p` is advertised only where the overlay is the *only* way
    // to see panels — exactly the case where it matters most.
    let show_p = !app.panels.is_empty() && app.layout_mode == LayoutMode::SinglePane;
    let mut parts: Vec<&str> = vec!["Enter send", "↑↓/PgUp·Dn scroll", "Tab focus"];
    if show_p {
        parts.push("p panels");
    }
    parts.extend(["y yank", "? help", "q quit"]);

    // Shed the least essential hints first; Enter/help/quit (and `p` when shown) always survive.
    let width = area.width as usize;
    let rendered = |parts: &[&str]| format!(" {} ", parts.join("   "));
    for droppable in ["y yank", "↑↓/PgUp·Dn scroll", "Tab focus"] {
        if rendered(&parts).chars().count() <= width {
            break;
        }
        parts.retain(|p| *p != droppable);
    }
    frame.render_widget(
        Paragraph::new(Line::styled(rendered(&parts), dim_style())),
        area,
    );
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

/// The tallest an inline widget may grow in the chat flow, so one big chart can't swallow the pane.
const MAX_INLINE_WIDGET_ROWS: u16 = 24;

/// One laid-out row-unit of the chat flow. Text is pre-wrapped to the pane width, so every `Line` is
/// exactly one row and the scroll arithmetic is exact (no hidden re-wrap by `Paragraph`).
enum Item<'a> {
    Line(Line<'static>),
    /// An inline widget and the number of rows it occupies.
    Widget(&'a RenderSpec, u16),
}

impl Item<'_> {
    fn height(&self) -> u16 {
        match self {
            Item::Line(_) => 1,
            Item::Widget(_, h) => *h,
        }
    }
}

fn render_chat(app: &App, frame: &mut Frame<'_>, area: Rect) {
    let width = area.width.max(1);
    let items = chat_items(app, width);
    let total: u16 = items.iter().map(|i| i.height()).sum();
    let height = area.height;

    // Bottom-anchored virtualized scroll: `scroll` counts rows up from the newest. Heights here are
    // the *rendered* heights, so the newest content always lands flush against the bottom edge.
    let max_offset = total.saturating_sub(height);
    let scroll_up = app.scroll.min(max_offset as usize) as u16;
    let top = max_offset.saturating_sub(scroll_up);
    let bottom = top.saturating_add(height);

    let mut y = 0u16; // absolute row index within the whole flow
    for item in &items {
        let h = item.height();
        let end = y.saturating_add(h);
        // Skip anything entirely above or below the visible window.
        if end > top && y < bottom {
            let skip = top.saturating_sub(y); // rows of this item hidden above the fold
            let draw_y = area.y + y.saturating_sub(top);
            let avail = (bottom.min(end) - top.max(y)).min(area.bottom().saturating_sub(draw_y));
            match item {
                Item::Line(l) => {
                    if avail > 0 {
                        frame.render_widget(
                            Paragraph::new(l.clone()),
                            Rect::new(area.x, draw_y, area.width, 1),
                        );
                    }
                }
                Item::Widget(spec, wh) => {
                    blit_widget(frame, spec, *wh, width, area, draw_y, skip, avail);
                }
            }
        }
        y = end;
    }
}

/// Draw an inline widget richly (the same truecolor path panels use) into the chat flow, honoring
/// partial scroll: the widget is rendered once into a scratch buffer at full height, then the visible
/// row window is blitted into the frame. This is what makes an inline `render(widget)` look like a
/// real visualization instead of a dim ASCII blob.
#[allow(clippy::too_many_arguments)]
fn blit_widget(
    frame: &mut Frame<'_>,
    spec: &RenderSpec,
    widget_rows: u16,
    width: u16,
    area: Rect,
    draw_y: u16,
    skip: u16,
    avail: u16,
) {
    if avail == 0 || width == 0 || widget_rows == 0 {
        return;
    }
    let full = Rect::new(0, 0, width, widget_rows);
    let mut scratch = Buffer::empty(full);
    crate::viz::buffer_render::render_into(spec, full, &mut scratch);

    let dst = frame.buffer_mut();
    for row in 0..avail {
        let src_y = skip + row;
        if src_y >= widget_rows {
            break;
        }
        let dy = draw_y + row;
        if dy >= area.bottom() {
            break;
        }
        for x in 0..width.min(area.width) {
            let dx = area.x + x;
            if dx >= area.right() {
                break;
            }
            dst[(dx, dy)] = scratch[(x, src_y)].clone();
        }
    }
}

/// Lay the chat out into row-units (see [`Item`]). Text wraps to the pane width; widgets get their
/// natural rendered height, capped so one chart can't fill the pane.
fn chat_items(app: &App, width: u16) -> Vec<Item<'_>> {
    let mut out: Vec<Item<'_>> = Vec::new();
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
                    out.push(Item::Line(Line::styled(content, style)));
                }
            }
            Body::Widget(spec) => {
                let h = crate::viz::buffer_render::spec_height(spec, width)
                    .clamp(1, MAX_INLINE_WIDGET_ROWS);
                out.push(Item::Widget(spec, h));
            }
        }
        out.push(Item::Line(Line::raw(""))); // blank between messages
    }
    out
}

/// Render the right-hand panel column (008-grid-tui, US2 T028): each live panel a bordered block
/// titled with its id, stacked in insertion order and given an equal share of the column height. The
/// panel's widget is drawn richly (truecolor) into the block's inner rect via `buffer_render`, which
/// clips any overflow to the region (FR-012).
/// Smallest useful panel: top border + one content row + bottom border.
const MIN_PANEL_ROWS: u16 = 3;

/// How wide the panel column may be: 008's own sizing, narrowed by the visual level's cap (FR-011).
///
/// `min(level_cap, 008_cap)` per contracts/visual-levels.md — the level narrows the budget, it never
/// widens it. 008's own "this widget needs at least N columns" script error still fires afterward on
/// whatever budget survives; the cap changes how much room there is, not whether the check runs.
///
/// Note that 008's 50-column ceiling binds before either level cap on any terminal wide enough for a
/// two-pane layout (≥120 cols ⇒ ⌊w/3⌋ ≥ 40, and the base is clamped to 50), so `panels-wide` widens
/// nothing there today. It is not dead: it still separates the tiers on narrower surfaces, and it is
/// the tier that admits `Overlay` requests one step below takeover.
pub(crate) fn panel_column_width(cols: u16, level: crate::config::VisualLevel) -> u16 {
    let base = (cols / 3).clamp(30, 50).min(cols.saturating_sub(20));
    base.min(crate::visual_gate::panel_width_cap(level, cols))
}

/// The drawable regions this layout offers, published for the render tool's fit checks
/// (008-grid-tui). Derived with the *same* constants `view` lays out with, so the tool can never
/// accept a widget this renderer would then have to mangle.
pub fn viewport_for(app: &App) -> crate::viz::viewport::Viewport {
    let (cols, rows) = app.size;
    // Mirror `view`: header(1) + input(3) + footer(1) are chrome; the rest is the body.
    let body_rows = rows.saturating_sub(5);
    let two_pane = app.layout_mode == LayoutMode::TwoPane && !app.panels.is_empty();

    let (inline_cols, panel_w) = if two_pane {
        let pw = panel_column_width(cols, app.visual.level);
        (cols.saturating_sub(pw), pw)
    } else if app.layout_mode == LayoutMode::TwoPane {
        // No panels yet, but a column *would* be carved out as soon as one appears.
        (cols, panel_column_width(cols, app.visual.level))
    } else {
        (cols, 0)
    };

    // How many more MIN_PANEL_ROWS-sized panels the column can seat.
    let used: u16 = (app.panels.len() as u16).saturating_mul(MIN_PANEL_ROWS);
    let panel_slots_free = body_rows.saturating_sub(used) / MIN_PANEL_ROWS.max(1);

    crate::viz::viewport::Viewport {
        cols,
        rows,
        inline_cols,
        // Interior width/height of a panel, net of its border.
        panel_cols: panel_w.saturating_sub(2),
        panel_rows: MIN_PANEL_ROWS.saturating_sub(2).max(1),
        panel_slots_free,
        live_panels: app.panels.iter().map(|(id, _)| id.to_string()).collect(),
        full_screen: true,
        constrained: true,
    }
}

/// Draw the panel column, then let each panel claim the transition it owes (009 T018/T019).
///
/// Registration happens here rather than at upsert time because a transition needs the panel's
/// `Rect`, which only exists after this function has laid the column out. The outgoing snapshot for
/// the *next* update is taken from this frame's buffer on the way out.
fn render_panels(app: &mut App, frame: &mut Frame<'_>, area: Rect) {
    // A zero-width column is what `visual_level = none` produces (its cap is 0). Panels should never
    // reach the TUI at that level — the gate routes them inline — but drawing into no space is a
    // no-op either way, so this stays a guard rather than an assertion.
    if app.panels.is_empty() || area.height == 0 || area.width == 0 {
        return;
    }
    // Size each panel to its *content* (border + natural widget height) rather than splitting the
    // column evenly. An even split starved every panel once a few accumulated — 13 panels in 28 rows
    // left each with 3 rows showing a title and nothing else. Greedy top-down allocation keeps early
    // panels legible and reports the overflow honestly instead of silently squeezing everything.
    let inner_w = area.width.saturating_sub(2).max(1);
    let mut y = area.y;
    let mut shown = 0usize;
    // Where each panel landed, so the transition pass below can register against the real Rect
    // without re-deriving the greedy layout.
    let mut placed: Vec<(String, PanelArea)> = Vec::new();

    for (id, spec) in app.panels.iter() {
        let remaining = area.bottom().saturating_sub(y);
        // Keep a row free for the "+N more" note if this isn't the last panel and space is tight.
        let more_after = app.panels.len() - shown > 1;
        let reserve = u16::from(more_after && remaining <= MIN_PANEL_ROWS + 1);
        if remaining.saturating_sub(reserve) < MIN_PANEL_ROWS {
            break;
        }
        let desired = crate::viz::buffer_render::spec_height(spec, inner_w).saturating_add(2);
        let h = desired.clamp(MIN_PANEL_ROWS, remaining - reserve);
        let rect = Rect::new(area.x, y, area.width, h);

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(dim_style())
            .title(Span::styled(
                format!(" {id} "),
                role_style(ThemeRole::Accent),
            ));
        let inner = block.inner(rect);
        frame.render_widget(block, rect);
        if inner.width > 0 && inner.height > 0 {
            crate::viz::buffer_render::render_into(spec, inner, frame.buffer_mut());
        }
        placed.push((id.to_string(), PanelArea { outer: rect, inner }));
        y += h;
        shown += 1;
    }

    // Now that every panel has a Rect: register whatever transition it owes, then snapshot the
    // content it just drew as the outgoing half of its *next* update (FR-003, FR-004).
    let visual = app.visual;
    for (id, at) in placed {
        let Some(panel) = app.panels.get_mut(&id) else {
            continue;
        };
        panels::register_transition(panel, &mut app.effects, at, visual);
        panels::snapshot_render(panel, at.inner, frame.buffer_mut());
    }

    // Tell the truth about anything that didn't fit, and point at the way to reclaim space.
    let hidden = app.panels.len() - shown;
    if hidden > 0 {
        let note_y = area.bottom().saturating_sub(1).max(area.y);
        frame.render_widget(
            Paragraph::new(Line::styled(
                format!(" +{hidden} more — remove_panel()/clear_panels() "),
                dim_style(),
            )),
            Rect::new(area.x, note_y, area.width, 1),
        );
    }
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
        ("p", "toggle panel overlay (narrow layouts)"),
        ("y", "yank newest message (OSC 52)"),
        ("? Esc", "close this help"),
        ("q  Ctrl-C", "quit"),
        ("Ctrl-Z", "suspend (fg to resume)"),
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
    fn render(app: &mut App, w: u16, h: u16) -> String {
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
        let out = render(&mut app, 80, 20);
        assert!(out.contains("bee"), "header missing:\n{out}");
        assert!(out.contains("you  hello bee"), "user line missing:\n{out}");
        assert!(out.contains("hi there"), "assistant line missing:\n{out}");
        assert!(out.contains("input"), "input border title missing:\n{out}");
        assert!(out.contains("q quit"), "footer hint missing:\n{out}");
    }

    #[test]
    fn too_small_terminal_shows_a_message_not_a_broken_layout() {
        let mut app = App::new(30, 8); // below the 40×10 floor
        let out = render(&mut app, 30, 8);
        assert!(out.contains("terminal too small"), "got:\n{out}");
    }

    #[test]
    fn help_overlay_lists_keybindings() {
        let mut app = App::new(80, 20);
        app.help_open = true;
        let out = render(&mut app, 80, 20);
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
        let out = render(&mut app, 120, 24);
        assert!(out.contains("metrics"), "panel title missing:\n{out}");
        assert!(out.contains("Latency"), "panel content missing:\n{out}");
        assert!(
            out.contains("you  show metrics"),
            "chat still present beside the panel:\n{out}"
        );
    }

    #[test]
    fn inline_widget_renders_richly_not_as_ascii_placeholder() {
        // Regression: inline widgets used to go through `to_ascii()` + dim styling, so a sprite showed
        // as the literal text "[sprite 16×16]" and a table as a gray blob. They now take the same
        // truecolor buffer path panels use.
        let mut app = App::new(80, 24);
        app.chat.push(ChatMessage::widget(RenderSpec::Sprite {
            spec: crate::viz::bee::sprite(),
        }));
        let out = render(&mut app, 80, 24);
        assert!(
            !out.contains("[sprite"),
            "sprite must rasterize, not print its ASCII placeholder:\n{out}"
        );
        assert!(
            out.contains('▀') || out.contains('▄') || out.contains('█'),
            "expected half-block sprite pixels:\n{out}"
        );
    }

    #[test]
    fn newest_content_is_visible_at_the_bottom_past_a_tall_widget() {
        // Regression: chat heights were counted pre-wrap while `Paragraph` re-wrapped at draw time,
        // so once a tall/wide widget entered the flow the bottom-anchoring drifted and the newest
        // lines fell off-screen. Heights are now the rendered heights.
        let mut app = App::new(80, 20);
        for i in 0..30 {
            app.chat.push(ChatMessage::text(
                Role::Assistant,
                format!("filler line {i}"),
            ));
        }
        app.chat.push(ChatMessage::widget(RenderSpec::BarChart {
            title: "big chart".into(),
            bars: (0..8)
                .map(|i| crate::render_spec::Bar {
                    label: format!("bar-{i}"),
                    value: i * 10,
                })
                .collect(),
            x_label: None,
            y_label: None,
            color: None,
        }));
        app.chat
            .push(ChatMessage::text(Role::Assistant, "THE NEWEST LINE"));
        let out = render(&mut app, 80, 20);
        assert!(
            out.contains("THE NEWEST LINE"),
            "newest message must be visible at the bottom:\n{out}"
        );
    }

    #[test]
    fn overflowing_panels_report_what_is_hidden() {
        // Many panels no longer get squeezed to unreadable 3-row slivers in silence: the ones that
        // fit render at content height, and the column says how many are hidden.
        let mut app = App::new(120, 24);
        for i in 0..12 {
            app.panels.upsert(
                format!("panel-{i}"),
                RenderSpec::Table {
                    title: format!("t{i}"),
                    headers: vec!["a".into()],
                    rows: vec![crate::render_spec::Row {
                        cells: vec!["1".into()],
                        color: None,
                    }],
                },
            );
        }
        let out = render(&mut app, 120, 24);
        assert!(out.contains("more"), "overflow note missing:\n{out}");
        assert!(
            out.contains("remove_panel") || out.contains("clear_panels"),
            "note should point at the way to reclaim space:\n{out}"
        );
    }

    // --- US3 (T032): responsive layouts, the overlay, and NO_COLOR --------------------------------

    #[test]
    fn narrow_terminal_hides_the_panel_column_and_the_overlay_brings_panels_back() {
        // Below 120 cols there is no room for a side-by-side column, so panels are invisible until
        // `p` opens the overlay (T036) — the gap that made panel output unreachable on a narrow term.
        let mut app = App::new(80, 24);
        app.chat.push(ChatMessage::text(Role::User, "hello"));
        app.panels.upsert(
            "metrics",
            RenderSpec::Text {
                content: "CPU 82%".into(),
                style: None,
                bold: false,
                dim: false,
            },
        );
        assert_eq!(app.layout_mode, LayoutMode::SinglePane);

        let closed = render(&mut app, 80, 24);
        assert!(
            !closed.contains("CPU 82%"),
            "panel hidden while the overlay is closed:\n{closed}"
        );

        app.panels_visible = true;
        let open = render(&mut app, 80, 24);
        assert!(
            open.contains("CPU 82%"),
            "overlay reveals the panel:\n{open}"
        );
        assert!(open.contains("panels"), "overlay is labelled:\n{open}");
    }

    #[test]
    fn the_overlay_is_single_pane_only() {
        // In TwoPane the panels already have a column, so `p` must not stack an overlay on top.
        let mut app = App::new(140, 40);
        app.panels.upsert("m", RenderSpec::Separator);
        app.panels_visible = true;
        assert_eq!(app.layout_mode, LayoutMode::TwoPane);
        assert!(!app.overlay_open(), "no overlay when a real column exists");
    }

    #[test]
    fn a_wide_but_short_terminal_stays_single_pane() {
        // 120+ cols but under 24 rows has no vertical room for panels beside chat (T034).
        assert_eq!(LayoutMode::from_size(160, 20), LayoutMode::SinglePane);
        assert_eq!(LayoutMode::from_size(160, 24), LayoutMode::TwoPane);
    }

    #[test]
    fn every_region_still_renders_without_color() {
        // NO_COLOR degrades to monochrome but must never blank a region (FR-014, SC-005). Symbols are
        // color-independent, so a matching symbol grid proves the layout survives unstyled.
        let mut app = App::new(120, 24);
        app.chat.push(ChatMessage::text(Role::User, "hello bee"));
        app.panels.upsert(
            "metrics",
            RenderSpec::Text {
                content: "CPU 82%".into(),
                style: None,
                bold: false,
                dim: false,
            },
        );
        let out = render(&mut app, 120, 24);
        for expected in [
            "bee",
            "you  hello bee",
            "metrics",
            "CPU 82%",
            "input",
            "q quit",
        ] {
            assert!(out.contains(expected), "{expected:?} missing:\n{out}");
        }
    }

    #[test]
    fn the_footer_advertises_yank_always_and_p_only_when_the_overlay_is_the_way_in() {
        // Wide + no panels: no reason to mention `p`.
        let mut wide = App::new(120, 24);
        let out = render(&mut wide, 120, 24);
        assert!(out.contains("y yank"), "yank hint missing:\n{out}");
        assert!(!out.contains("p panels"), "no overlay to advertise:\n{out}");

        // Narrow + panels: the overlay is the only way to see them, so `p` is advertised.
        let mut narrow = App::new(80, 24);
        narrow.panels.upsert("m", RenderSpec::Separator);
        let out = render(&mut narrow, 80, 24);
        assert!(out.contains("p panels"), "overlay hint missing:\n{out}");
    }

    #[test]
    fn the_footer_never_truncates_mid_hint_on_a_narrow_terminal() {
        // It sheds low-priority hints rather than getting cut off; quit must always survive.
        let mut app = App::new(50, 20);
        app.panels.upsert("m", RenderSpec::Separator);
        let out = render(&mut app, 50, 20);
        let footer = out.lines().last().unwrap_or_default();
        assert!(
            footer.contains("q quit"),
            "quit hint must survive:\n{footer}"
        );
        assert!(
            footer.chars().filter(|c| !c.is_whitespace()).count() > 0,
            "footer is not blank"
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
        let empty = render(&mut base(), 120, 24);
        assert!(
            !empty.contains("metrics"),
            "no panel title when empty:\n{empty}"
        );

        let mut with = base();
        with.panels.upsert("metrics", RenderSpec::Separator);
        let populated = render(&mut with, 120, 24);
        assert!(
            populated.contains("metrics"),
            "panel appears once populated:\n{populated}"
        );
        assert_ne!(empty, populated, "the panel column changes the layout");
    }

    // --- 009 US1 (T018/T020): the render path is what puts effects on screen ---------------------

    #[test]
    fn drawing_a_new_panel_registers_its_entrance_and_snapshots_what_it_drew() {
        // The whole US1 loop through the real renderer: layout assigns the panel a Rect, the
        // transition is registered against it, and the drawn content is kept as the outgoing half of
        // the next update.
        let mut app = App::new(120, 24);
        app.panels.upsert("metrics", RenderSpec::Separator);
        assert!(!app.effects.is_running(), "nothing animates before a frame");

        render(&mut app, 120, 24);
        assert!(app.effects.is_running(), "the panel entered");
        let panel = app.panels.get_mut("metrics").expect("panel");
        assert!(panel.pending.is_none(), "the transition was consumed");
        assert!(
            panel.fx.prev.is_some(),
            "the drawn content is the next update's outgoing half"
        );
    }

    // --- 009 US2 (T030/T034/T035): the level bounds what the agent gets ------------------------

    fn with_level(level: crate::config::VisualLevel) -> crate::config::VisualConfig {
        crate::config::VisualConfig {
            level,
            ..crate::config::VisualConfig::default()
        }
    }

    #[test]
    fn the_panel_column_never_exceeds_the_levels_share_of_the_screen() {
        // SC-005, measured on the real Rect the layout hands the column. `panels` is a third,
        // `panels-wide` a half — and 008's own sizing still narrows it further where it is stricter.
        for cols in [120u16, 160, 200] {
            let third = cols / 3;
            let half = cols / 2;
            assert!(
                panel_column_width(cols, crate::config::VisualLevel::Panels) <= third,
                "panels exceeded a third at {cols} cols"
            );
            assert!(
                panel_column_width(cols, crate::config::VisualLevel::PanelsWide) <= half,
                "panels-wide exceeded a half at {cols} cols"
            );
        }
    }

    #[test]
    fn a_wider_level_is_never_narrower_than_a_stricter_one() {
        // The tiers are a ceiling, so they must be monotonic. On a two-pane terminal 008's own
        // 50-column ceiling binds before either level cap, which is why these are `<=` and why
        // `panels-wide` widens nothing at these sizes — it separates the tiers on narrower
        // surfaces, and it is the tier one step below takeover.
        for cols in [120u16, 160, 200] {
            let strict = panel_column_width(cols, crate::config::VisualLevel::Panels);
            let wide = panel_column_width(cols, crate::config::VisualLevel::PanelsWide);
            assert!(strict <= wide, "tiers inverted at {cols} cols");
        }
        assert_eq!(
            panel_column_width(80, crate::config::VisualLevel::Panels),
            26,
            "below 008's clamp the level cap is what binds"
        );
        assert_eq!(
            panel_column_width(80, crate::config::VisualLevel::PanelsWide),
            30
        );
    }

    #[test]
    fn level_none_leaves_no_column_for_the_agent_at_all() {
        // FR-010: the gate routes panels inline before they ever reach the TUI, and even if one
        // arrived anyway there is no width for it to occupy.
        assert_eq!(panel_column_width(120, crate::config::VisualLevel::None), 0);
        let mut app = App::new(120, 24).with_visual(with_level(crate::config::VisualLevel::None));
        app.panels.upsert("metrics", RenderSpec::Separator);
        let out = render(&mut app, 120, 24);
        assert!(
            !out.contains("metrics"),
            "no panel column at level none:\n{out}"
        );
        assert_eq!(
            viewport_for(&app).panel_cols,
            0,
            "and the render tool is told so"
        );
    }

    #[test]
    fn level_none_strips_agent_effects_while_chrome_keeps_moving() {
        // FR-006d / FR-024 / T031: the level governs the agent. Motion is a separate axis, so
        // bee's own chrome is untouched by it.
        let visual = with_level(crate::config::VisualLevel::None);
        let area = Rect::new(0, 0, 20, 5);
        let spec = crate::tui::effects::panel_enter_spec();
        assert!(
            crate::tui::effects::resolve(
                &spec,
                &crate::tui::effects::ResolveCtx::agent(visual, area)
            )
            .is_none(),
            "an agent effect is stripped entirely"
        );
        assert!(
            crate::tui::effects::resolve(
                &spec,
                &crate::tui::effects::ResolveCtx::chrome(visual, area)
            )
            .is_some(),
            "chrome is bee's own UI, not the agent's"
        );
    }

    #[test]
    fn the_published_viewport_reflects_the_level_so_the_tool_sizes_to_it() {
        // The render tool checks widgets against `panel_cols`; if that were the un-capped width the
        // tool would accept a widget the renderer then had to mangle.
        let mut app = App::new(120, 24).with_visual(with_level(crate::config::VisualLevel::Panels));
        app.panels.upsert("m", RenderSpec::Separator);
        let vp = viewport_for(&app);
        assert!(vp.panel_cols > 0 && vp.panel_cols <= 120 / 3);
        assert_eq!(vp.inline_cols, 120 - (vp.panel_cols + 2));
    }

    #[test]
    fn a_motionless_session_draws_panels_with_no_effect_at_all() {
        // FR-006c end to end: same frame, same panel, nothing registered — so the loop has nothing
        // to advance and the operator sees final content immediately.
        let mut app = App::new(120, 24).with_visual(crate::config::VisualConfig {
            animations: false,
            ..crate::config::VisualConfig::default()
        });
        app.panels.upsert("metrics", RenderSpec::Separator);
        let out = render(&mut app, 120, 24);
        assert!(!app.effects.is_running(), "no motion may be registered");
        assert!(out.contains("metrics"), "the panel is on screen regardless");
    }
}
