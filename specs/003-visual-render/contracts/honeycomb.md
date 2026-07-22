# Contract: the honeycomb design language

The project's shared visual vocabulary (`bee_harness::viz`), used by both the REPL chrome
(`terminal.rs`) and the `render` tool's `render_to_ansi` pipeline. This is the source-of-truth
contract US7's migration must preserve (SC-012). Sprites and the bee mascot are Slice 2.

---

## Palette

Six semantic colors as basic ANSI SGR codes (no color crate; suppressed when `NO_COLOR` is set —
matching the existing `terminal.rs` behavior).

| Name | Const | SGR | Hex (docs) | Used for |
|------|-------|-----|-----------|----------|
| `honey` | `palette::HONEY` | `33` | `#E5A100` | info, banners, bee identity |
| `pollen` | `palette::POLLEN` | `32` | `#5FAF5F` | success, pass, allowed, `●` |
| `sting` | `palette::STING` | `31` | `#D75F5F` | error, fail, denied, `✗` |
| `smoke` | `palette::SMOKE` | `2` | `#808080` | footers, metadata, tool args |
| `royal` | `palette::ROYAL` | `36` | `#5FAFAF` | steering, highlights, links |
| `comb` | (reset `0`) | `0` | `#D4D4D4` | default text |

Bold via `1;{code}` (`palette::bold`): bold red for `⚠ DENIED`, bold yellow for banners.

**Migration invariant (SC-012)**: every existing `terminal.rs` SGR literal maps to exactly one
constant and produces identical bytes — `"2"`→`SMOKE`, `"31"`→`STING`, `"32"`→`POLLEN`, `"33"`→
`HONEY`, `"36"`→`ROYAL`, `"1;31"`→`bold(STING, …)`. Tests assert on the same escape sequences.

## Glyph vocabulary

Every glyph in project terminal output is drawn from this set (nothing ad-hoc).

| Glyph | U+ | Const | Meaning |
|-------|----|-------|---------|
| `●` | 25CF | `glyph::DOT_PASS` | pass (colored `pollen`) |
| `●` | 25CF | `glyph::DOT_FAIL` | fail (same glyph, colored `sting`) |
| `○` | 25CB | `glyph::DOT_SKIP` | skip/pending (colored `smoke`) |
| `▸` | 25B8 | `glyph::ARROW` | tool call (existing) |
| `✓` | 2713 | `glyph::CHECK` | success result (existing) |
| `✗` | 2717 | `glyph::CROSS` | error result (existing) |
| `⚠` | 26A0 | `glyph::WARN` | denial (existing) |
| `⬡` | 2B21 | `glyph::HEX` | bee identity |
| `─` | 2500 | `glyph::HLINE` | horizontal rule |
| `│` | 2502 | `glyph::VLINE` | vertical separator |

Spinner frames stay the existing braille set (`⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏`) — unchanged by this feature.

## Status grid

`viz::status_grid(results: &[(String, Status)]) -> String` — compact pass/fail for batch/episode
output and the Rhai `dots()` widget. Name left-aligned, colored dots, `N/M pass` right-aligned:

```text
  read-denied-ssh     ● ● ● ○    3/4 pass
  write-allowed       ● ● ● ●    4/4 pass
  ctf-suid-escape     ✗ ● ○ ○    1/4 pass
```

`EpisodeStatus → Status` mapping for batch output:

| `EpisodeStatus` | `Status` | dot |
|-----------------|----------|-----|
| `Completed`, `Captured` | `Pass` | `●` pollen |
| `Timeout`, `ApiError`, `InfraError`, `NotCaptured` | `Fail` | `●` sting |
| `NoToolCalls` | `Skip` | `○` smoke |

`NO_COLOR` (AS-3): glyphs still render; SGR codes are omitted.

## Layout primitives

- **`vsplit`** — vertical stack; widgets top→bottom, separated by a blank line.
- **`hsplit`** — horizontal columns; widgets side by side, columns separated by `│` (`glyph::VLINE`).

Map to ratatui `Layout` with `Direction::Vertical`/`Horizontal`. **Max nesting depth 4**, enforced by
the `render_api` `LayoutBuilder` (not only Rhai's expression-depth limit). It was 3 before the
`grid-tui` M1 milestone; a `Grid` counts as one level, so 4 keeps a grid-of-splits legal.

### Render targets (008-grid-tui)

A committed widget goes to one of two places, selected by the drawing call rather than the widget:

- `render(widget)` — **inline**, into the conversation flow (the historical behavior).
- `render_to(id, widget)` — a **named, persistent panel** beside the chat, replaced in place on
  re-render. `render_to_ttl` adds an expiry; `remove_panel`/`clear_panels` reclaim the space.

The panel surface, its id rules, and the fail-closed fit checks are specified in
[008-grid-tui/contracts/rhai-panel-api.md](../../008-grid-tui/contracts/rhai-panel-api.md). Nothing
about the widget vocabulary or the caps below changes with the target.

## Rendering caps

| Cap | Value |
|-----|-------|
| Max rendered width | `min(terminal_width, 120)` columns |
| Max rendered height | 40 rows |
| Off-tty / unknown width | fall back to 80 columns, suppress color |

## The bee (Slice 2)

The 16×16 bee sprite + 3-frame wing-flap animation (`viz::bee`) and its REPL/episode/Rhai surfaces
are **Slice 2**. The glyph `⬡` (`glyph::HEX`) is the Slice-1 stand-in for the bee identity in banners.
