# The Bee TUI Design System

The shared visual language for everything bee prints to a terminal — the REPL chrome, the
`render` tool, batch/episode output, and the mascot. One vocabulary, drawn from `bee_harness::viz`,
so the output reads as a single system whether it's a status dot or the startup banner.

> **Authoritative token spec:** palette, glyphs, status grid, layout primitives, and rendering caps
> live in [`specs/003-visual-render/contracts/honeycomb.md`](../specs/003-visual-render/contracts/honeycomb.md).
> This document is the front door: the principles that govern the whole system, plus the **mascot &
> sprite** conventions that the contract left to "Slice 2."

---

## Principles

These are non-negotiable and already hold across the codebase — keep them holding.

1. **Color is semantic, never decorative.** Nothing hardcodes a hex or SGR code at a call site. Six
   roles — `success` `error` `info` `dim` `accent` `text` — resolve through the active theme
   (`viz::palette` → `viz::theme`). Add meaning by picking a role, not a color.
2. **Monochrome is the floor, color is enhancement.** Every signal must survive `NO_COLOR`
   (honored per <https://no-color.org>) and a non-tty. Dots keep their glyph; the sprite degrades to
   block silhouettes; denials keep the `⚠` and the word `DENIED`. Never let color be the *only*
   carrier of state — pair it with a glyph or a letter.
3. **One glyph vocabulary.** Every glyph in project output comes from `viz::glyph` (`●○▸✓✗⚠⬡─│`).
   Nothing ad-hoc. A new state reuses an existing glyph before it invents one.
4. **Theme by indirection.** A new theme is one entry in `viz::themes` (a token→color map), never a
   code change at a render site. `honeycomb` is the default and is byte-identical to the pre-theming
   basic-ANSI output; Catppuccin ×4, Dracula, and Nord ship alongside it.
5. **Spatial stability.** Banners, status lines, and dot grids keep fixed positions and fixed column
   order. Chrome doesn't rearrange itself between frames.

## Identity

| Element | What | Where |
|---|---|---|
| **Honey yellow** | `#E5A100` — the one brand color; info, banners, the bee | `palette::HONEY` / `Role::Info` |
| **`⬡` hexagon** | the inline identity glyph (honeycomb cell) | `glyph::HEX` |
| **The bee** | the 16×16 mascot + wing-flap, on the startup banner | `viz::bee` |

The name is the whole metaphor: a **bee** in a **honeycomb**, working the cells. Yellow-and-black is
the identity — which is exactly why the mascot must *read* as a bee (see below).

---

## Mascot & sprite system

The mascot is a **16×16 pixel sprite** rendered with vertical half-blocks — two stacked pixels per
terminal cell (`▀▄█`), so 16 rows of pixels become **8 rows of text**, 16 columns wide. Truecolor
when available, quantized to 256/16 colors otherwise, block silhouette under `NO_COLOR`. Pipeline:
`viz::bee` (bitmap) → `viz::sprite_render::render_frame_with` → ANSI. No image protocol, no new crate.

### Authoring a sprite

A sprite is 16 rows of 16 characters. Each character is a **palette letter**:

| Letter | Color | Role in the bee |
|---|---|---|
| `K` | `#1A1A1A` near-black | outline, head, **stripes** |
| `Y` | `#E5A100` honey | body / abdomen fill |
| `W` | `#FFFFFF` white | wings, eyes |
| `B` | `#4A90D9` blue | wing tint (optional) |
| `R` | `#D75F5F` sting-red | stinger |
| `.` | transparent | background (emits no escape) |

Rows shorter than 16 are padded with transparent; anything not in the palette is transparent.

### What makes it read as a bee

The mascot is a **side-view flying bee**, facing right and tilted up as if in flight. It replaced the
original "yellow blob" by turning on the silhouette cues that a bee is recognized by — its outline and
its stripes read long before its color. Left → right the anatomy is legible:

1. **A stinger** (`R`) at the tail, then
2. **A striped abdomen** — alternating `Y` and `K` **vertical** bands (side view, so the stripes run
   across the body). *This is the defining feature;* a solid yellow body reads as a duck. If you draw
   one thing, draw the stripes.
3. **Wings over the thorax** (`W`, upper-center) — attached to the middle of the back, not stacked on
   the head. This is the "it's flying" cue.
4. **A head with an eye and an antenna** (`K` head, `W` eye glint, `K` antenna) at the front — the
   "this is a bug, not a bird" cue. Three little legs (`K`) dangle below.

Verify the silhouette in monochrome first — if the black-and-white block outline doesn't say "bee,"
color won't save it.

### Animation

Three frames — `FRAME_UP` / `FRAME_MID` / `FRAME_DOWN` — that differ **only in the wing region**
(rows 3–6, columns 4–10); the body, head, antenna, and legs are byte-identical across all three, so
the bee holds still while its wings flap. The animator bounces `up → mid → down → mid` at 150 ms/frame
for one cycle beside the banner, then reclaims its rows. `sprite()` (the static/Rhai surface) returns
the level-wing `FRAME_MID` resting pose. Deterministic: built once via `OnceLock`, no clock or RNG.

### Preview

```sh
cargo run -p bee-harness --example bee_preview      # static sprite, real pipeline
./target/debug/bee-repl --provider <cfg>            # full banner + wing-flap
BEE_MASCOT=0 ./target/debug/bee-repl ...            # or --no-bee to suppress
NO_COLOR=1 cargo run -p bee-harness --example bee_preview   # check the monochrome floor
```

---

## Component inventory

The reusable chrome pieces, all built from the tokens + glyphs above:

| Component | Glyphs / tokens | Used for |
|---|---|---|
| **Startup banner** | honey title + `⬡` + mascot | REPL identity header |
| **Status dot grid** | `●` pollen / `●` sting / `○` smoke | batch & episode pass/fail (`viz::status_grid`) |
| **Tool-call line** | `▸` arrow, `✓`/`✗` result, dim args | live tool execution in the REPL |
| **Denial line** | bold `⚠ DENIED` in sting-red | kernel/policy denials |
| **Spinner** | braille `⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏` | in-flight work |
| **Rules / splits** | `─` `│`, `vsplit`/`hsplit` (max depth 4) | layout separators |
| **Grid** | `grid(rows, cols)` → cells hold any widget; spans union tracks | model-defined N×M dashboards ([plan](./grid-tui-plan.md)) |

Render widgets — charts, gauges, tables, dot grids, sprites, grids — are authored in Rhai
(`viz::render_api`) and validated into a pure-serde `RenderSpec`. The full component pipeline and the
grid system live in [the grid TUI plan](./grid-tui-plan.md).

### Full-screen surface (008-grid-tui)

`bee-repl --tui` (build with `--features tui`) adds a full-screen front-end over the same
conversation core the inline REPL uses, so both stay in lock-step.

| Element | Form | Used for |
|---|---|---|
| **Regions** | header · chat · input · footer, fixed positions | the frame never reflows between turns |
| **Panel** | bordered block titled with its id, right-hand column | model-owned live output (`render_to`) |
| **Panel overlay** | centered popup over chat, toggled with `p` | panels on single-pane (< 120 col) layouts |
| **Help overlay** | `?` — full key reference | discoverability ladder (footer → `?`) |
| **Overflow note** | `+N more — remove_panel()/clear_panels()` | honest report when panels exceed the column |
| **Too-small** | `terminal too small (min 40×10)` | below the hard floor; nothing else is drawn |

Panel rules that shape the design: a panel is **replaced in place** by id (never duplicated), panels
are sized to their **content** rather than split evenly, and a widget that cannot be displayed fails
the script with an actionable message instead of drawing a smear. Inline widgets render through the
same truecolor buffer path as panels — never an ASCII placeholder.

New chrome should compose these before introducing anything new. If a state needs a color, it needs
a role; if it needs a mark, it needs a glyph from the vocabulary.

---

## Checklist for new TUI work

- [ ] Colors come from a `Role`, not a literal SGR/hex.
- [ ] Readable with `NO_COLOR=1` and off a tty (glyph/layout still carries the meaning).
- [ ] Any new glyph is justified against the existing vocabulary first.
- [ ] A new theme would be a `viz::themes` entry, not a render-site change.
- [ ] Sprites verified in monochrome silhouette before judging the color.
- [ ] Fixed column order; chrome doesn't reflow between frames.
- [ ] Full-screen work restores the terminal on **every** exit path (quit / Ctrl-C / Ctrl-Z / panic).
- [ ] Anything drawn in a panel is also reachable on a narrow terminal (the `p` overlay).
