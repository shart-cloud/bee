//! The visual test cases.
//!
//! Each scene pairs a *base drawing* (ratatui widgets, drawn fresh every frame) with *one effect*
//! (a tachyonfx constructor). Together the fifteen cover all twelve `EffectSpec` variants bee's
//! agents can request, plus the three compositions bee builds itself: the panel cross-fade
//! (`tui::effects::panel_update`) and the two harness-chrome presets (`Chrome::Header`,
//! `Chrome::Footer`).
//!
//! **The effect constructors below are copied from `src/tui/effects.rs`, argument for
//! argument** — same house constants, same durations, same `Motion` mapping, same two composites.
//! That copying is the entire point: this crate cannot import the bee application package (its tree is tokio,
//! rig-core, aya, rustyline and rhai, none of which build for wasm32), so the guarantee on offer is
//! not "bee's code is exercised" but "the *upstream API surface* bee's code stands on is exercised,
//! with bee's arguments". If ratatui or tachyonfx changes what these calls draw, the screenshots
//! move — which is the signal worth having.
//!
//! Adding a scene: append a `Scene` to [`SCENES`]. Nothing else needs touching — the snapshot driver
//! reads the manifest off the page rather than carrying its own list.

use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Position, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{
    Bar, BarChart, BarGroup, Block, Cell, Gauge, Paragraph, Row, Table, Widget,
};
use tachyonfx::{blit_buffer_region, fx, Effect, Motion, SimpleRng};

use crate::mascot;
use crate::palette;

// --- Determinism ------------------------------------------------------------------------------

/// The seed every cell-scattering effect is pinned to.
///
/// **This is the one place these scenes deliberately diverge from the bee application package.** The scatter
/// effects — `dissolve`, `coalesce`, `slide_in`, `slide_out` — build their per-cell thresholds from
/// `SimpleRng::default()`, and under tachyonfx's `std`/`wasm` features that constructor seeds itself
/// from `SystemTime::now()`. They are therefore *not* reproducible run to run, and a baseline of an
/// unseeded dissolve would fail against itself.
///
/// `Effect::with_rng` pins them. It has to be applied to each **leaf**: `fx::sequence` and
/// `fx::parallel` inherit the `Shader` trait's no-op `set_rng`, so seeding a composite silently does
/// nothing and only the leaves inside it count. That is why [`panel_update`] seeds its inner
/// `dissolve` and `coalesce` rather than the sequence wrapping them.
///
/// The value is arbitrary — any fixed seed samples one of the patterns bee produces at random. What
/// matters is that it is fixed. `fx::sweep_in`/`sweep_out` need no help: those already construct
/// `SimpleRng::new(0)` internally.
const SEED: u32 = 0x9E37_79B9;

/// Pin `effect` to [`SEED`]. A no-op on effects that use no randomness.
fn seeded(effect: Effect) -> Effect {
    effect.with_rng(SimpleRng::new(SEED))
}

// --- The house constants, from `src/tui/effects.rs` -------------------------------

/// How wide the leading gradient is on slide/sweep effects, in cells.
const GRADIENT_LEN: u16 = 8;
/// Per-cell timing jitter on slide/sweep, so the edge does not read like a ruler.
const RANDOMNESS: u16 = 15;
/// How far a glow lightens before returning.
const GLOW_AMOUNT: f32 = 0.4;

/// Default entrance transition for a newly created panel.
const PANEL_ENTER_MS: u32 = 300;
/// Default transition when a panel's content is replaced.
const PANEL_UPDATE_MS: u32 = 400;
/// Slide and sweep budget.
const TRAVEL_MS: u32 = 400;
/// A tool-call line acknowledging its result.
const PULSE_MS: u32 = 200;
/// The glow's full there-and-back.
const GLOW_MS: u32 = 500;
/// The mascot materializing.
const MASCOT_MS: u32 = 600;
/// The header's session-start fade, and the footer's slide.
const CHROME_MS: u32 = 300;

/// One visual test case.
pub struct Scene {
    /// Stable identifier. Also the baseline filename stem, so renaming a scene orphans its baseline
    /// rather than silently comparing against the wrong picture.
    pub name: &'static str,
    /// What the scene is for, shown under the gallery.
    pub about: &'static str,
    /// The effect's full duration. Drives the gallery's pacing.
    pub ms: u32,
    /// The offsets to screenshot, in ms.
    ///
    /// Usually `[0, ms/2, ms]`: start, halfway, end. Three points, because two would only prove the
    /// effect started and finished — the midpoint is the one that catches a changed interpolation
    /// curve. Declared per scene rather than derived because "halfway" is not always the
    /// interesting frame: a two-phase composite is *empty* at its midpoint by construction, and a
    /// blank baseline is one almost any bug would also produce.
    pub times: &'static [u32],
    pub cols: u16,
    pub rows: u16,
    /// Paints the widgets. Called fresh on **every** frame, exactly as a real render loop does —
    /// an effect like `coalesce` has nothing to reform if the base is painted only once.
    pub draw: fn(&mut Buffer, Rect),
    /// Builds the effect over the scene's area.
    pub effect: fn(Rect) -> Effect,
}

impl Scene {
    pub fn area(&self) -> Rect {
        Rect::new(0, 0, self.cols, self.rows)
    }
}

pub fn find(name: &str) -> Option<&'static Scene> {
    SCENES.iter().find(|s| s.name == name)
}

pub static SCENES: &[Scene] = &[
    Scene {
        name: "fade_in",
        about: "a panel arriving — fx::fade_from over a bordered paragraph",
        ms: PANEL_ENTER_MS,
        times: &[0, PANEL_ENTER_MS / 2, PANEL_ENTER_MS],
        cols: 44,
        rows: 7,
        draw: draw_panel,
        effect: |area| fx::fade_from(palette::INFO, palette::INFO, PANEL_ENTER_MS).with_area(area),
    },
    Scene {
        name: "fade_out",
        about: "the same panel leaving — fx::fade_to",
        ms: PANEL_ENTER_MS,
        times: &[0, PANEL_ENTER_MS / 2, PANEL_ENTER_MS],
        cols: 44,
        rows: 7,
        draw: draw_panel,
        effect: |area| fx::fade_to(palette::INFO, palette::INFO, PANEL_ENTER_MS).with_area(area),
    },
    Scene {
        // bee's naming inverts against tachyonfx's: "in" means the content arrives, which is
        // `coalesce` (characters reforming), not `dissolve`.
        name: "dissolve_in",
        about: "a table reforming out of noise — fx::coalesce",
        ms: PANEL_ENTER_MS,
        times: &[0, PANEL_ENTER_MS / 2, PANEL_ENTER_MS],
        cols: 44,
        rows: 7,
        draw: draw_table,
        effect: |area| seeded(fx::coalesce(PANEL_ENTER_MS)).with_area(area),
    },
    Scene {
        name: "dissolve_out",
        about: "the same table scattering — fx::dissolve",
        ms: PANEL_ENTER_MS,
        times: &[0, PANEL_ENTER_MS / 2, PANEL_ENTER_MS],
        cols: 44,
        rows: 7,
        draw: draw_table,
        effect: |area| seeded(fx::dissolve(PANEL_ENTER_MS)).with_area(area),
    },
    Scene {
        name: "slide_in_left",
        about: "a bar chart arriving from the left — fx::slide_in(LeftToRight)",
        ms: TRAVEL_MS,
        times: &[0, TRAVEL_MS / 2, TRAVEL_MS],
        cols: 44,
        rows: 12,
        draw: draw_bar_chart,
        effect: |area| {
            seeded(fx::slide_in(
                Motion::LeftToRight,
                GRADIENT_LEN,
                RANDOMNESS,
                palette::INFO,
                TRAVEL_MS,
            ))
            .with_area(area)
        },
    },
    Scene {
        name: "slide_out_right",
        about: "the same bar chart leaving rightward — fx::slide_out(RightToLeft)",
        ms: TRAVEL_MS,
        times: &[0, TRAVEL_MS / 2, TRAVEL_MS],
        cols: 44,
        rows: 12,
        draw: draw_bar_chart,
        effect: |area| {
            seeded(fx::slide_out(
                Motion::RightToLeft,
                GRADIENT_LEN,
                RANDOMNESS,
                palette::INFO,
                TRAVEL_MS,
            ))
            .with_area(area)
        },
    },
    Scene {
        name: "sweep_in_top",
        about: "a gauge revealed by a wave from the top — fx::sweep_in(UpToDown)",
        ms: TRAVEL_MS,
        times: &[0, TRAVEL_MS / 2, TRAVEL_MS],
        cols: 44,
        rows: 3,
        draw: draw_gauge,
        effect: |area| {
            seeded(fx::sweep_in(
                Motion::UpToDown,
                GRADIENT_LEN,
                RANDOMNESS,
                palette::INFO,
                TRAVEL_MS,
            ))
            .with_area(area)
        },
    },
    Scene {
        name: "sweep_out_bottom",
        about: "the same gauge hidden by a wave from the bottom — fx::sweep_out(DownToUp)",
        ms: TRAVEL_MS,
        times: &[0, TRAVEL_MS / 2, TRAVEL_MS],
        cols: 44,
        rows: 3,
        draw: draw_gauge,
        effect: |area| {
            seeded(fx::sweep_out(
                Motion::DownToUp,
                GRADIENT_LEN,
                RANDOMNESS,
                palette::INFO,
                TRAVEL_MS,
            ))
            .with_area(area)
        },
    },
    Scene {
        // Composite: tachyonfx has no `pulse`. Flash toward the color, then back, half the budget
        // each way — exactly what `EffectSpec::Pulse` resolves to.
        name: "pulse_accent",
        about: "a status line acknowledging a tool result — fade_to_fg + fade_from_fg",
        ms: PULSE_MS,
        times: &[0, PULSE_MS / 2, PULSE_MS],
        cols: 44,
        rows: 1,
        draw: draw_status,
        effect: |area| {
            let half = PULSE_MS / 2;
            fx::sequence(&[
                fx::fade_to_fg(palette::ACCENT, half),
                fx::fade_from_fg(palette::ACCENT, half),
            ])
            .with_area(area)
        },
    },
    Scene {
        // The other composite: "breathing" is a ping-ponged lighten.
        name: "glow",
        about: "a panel breathing — fx::ping_pong(fx::lighten)",
        ms: GLOW_MS,
        times: &[0, GLOW_MS / 2, GLOW_MS],
        cols: 44,
        rows: 7,
        draw: draw_block,
        effect: |area| {
            fx::ping_pong(fx::lighten(
                Some(GLOW_AMOUNT),
                Some(GLOW_AMOUNT),
                GLOW_MS / 2,
            ))
            .with_area(area)
        },
    },
    Scene {
        name: "evolve_in",
        about: "the mascot materializing out of block glyphs — fx::evolve_into(BlocksHorizontal)",
        ms: MASCOT_MS,
        times: &[0, MASCOT_MS / 2, MASCOT_MS],
        cols: mascot::COLS,
        rows: mascot::ROWS,
        draw: |buf, area| mascot::draw(buf, area),
        effect: |area| {
            fx::evolve_into(fx::EvolveSymbolSet::BlocksHorizontal, MASCOT_MS).with_area(area)
        },
    },
    Scene {
        name: "evolve_out",
        about: "the mascot devolving back to block glyphs — fx::evolve_from(BlocksHorizontal)",
        ms: MASCOT_MS,
        times: &[0, MASCOT_MS / 2, MASCOT_MS],
        cols: mascot::COLS,
        rows: mascot::ROWS,
        draw: |buf, area| mascot::draw(buf, area),
        effect: |area| {
            fx::evolve_from(fx::EvolveSymbolSet::BlocksHorizontal, MASCOT_MS).with_area(area)
        },
    },
    Scene {
        name: "panel_update",
        about: "a panel's content replaced — the old dissolves out, the new coalesces in",
        ms: PANEL_UPDATE_MS,
        // Four points, at the quarters — the same offsets `tui::effects`'s own test uses, and for
        // the same reason. This transition is two sequential halves, so at the exact midpoint the
        // old content has finished dissolving and the new has not begun coalescing: the frame is
        // blank by construction, which is also what a broken transition would produce. The quarter
        // and three-quarter frames are the ones that carry the information — old content
        // part-dissolved, then new content part-formed.
        times: &[
            0,
            PANEL_UPDATE_MS / 4,
            PANEL_UPDATE_MS * 3 / 4,
            PANEL_UPDATE_MS,
        ],
        cols: 44,
        rows: 7,
        // The base painter is the *new* content throughout, matching the live pipeline: the
        // outgoing content is a snapshot the effect carries, not something the widget redraws.
        draw: |buf, area| draw_fill_panel(buf, area, 'N', palette::SUCCESS),
        effect: panel_update,
    },
    Scene {
        name: "chrome_header",
        about: "bee's own header fading up at session start — Chrome::Header",
        ms: CHROME_MS,
        times: &[0, CHROME_MS / 2, CHROME_MS],
        cols: 64,
        rows: 1,
        draw: draw_header,
        effect: |area| fx::fade_from(palette::INFO, palette::INFO, CHROME_MS).with_area(area),
    },
    Scene {
        name: "chrome_footer",
        about: "bee's own footer sliding up beneath it — Chrome::Footer",
        ms: CHROME_MS,
        times: &[0, CHROME_MS / 2, CHROME_MS],
        cols: 64,
        rows: 1,
        draw: draw_footer,
        effect: |area| {
            seeded(fx::slide_in(
                Motion::DownToUp,
                GRADIENT_LEN,
                RANDOMNESS,
                palette::INFO,
                CHROME_MS,
            ))
            .with_area(area)
        },
    },
];

// --- Composites -------------------------------------------------------------------------------

/// bee's panel-replacement transition (`tui::effects::panel_update`).
///
/// Realized as a sequence over one budget rather than two literally-simultaneous shaders: tachyonfx
/// effects transform cells already in the buffer, so blending two *sources* per cell is not
/// something the stock primitives express. The first half blits the outgoing snapshot back over the
/// freshly drawn content and dissolves it away; the second half coalesces the new content that was
/// underneath all along.
fn panel_update(area: Rect) -> Effect {
    let mut prev = Buffer::empty(area);
    draw_fill_panel(&mut prev, area, 'O', palette::DIM);
    let half = (PANEL_UPDATE_MS / 2).max(1);
    fx::sequence(&[
        fx::parallel(&[blit(prev, area, half), seeded(fx::dissolve(half))]),
        seeded(fx::coalesce(half)),
    ])
    .with_area(area)
}

/// An effect that redraws `src` over `area` on every tick for `ms`, changing nothing else. The floor
/// of the update transition: the dissolve running beside it needs the *outgoing* content in the
/// buffer to eat away at, and the widget has already overwritten it with the new one.
fn blit(src: Buffer, area: Rect, ms: u32) -> Effect {
    fx::effect_fn_buf(src, ms, move |src, _ctx, buf| {
        blit_buffer_region(
            src,
            src.area,
            buf,
            ratatui::layout::Offset {
                x: i32::from(area.x),
                y: i32::from(area.y),
            },
        );
    })
}

// --- Base drawings ----------------------------------------------------------------------------

fn dim(s: &'static str) -> Span<'static> {
    Span::styled(s, Style::default().fg(palette::DIM))
}

fn text(s: &'static str) -> Span<'static> {
    Span::styled(s, Style::default().fg(palette::TEXT))
}

/// A bordered paragraph of styled key/value lines — the shape most bee panels take.
fn draw_panel(buf: &mut Buffer, area: Rect) {
    let body = Text::from(vec![
        Line::from(vec![dim("scope    "), text("read /workspace")]),
        Line::from(vec![
            dim("verdict  "),
            Span::styled("allowed", Style::default().fg(palette::SUCCESS)),
        ]),
        Line::from(vec![
            dim("denied   "),
            Span::styled("3 syscalls", Style::default().fg(palette::ERROR)),
        ]),
        Line::from(vec![dim("elapsed  "), text("412ms")]),
    ]);
    Paragraph::new(body)
        .block(titled("permissions"))
        .render(area, buf);
}

/// A bordered block with a filled surface and nothing in it — the glow scene wants the *frame* and
/// the fill to breathe, with no text competing for attention.
fn draw_block(buf: &mut Buffer, area: Rect) {
    titled("episode")
        .style(Style::default().bg(palette::SURFACE0))
        .render(area, buf);
}

/// A 4-row table under a bold header.
fn draw_table(buf: &mut Buffer, area: Rect) {
    let widths = [
        Constraint::Percentage(40),
        Constraint::Percentage(30),
        Constraint::Percentage(30),
    ];
    let header = Row::new(["tool", "calls", "verdict"].map(Cell::from))
        .style(Style::default().fg(palette::TEXT).add_modifier(Modifier::BOLD));
    let rows = [
        (["read_file", "12", "allow"], palette::SUCCESS),
        (["write_file", "3", "allow"], palette::SUCCESS),
        (["exec", "1", "deny"], palette::ERROR),
        (["net", "0", "deny"], palette::DIM),
    ]
    .map(|(cells, color)| Row::new(cells.map(Cell::from)).style(Style::default().fg(color)));
    Table::new(rows, widths)
        .header(header)
        .block(titled("tool budget"))
        .render(area, buf);
}

/// Five labelled bars.
fn draw_bar_chart(buf: &mut Buffer, area: Rect) {
    let bars: Vec<Bar> = [("mon", 42u64), ("tue", 68), ("wed", 21), ("thu", 87), ("fri", 55)]
        .iter()
        .map(|(label, value)| Bar::default().value(*value).label(Line::from(*label)))
        .collect();
    BarChart::default()
        .block(titled("denials / day"))
        .data(BarGroup::default().bars(&bars))
        .bar_width(6)
        .bar_gap(1)
        .bar_style(Style::default().fg(palette::INFO))
        .render(area, buf);
}

/// A gauge at 67%.
fn draw_gauge(buf: &mut Buffer, area: Rect) {
    Gauge::default()
        .block(titled("token budget"))
        .ratio(0.67)
        .label("67%")
        .gauge_style(Style::default().fg(palette::ACCENT).bg(palette::SURFACE0))
        .render(area, buf);
}

/// A single status line — a tool call and the result that just landed.
fn draw_status(buf: &mut Buffer, area: Rect) {
    Paragraph::new(Line::from(vec![
        Span::styled("✓ ", Style::default().fg(palette::SUCCESS)),
        text("read_file"),
        dim("  /workspace/src/main.rs  "),
        Span::styled("2.1 kB", Style::default().fg(palette::ACCENT)),
    ]))
    .render(area, buf);
}

/// bee's header bar.
fn draw_header(buf: &mut Buffer, area: Rect) {
    Paragraph::new(Line::from(vec![
        Span::styled(
            " bee ",
            Style::default()
                .fg(palette::BASE)
                .bg(palette::INFO)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" episode 0c4f ", Style::default().fg(palette::TEXT)),
        dim("· scope: workspace-rw · model: claude-opus-4-8"),
    ]))
    .style(Style::default().bg(palette::SURFACE0))
    .render(area, buf);
}

/// bee's footer hint bar.
fn draw_footer(buf: &mut Buffer, area: Rect) {
    Paragraph::new(Line::from(vec![
        dim(" ^C "),
        text("quit"),
        dim("   ^L "),
        text("clear"),
        dim("   tab "),
        text("panels"),
        dim("   ? "),
        text("help"),
    ]))
    .style(Style::default().bg(palette::SURFACE0))
    .render(area, buf);
}

/// A bordered panel whose body is a solid field of one glyph. The `panel_update` scene uses two of
/// these — 'O' for the outgoing content and 'N' for the incoming — so a frame midway through the
/// cross-fade shows at a glance which cells have handed over and which have not.
fn draw_fill_panel(buf: &mut Buffer, area: Rect, glyph: char, color: ratatui::style::Color) {
    let block = titled("metrics");
    let inner = block.inner(area);
    block.render(area, buf);
    let mut s = String::new();
    s.push(glyph);
    for y in inner.y..inner.bottom() {
        for x in inner.x..inner.right() {
            let pos = Position::new(x, y);
            if buf.area.contains(pos) {
                buf[pos]
                    .set_symbol(&s)
                    .set_style(Style::default().fg(color));
            }
        }
    }
}

fn titled(title: &'static str) -> Block<'static> {
    Block::bordered()
        .title(title)
        .border_style(Style::default().fg(palette::DIM))
        .title_style(Style::default().fg(palette::INFO))
}
