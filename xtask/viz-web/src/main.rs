//! bee's visual scenes, in a browser.
//!
//! Two modes, chosen by the query string:
//!
//! * **gallery** (no query string) — every scene plays in turn, in real time, captioned. This is
//!   what `cargo xtask viz-serve` opens; it exists to be looked at.
//! * **snapshot** (`?mode=snapshot`) — nothing renders on load. The page publishes a manifest of
//!   scenes and time points, then waits for the driver to call [`bee_render`] once per screenshot.
//!
//! # Why snapshot mode renders on demand rather than on navigation
//!
//! Two reasons, both about the pixels being trustworthy.
//!
//! *Determinism.* Effects here are advanced by a **fixed 16ms step**, never by wall-clock time —
//! the same stepping `bee-harness`'s `tui::effects::Timeline` test helper uses, including the
//! zero-length first frame that makes a `t=0` capture show the *start* of the animation rather than
//! the untouched widget. A real-time animation screenshotted "about 150ms in" would not reproduce.
//! Fixed stepping is necessary but not sufficient: the cell-scattering effects also have to be
//! seeded, because tachyonfx's default RNG is wall-clock seeded. See `scenes::SEED`.
//!
//! *Font measurement.* `DomBackend` measures the page's monospace cell with
//! `getBoundingClientRect()` when it is constructed. The bundled webfont loads asynchronously, so a
//! page that rendered during load could measure the fallback font and build a differently-sized
//! grid. Rendering on demand lets the driver await `document.fonts.ready` first, once.

mod mascot;
mod palette;
mod scenes;

use std::cell::RefCell;
use std::rc::Rc;

use ratatui::buffer::Buffer;
use ratatui::layout::Position;
use ratatui::Terminal;
use ratzilla::web_sys::{self, Element};
use ratzilla::{DomBackend, WebRenderer};
use tachyonfx::{Duration, Effect};
use wasm_bindgen::prelude::*;

use scenes::Scene;

/// One 60fps frame. The step [`render_at`] advances effects by, matching
/// `bee-harness`'s `tui::effects::Timeline::frames`.
const FRAME_MS: u32 = 16;

/// How long the gallery holds a finished scene before moving to the next one.
const GALLERY_HOLD_MS: u32 = 900;

fn main() {
    if let Err(err) = run() {
        web_sys::console::error_1(&err);
    }
}

fn run() -> Result<(), JsValue> {
    let body = body()?;
    body.set_attribute("data-bee-manifest", &manifest())?;

    if search().contains("mode=snapshot") {
        // Publish nothing but the manifest and wait. `bee_render` does the rest.
        body.set_attribute("data-bee-mode", "snapshot")?;
    } else {
        body.set_attribute("data-bee-mode", "gallery")?;
        gallery()?;
    }
    body.set_attribute("data-bee-loaded", "1")?;
    Ok(())
}

/// Render `scene` as it looks `t_ms` into its effect, and flush it to the DOM.
///
/// Callable from JavaScript as `window.wasmBindings.bee_render(scene, t_ms)`. On return the page
/// carries `data-bee-cols` / `data-bee-rows` (the driver's screenshot clip) and `data-bee-ready`
/// (the scene and offset actually drawn, so the driver can confirm it screenshotted what it asked
/// for rather than a stale frame).
#[wasm_bindgen]
pub fn bee_render(scene: &str, t_ms: u32) -> Result<(), JsValue> {
    let scene = scenes::find(scene)
        .ok_or_else(|| JsValue::from_str(&format!("unknown scene {scene:?}")))?;
    let area = scene.area();
    let frame = render_at(scene, t_ms);

    // A fresh backend per render. `DomBackend::draw` clears and repopulates the grid when it finds
    // one already in the document, so successive calls replace rather than accumulate.
    let backend = DomBackend::new().map_err(js_err)?;
    let mut terminal = Terminal::new(backend).map_err(js_err)?;
    terminal
        .draw(|f| blit(&frame, f.buffer_mut(), 0, 0))
        .map_err(js_err)?;

    let body = body()?;
    body.set_attribute("data-bee-cols", &area.width.to_string())?;
    body.set_attribute("data-bee-rows", &area.height.to_string())?;
    body.set_attribute("data-bee-ready", &format!("{}@{}", scene.name, t_ms))?;
    Ok(())
}

/// Replay a scene from its start to `t_ms` in fixed steps, returning the frame at that offset.
///
/// The base drawing is repainted into a fresh buffer **every step**, then the effect is applied on
/// top — which is what a real render loop does, and what makes the reforming effects (`coalesce`,
/// `evolve_into`) meaningful. A harness that advanced the effect over one static buffer would be
/// testing something the renderer never does.
fn render_at(scene: &Scene, t_ms: u32) -> Buffer {
    let area = scene.area();
    let mut effect: Effect = (scene.effect)(area);
    let mut buf = Buffer::empty(area);
    let mut elapsed = 0u32;
    loop {
        buf.reset();
        (scene.draw)(&mut buf, area);
        // The first frame advances by zero — the same delta the first frame of a real session gets,
        // and the reason a `t=0` capture is the animation's opening frame and not the bare widget.
        let dt = if elapsed == 0 { 0 } else { FRAME_MS };
        effect.process(Duration::from_millis(dt), &mut buf, area);
        if elapsed >= t_ms {
            return buf;
        }
        elapsed += FRAME_MS;
    }
}

/// The gallery: every scene in turn, advanced by real elapsed time, captioned underneath.
fn gallery() -> Result<(), JsValue> {
    struct State {
        idx: usize,
        effect: Effect,
        elapsed: u32,
        /// Set on the first frame of a scene so it advances by zero, matching [`render_at`].
        fresh: bool,
        last_ms: f64,
    }

    let first = &scenes::SCENES[0];
    let state = Rc::new(RefCell::new(State {
        idx: 0,
        effect: (first.effect)(first.area()),
        elapsed: 0,
        fresh: true,
        last_ms: now_ms(),
    }));
    caption(first, 0);

    let backend = DomBackend::new().map_err(js_err)?;
    let terminal = Terminal::new(backend).map_err(js_err)?;
    terminal.draw_web(move |f| {
        let mut s = state.borrow_mut();
        let now = now_ms();
        // Clamped: a backgrounded tab resumes with an enormous delta, which would skip a whole
        // scene in one frame.
        let dt = if s.fresh {
            s.fresh = false;
            0
        } else {
            (now - s.last_ms).clamp(0.0, 100.0) as u32
        };
        s.last_ms = now;
        s.elapsed += dt;

        let scene = &scenes::SCENES[s.idx];
        let area = scene.area();
        let mut buf = Buffer::empty(area);
        (scene.draw)(&mut buf, area);
        s.effect.process(Duration::from_millis(dt), &mut buf, area);
        blit(&buf, f.buffer_mut(), 2, 1);

        if s.elapsed >= scene.ms + GALLERY_HOLD_MS {
            s.idx = (s.idx + 1) % scenes::SCENES.len();
            let next = &scenes::SCENES[s.idx];
            s.effect = (next.effect)(next.area());
            s.elapsed = 0;
            s.fresh = true;
            caption(next, s.idx);
        }
    });
    Ok(())
}

/// Copy `src` into `dst` at `(ox, oy)`, skipping anything that would fall outside `dst`.
fn blit(src: &Buffer, dst: &mut Buffer, ox: u16, oy: u16) {
    for y in src.area.y..src.area.bottom() {
        for x in src.area.x..src.area.right() {
            let to = Position::new(x + ox, y + oy);
            if dst.area.contains(to) {
                dst[to] = src[Position::new(x, y)].clone();
            }
        }
    }
}

/// The scene list the snapshot driver iterates, as JSON on `data-bee-manifest`.
///
/// Published by the page rather than duplicated in the driver so there is exactly one list: adding a
/// [`Scene`] is the whole of adding a scene.
fn manifest() -> String {
    let mut out = String::from("[");
    for (i, s) in scenes::SCENES.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        let ts: Vec<String> = s.times.iter().map(|t| t.to_string()).collect();
        out.push_str(&format!(
            r#"{{"name":"{}","about":{},"ms":{},"cols":{},"rows":{},"times":[{}]}}"#,
            s.name,
            json_string(s.about),
            s.ms,
            s.cols,
            s.rows,
            ts.join(",")
        ));
    }
    out.push(']');
    out
}

/// Minimal JSON string escaping — enough for the ASCII prose in [`scenes::SCENES`], and cheaper
/// than a serde dependency in a crate whose whole job is to stay small.
fn json_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn caption(scene: &Scene, idx: usize) {
    if let Ok(Some(el)) = document().map(|d| d.get_element_by_id("caption")) {
        el.set_text_content(Some(&format!(
            "{}/{}  {}  —  {}  ({}ms)",
            idx + 1,
            scenes::SCENES.len(),
            scene.name,
            scene.about,
            scene.ms
        )));
    }
}

fn now_ms() -> f64 {
    web_sys::window()
        .and_then(|w| w.performance())
        .map(|p| p.now())
        .unwrap_or(0.0)
}

fn search() -> String {
    web_sys::window()
        .map(|w| w.location())
        .and_then(|l| l.search().ok())
        .unwrap_or_default()
}

fn document() -> Result<web_sys::Document, JsValue> {
    web_sys::window()
        .and_then(|w| w.document())
        .ok_or_else(|| JsValue::from_str("no document"))
}

fn body() -> Result<Element, JsValue> {
    document()?
        .body()
        .map(Element::from)
        .ok_or_else(|| JsValue::from_str("no body"))
}

fn js_err(e: impl std::fmt::Display) -> JsValue {
    JsValue::from_str(&e.to_string())
}
