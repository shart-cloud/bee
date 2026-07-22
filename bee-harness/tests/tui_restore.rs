#![cfg(feature = "tui")]
//! T010 — the terminal-restore matrix (008-grid-tui, SC-001; contracts/modes-and-cli.md).
//!
//! The contract: on **every** exit path the full-screen front-end returns the terminal to the main
//! screen, cooked mode, cursor visible. CI has no tty, so we verify the *guarantee* rather than the
//! real escape bytes. Two mechanisms back the contract and both are deterministically testable here:
//!
//! * [`RestoreGuard`] restores on `Drop` — it covers the early-`?`-return and panic rows. Its action
//!   is injectable, so a counter stands in for the real terminal restore.
//! * the reducer decides the quit / interrupt rows (Ctrl-C and `q` set `should_quit`, so `run`'s
//!   loop exits and reaches the explicit restore).
//!
//! Each test below is one row of the matrix. The SIGTSTP suspend/resume row is a reducer no-op today
//! (full signal handling is US3/T035); its test pins that honest state so the matrix isn't
//! over-claiming coverage.

use std::cell::Cell;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::rc::Rc;

use crossterm::event::{KeyCode, KeyModifiers};

use bee_harness::tui::app::{update, App};
use bee_harness::tui::message::Message;
use bee_harness::tui::term::RestoreGuard;

/// A restore counter paired with a guard whose `Drop` increments it — the terminal stand-in.
fn counting_guard() -> (Rc<Cell<usize>>, RestoreGuard) {
    let n = Rc::new(Cell::new(0));
    let seen = n.clone();
    (n, RestoreGuard::with(move || seen.set(seen.get() + 1)))
}

// --- Row: early return (`?`) — the guard restores when the scope unwinds early ------------------

#[test]
fn early_return_restores_via_the_guard() {
    let (n, guard) = counting_guard();
    let run = move || -> Result<(), ()> {
        let _g = guard; // armed for the whole scope
        Err(())?; // `?` returns early before any explicit restore; `_g` drops here
        unreachable!("returned early");
    };
    let _ = run();
    assert_eq!(n.get(), 1, "the early `?` return restored exactly once");
}

// --- Row: panic — unwinding through the loop restores, and the panic still propagates ------------

#[test]
fn panic_restores_via_the_guard_and_still_propagates() {
    let (n, guard) = counting_guard();
    let result = catch_unwind(AssertUnwindSafe(move || {
        let _g = guard; // armed
        panic!("boom"); // unwinds; `_g` drops on the way out
    }));
    assert!(result.is_err(), "the panic still propagates past the guard");
    assert_eq!(n.get(), 1, "unwinding restored the terminal exactly once");
}

// --- Row: normal quit — explicit restore then disarm, so the guard never restores twice ----------

#[test]
fn disarm_prevents_a_double_restore_on_the_normal_path() {
    // `run`'s normal path calls the real `restore()` explicitly, then disarms the guard.
    let (n, mut guard) = counting_guard();
    guard.disarm();
    drop(guard);
    assert_eq!(
        n.get(),
        0,
        "a disarmed guard is a no-op — the explicit restore already ran, so no double restore"
    );
}

// --- Row: interrupt (Ctrl-C) — treated as quit, so the loop exits and reaches restore ------------

#[test]
fn ctrl_c_is_treated_as_quit() {
    let mut app = App::new(120, 40);
    update(&mut app, Message::char_mods('c', KeyModifiers::CONTROL));
    assert!(
        app.should_quit,
        "Ctrl-C sets should_quit → the event loop exits and restores"
    );
}

// --- Row: normal quit key (`q` from chat focus) — same exit-and-restore path ---------------------

#[test]
fn q_from_chat_focus_quits() {
    let mut app = App::new(120, 40);
    update(&mut app, Message::key(KeyCode::Tab)); // focus the chat pane
    update(&mut app, Message::char('q'));
    assert!(
        app.should_quit,
        "`q` sets should_quit → the loop exits and restores"
    );
}

// --- Row: suspend/resume — a reducer no-op today (full SIGTSTP handling is US3/T035) -------------

#[test]
fn suspend_resume_is_a_noop_today_and_never_a_hidden_quit() {
    let mut app = App::new(120, 40);
    update(&mut app, Message::Suspend);
    update(&mut app, Message::Resume);
    assert!(
        !app.should_quit,
        "suspend/resume must not quit — the eventual normal exit still restores"
    );
    assert_eq!(
        app.size,
        (120, 40),
        "suspend/resume leave model state intact"
    );
}
