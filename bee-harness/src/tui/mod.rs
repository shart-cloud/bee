//! The full-screen TUI front-end (008-grid-tui, M3–M4). Feature-gated behind `tui`.
//!
//! Architecture: The Elm Architecture (Model → Message → update → view) over the harness's tokio
//! loop, consuming [`crate::session::SessionEvent`]s beside terminal input. The event loop, the
//! `App`/`update` reducer, and `view` land in the US1 tasks (T011–T019); the module tree below is
//! scaffolded (T002) so each task has a home. `theme_bridge` (T007) is implemented.

pub mod app;
pub mod chat;
pub mod input;
pub mod message;
pub mod panels;
pub mod term;
pub mod theme_bridge;
pub mod view;
