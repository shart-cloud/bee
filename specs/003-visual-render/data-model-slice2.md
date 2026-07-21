# Data Model: bee Visual Rendering — Slice 2 (sprites & animation)

Entities added by Slice 2. All in `bee-harness` (Constitution V / NFR-002). The serde data types
(`SpriteSpec`/`AnimationSpec` + the additive `RenderSpec` variants) carry **no ratatui/rhai types** —
sprites render through hand-rolled ANSI (`viz::sprite_render`), never ratatui.

---

## 1. `RenderSpec` additive variants — `src/render_spec.rs` (EDIT)

The `#[non_exhaustive]` enum gains two variants (Slice 1 arms untouched):

```rust
#[non_exhaustive]
pub enum RenderSpec {
    // … Slice 1 variants (BarChart, LineChart, …, Layout) …
    Sprite    { spec: SpriteSpec },
    Animation { spec: AnimationSpec },
}
```

The Slice-1 helper methods (`element_count`, `layout_depth`, `ascii_fallback`, `summary_phrase`)
currently end in wildcard `_` arms; Slice 2 **replaces the wildcards with explicit arms**:

| Method | `Sprite` | `Animation` |
|--------|----------|-------------|
| `element_count` | 1 | `frames.len()` |
| `layout_depth` | 0 | 0 |
| `ascii_fallback` | `"[sprite {w}×{h}]"` | `"[animation: {n} frames]"` |
| `summary_phrase` | `"a {w}×{h} sprite"` | `"a {n}-frame animation"` |

---

## 2. `SpriteSpec` (struct, serde) — `src/render_spec.rs`

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpriteSpec {
    pub width: u16,                          // pixel columns (1..=32)
    pub height: u16,                         // pixel rows (1..=32)
    pub pixels: Vec<Option<(u8, u8, u8)>>,   // row-major RGB; None = transparent; len == width*height
}
```

### Validation (builders, D18 — not at deserialize)

| Rule | Value |
|------|-------|
| `width`, `height` | 1..=32 (default/recommended 16×16) |
| `pixels.len()` | `width * height` (builder guarantees; short paint rows pad transparent) |
| unknown palette char in `paint` | → `None` (transparent) |
| fully transparent | renders `⌈height/2⌉` blank rows; summary notes it |

### Rendered form (`viz::sprite_render::render_frame`)

`⌈height/2⌉` terminal rows, each a string of `▄`/`▀`/space cells with truecolor (or quantized) SGR
per half (D12/D13). A 16×16 sprite → **8** rows × 16 cols (SC-015).

---

## 3. `AnimationSpec` (struct, serde) — `src/render_spec.rs`

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnimationSpec {
    pub frames: Vec<SpriteSpec>,   // 1..=16 frames, all identical dimensions
    pub interval_ms: u64,          // per-frame duration, clamped 50..=1000
    pub bounce: bool,              // ping-pong playback (0,1,…,k,…,1)
    pub cycles: u32,               // full cycles then stop; 0 = loop until interrupted
}
```

### Validation (builders)

| Rule | Value |
|------|-------|
| `frames.len()` | 1..=16 (`anim.add` past 16 → Rhai error) |
| all frames same `(width, height)` | enforced by `anim.add` |
| `interval_ms` | clamped to 50..=1000 by `animation(ms)` |
| `cycles` | any u32; 0 = infinite until next output/interrupt |

---

## 4. `ColorMode` (enum) — `src/viz/sprite_render.rs`

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorMode { TrueColor, Ansi256, Ansi16 }

pub fn detect_color_mode() -> ColorMode;     // COLORTERM → TrueColor; TERM *256color* → Ansi256; else Ansi16
pub fn quantize_256(r: u8, g: u8, b: u8) -> u8;   // nearest xterm-256 (cube 16–231 or gray 232–255)
pub fn quantize_16(r: u8, g: u8, b: u8) -> u8;    // nearest of the 16 basic ANSI colors
pub fn render_frame(spec: &SpriteSpec) -> Vec<String>;   // one frame → half-block ANSI lines
```

Off-tty / `NO_COLOR` → monochrome (`█`/`▄`/space, no SGR), decided by the caller passing the mode.

---

## 5. Rhai builder types — `src/render_api.rs` (EDIT, additive)

| Builder | Rhai constructor | Produces |
|---------|------------------|----------|
| `PaletteBuilder` | `palette()` | `HashMap<char, Option<(u8,u8,u8)>>` (≤ 32 entries) |
| `SpriteBuilder` | `sprite(w, h, pal)` | `SpriteSpec` (≤ 32×32) |
| `AnimationBuilder` | `animation(ms)` | `AnimationSpec` (≤ 16 frames) |

- `PaletteBuilder.set(char, color)` — parses `"#RRGGBB"` or `"transparent"`; a bad hex → Rhai error.
- `SpriteBuilder.paint(rows: Array)` — one string per pixel row; each char looked up in the palette
  (unknown → transparent); `.set(x, y, color)`, `.fill(color)`.
- `AnimationBuilder.add(sprite)` (≤ 16, same dims), `.bounce(b)`, `.cycles(n)`.
- `render(widget)` / `layout.add(widget)` — extended `try_cast` arms accept `SpriteBuilder` /
  `AnimationBuilder`. An animation nested in a layout renders its **first frame** only (static
  composition); the summary notes it.
- `bee_sprite()` / `bee_animation()` — return `viz::bee` constants as builder outputs (SC-018).

---

## 6. `viz::bee` — `src/viz/bee.rs` (NEW)

```rust
pub fn sprite() -> SpriteSpec;      // the 16×16 bee, built once via OnceLock from a bitmap + palette
pub fn animation() -> AnimationSpec; // 3-frame wing-flap, interval 150ms, bounce = true
```

Deterministic (no `Date`/random). Consumed by the REPL startup banner (`--bee`/`BEE_MASCOT=1`), the
episode-completion line, and the Rhai `bee_*` functions.

---

## 7. `TerminalOutput` state change — `src/repl/terminal.rs` (EDIT)

```rust
// was: reclaim: AtomicBool          (single spinner row)
reclaim_rows: std::sync::atomic::AtomicUsize,   // 0 = none; N = next output reclaims N rows
// spinning: Arc<AtomicBool>  → the shared "one active animation" flag (spinner OR sprite animation)
// spin_task: Mutex<Option<JoinHandle<()>>>  → the shared active-animation task handle
```

`emit()` consumes `reclaim_rows`: when `N > 0`, move up N rows + clear N lines before printing.
`N = 1` reproduces the spinner's current behavior exactly (regression gate — SC-012 spinner tests).

---

## Entity relationships

```text
Rhai script ──palette()/sprite()/animation()/bee_*()──▶ builders ──render()──▶ RenderContext
                                                                                    │ take()
                                                                                    ▼
                                        RenderSpec::Sprite{SpriteSpec} / ::Animation{AnimationSpec}
                                                                                    │ serde → transcript
                     ┌──────────────────────────────────────────────────────────────┤
                     ▼                                                                ▼
   TerminalOutput::render_widget(Sprite)                       TerminalOutput::render_widget(Animation)
     → viz::sprite_render::render_frame → inline rows            → viz::animator: claim active slot
                                                                    (stop spinner), spawn redraw task,
                                                                    reclaim_rows = ⌈H/2⌉ on completion
```
