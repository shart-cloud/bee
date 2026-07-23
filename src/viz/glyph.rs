//! The honeycomb glyph vocabulary (003-visual-render, FR-026). Every glyph in the project's terminal
//! output is drawn from this set — nothing ad-hoc (contracts/honeycomb.md).

/// Pass / success — colored `POLLEN` (green).
pub const DOT_PASS: &str = "\u{25CF}"; // ●
/// Fail / error — same glyph as [`DOT_PASS`], colored `STING` (red).
pub const DOT_FAIL: &str = "\u{25CF}"; // ●
/// Skipped / pending / neutral — colored `SMOKE` (dim).
pub const DOT_SKIP: &str = "\u{25CB}"; // ○
/// Tool-call indicator.
pub const ARROW: &str = "\u{25B8}"; // ▸
/// Successful tool result.
pub const CHECK: &str = "\u{2713}"; // ✓
/// Error tool result.
pub const CROSS: &str = "\u{2717}"; // ✗
/// Kernel denial / warning.
pub const WARN: &str = "\u{26A0}"; // ⚠
/// The bee / honeycomb identity glyph.
pub const HEX: &str = "\u{2B21}"; // ⬡
/// Horizontal rule / separator.
pub const HLINE: &str = "\u{2500}"; // ─
/// Vertical separator.
pub const VLINE: &str = "\u{2502}"; // │
