//! The shared conversation engine (008-grid-tui, plan M2 / research D4).
//!
//! Both front-ends — the inline REPL and the full-screen [`crate::tui`] — drive the *same* turn loop
//! (provider streaming, tool execution, transcript) and consume its output as a stream of
//! [`SessionEvent`]s. Extracting this out of `repl.rs` is task **T005**; rewiring the inline REPL onto
//! it is **T006**. This module currently defines the event contract (T004) that both consumers share.

pub mod event;

pub use event::SessionEvent;
