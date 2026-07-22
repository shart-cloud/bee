//! The full-screen TUI front-end (008-grid-tui, M3–M4). Feature-gated behind `tui`.
//!
//! Architecture: The Elm Architecture (Model → Message → update → view) over the harness's tokio
//! loop, consuming [`crate::session::SessionEvent`]s beside terminal input. The [`run`] event loop
//! (T018) selects over crossterm's async [`EventStream`], the `SessionEvent` channel, and an
//! on-demand tick, driving the shared [`crate::repl::run_exchange`] turn loop through a
//! [`SessionSink`] — the same engine the inline REPL uses (T005/T006), so both front-ends stay in
//! lock-step. `theme_bridge` (T007) is implemented; `--tui` launch wiring lands in T019.

pub mod app;
pub mod chat;
pub mod input;
pub mod message;
pub mod panels;
pub mod term;
pub mod theme_bridge;
pub mod view;

use std::collections::VecDeque;
use std::io;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crossterm::event::EventStream;
use futures_util::StreamExt;

use crate::metrics::Recorder;
use crate::provider::{Conversation, Model};
use crate::render_spec::RenderSpec;
use crate::repl::{effective_system_prompt, run_exchange, ReplConfig, SteeringQueue};
use crate::sandbox::Sandbox;
use crate::session::SessionSink;
use crate::tools::ToolRegistry;

use app::{update, App};
use chat::{ChatMessage, Role};
use message::Message;
use term::{message_from_event, Tui};

/// Run the full-screen TUI front-end to completion (008-grid-tui, US1 T018).
///
/// Owns the same conversation core as [`crate::repl::run_repl`] — a [`Conversation`], a steering
/// queue, and a metrics recorder — minus the readline editor, since input arrives through
/// crossterm instead. Each submitted line drives one [`run_exchange`] turn via a [`SessionSink`];
/// the loop pumps streaming tokens, tool results, and denials into the model as they arrive, so the
/// screen stays live and Ctrl-C quits promptly even mid-turn. Returns once the user quits (or the
/// input stream closes), with the terminal already restored.
pub async fn run(
    model: &dyn Model,
    registry: &mut ToolRegistry,
    sandbox: &mut Sandbox,
    config: &ReplConfig,
) -> io::Result<()> {
    // Shared conversation core, identical to the inline REPL's setup (session/mod.rs).
    let mut conversation = Conversation {
        system: effective_system_prompt(&config.system_prompt, registry),
        messages: Vec::new(),
    };
    let steering: SteeringQueue = Arc::new(Mutex::new(VecDeque::new()));
    let recorder = Recorder::new("tui", format!("tui:{}", std::process::id()));

    // Enter the alternate screen + raw mode; the panic hook (installed here) restores first. The
    // guard then covers the abnormal exits — an early `?` return or a panic unwinding through the
    // loop restore the terminal on drop (contracts/modes-and-cli.md; T010).
    let mut terminal = term::init();
    let mut restore_guard = term::RestoreGuard::terminal();
    let (cols, rows) = crossterm::terminal::size().unwrap_or((80, 24));
    let mut app = App::new(cols, rows);

    // Greeting, mirroring the inline REPL's info banner — plus the resting-pose mascot when enabled,
    // which also exercises the sprite→Buffer rasterizer (T008) on the full-screen path.
    if config.mascot {
        app.chat.push(ChatMessage::widget(RenderSpec::Sprite {
            spec: crate::viz::bee::sprite(),
        }));
    }
    app.chat.push(ChatMessage::text(
        Role::System,
        format!(
            "interactive session — {} — type a message, ? for help, q to quit",
            model.id()
        ),
    ));

    let mut events = EventStream::new();

    // The Elm loop: draw, then wait for the next thing that changes the model.
    while !app.should_quit {
        app.panels.prune(std::time::Instant::now());
        // Publish the drawable regions so the render tool can reject widgets this screen can't show.
        crate::viz::viewport::set(view::viewport_for(&app));
        terminal.draw(|f| view::view(&app, f))?;

        // A submitted line starts a model turn; drive it to completion while still pumping the UI.
        if let Some(text) = app.take_outbox() {
            #[allow(clippy::too_many_arguments)]
            run_turn(
                &mut app,
                &mut terminal,
                &mut events,
                &text,
                model,
                &mut conversation,
                registry,
                sandbox,
                config,
                &steering,
                recorder.as_ref(),
            )
            .await?;
            continue;
        }

        // Idle: block on a single terminal event, apply it, loop back to redraw. When a panel carries
        // a TTL we also wake periodically so it expires on time instead of lingering until the next
        // keypress; with no expiring panels this stays a pure blocking wait (no idle CPU).
        let next = if app.panels.has_expiring() {
            tokio::select! {
                ev = events.next() => ev,
                _ = tokio::time::sleep(Duration::from_millis(250)) => continue,
            }
        } else {
            events.next().await
        };
        match next {
            Some(Ok(ev)) => {
                if let Some(msg) = message_from_event(ev) {
                    update(&mut app, msg);
                }
            }
            // A read error or the stream ending both mean input is gone — exit cleanly.
            Some(Err(_)) | None => break,
        }
    }

    // Normal exit (quit / Ctrl-C / Ctrl-D / closed input): restore explicitly, then disarm the guard
    // so its Drop doesn't restore a second time (contracts/modes-and-cli.md: normal-quit row).
    term::restore();
    restore_guard.disarm();
    Ok(())
}

/// Drive one user→model exchange to completion while keeping the UI live (008-grid-tui, T018).
///
/// The [`run_exchange`] future borrows the conversation core; we poll it inside `tokio::select!`
/// beside the `SessionEvent` channel (streaming output), terminal input (so Ctrl-C / resize stay
/// responsive), and an on-demand tick (redraws the "working…" indicator). Every branch redraws on
/// the next loop turn. Quitting mid-turn drops the exchange future, cancelling the in-flight call.
#[allow(clippy::too_many_arguments)]
async fn run_turn(
    app: &mut App,
    terminal: &mut Tui,
    events: &mut EventStream,
    text: &str,
    model: &dyn Model,
    conversation: &mut Conversation,
    registry: &mut ToolRegistry,
    sandbox: &mut Sandbox,
    config: &ReplConfig,
    steering: &SteeringQueue,
    recorder: Option<&Recorder>,
) -> io::Result<()> {
    let (sink, mut rx) = SessionSink::new();
    let exchange = run_exchange(
        text,
        model,
        conversation,
        registry,
        sandbox,
        config,
        &sink,
        steering,
        recorder,
    );
    tokio::pin!(exchange);

    // The tick exists only for the duration of a turn — the "on-demand" tick (T018): it paces the
    // "working…" indicator's redraws and idles completely between turns. (The first tick fires
    // immediately, which just forces one extra harmless redraw.)
    let mut tick = tokio::time::interval(Duration::from_millis(120));
    let mut turn_done = false;

    loop {
        app.panels.prune(std::time::Instant::now());
        crate::viz::viewport::set(view::viewport_for(app));
        terminal.draw(|f| view::view(app, f))?;
        // A mid-turn quit (Ctrl-C / q from chat focus) drops `exchange`, cancelling the call.
        if app.should_quit {
            return Ok(());
        }

        tokio::select! {
            // Guarded so a completed future is never polled again.
            _ = &mut exchange, if !turn_done => {
                turn_done = true;
                // The exchange emits its trailing events (footer/TurnDone) before returning; fold
                // them in, redraw once more via the loop head, then finish.
                while let Ok(ev) = rx.try_recv() {
                    update(app, Message::session(ev));
                }
            }
            Some(ev) = rx.recv() => update(app, Message::session(ev)),
            maybe = events.next() => match maybe {
                Some(Ok(ev)) => {
                    if let Some(msg) = message_from_event(ev) {
                        update(app, msg);
                    }
                }
                Some(Err(_)) | None => app.should_quit = true,
            },
            _ = tick.tick() => {}
        }

        // Once the turn has ended and its trailing events are drained, one final redraw + return.
        if turn_done {
            terminal.draw(|f| view::view(app, f))?;
            return Ok(());
        }
    }
}
