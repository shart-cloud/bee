# Feature Specification: Full-Screen TUI with Model-Owned Live Grid Panels

**Feature Branch**: `008-grid-tui`

**Created**: 2026-07-21

**Status**: Draft

**Input**: User description: "Full-screen TUI mode (opt-in) that balances a live chat history with an
N×M grid the model can take over to show live data, building on the existing Rhai visual scripting
and the RenderSpec grid. Derived from docs/grid-tui-plan.md §3–§4 (milestones M3–M4)."

## User Scenarios & Testing *(mandatory)*

### User Story 1 - A full-screen session that never corrupts the terminal (Priority: P1)

An operator launches bee in full-screen mode. Their conversation with the model fills a scrollable
chat area; they type into a persistent input line at the bottom; a footer shows the most useful keys.
Model replies stream in as they arrive. When they quit — or the process is interrupted, suspended, or
crashes — the terminal is returned exactly as it was, with their shell prompt and prior scrollback
intact.

**Why this priority**: This is the foundation. Without a driver that owns the screen *and* restores it
on every exit path, nothing else can be built, and a TUI that leaves the terminal in raw mode is worse
than no TUI. On its own it already delivers value: a cleaner, self-contained conversation surface.

**Independent Test**: Run a full session (send messages, receive streamed replies, scroll history)
then exit via each path — normal quit, Ctrl-C, suspend/resume, and a forced panic — and confirm the
terminal is usable and the prior scrollback is untouched every time.

**Acceptance Scenarios**:

1. **Given** a capable terminal, **When** the operator launches full-screen mode, **Then** the chat
   area, input line, and footer hint bar appear in fixed positions and the shell scrollback is not
   disturbed.
2. **Given** an in-progress model reply, **When** tokens stream in, **Then** they appear incrementally
   in the chat area without blocking input and without a busy-wait redraw.
3. **Given** an active session, **When** the operator quits, is interrupted (Ctrl-C), suspends
   (Ctrl-Z) and resumes, or the program panics, **Then** in every case the terminal is restored to a
   usable state (normal screen, cooked mode, visible cursor) and any panic message is readable.
4. **Given** the operator resizes the terminal mid-session, **When** the new size is applied, **Then**
   the layout reflows without crashing and no content is lost.

---

### User Story 2 - The model takes over a grid panel and updates it live (Priority: P2)

During a conversation the model decides to surface structured, evolving data — say resource metrics or
a task board. It emits an N×M grid (dimensions and cell contents of its own choosing) addressed to a
named panel. The panel appears alongside the chat and **persists**. On later turns the model re-renders
the same panel and it updates in place, rather than scrolling away as a new block each time. The
operator keeps chatting while the panel stays live.

**Why this priority**: This is the headline capability the whole iteration exists for — "the model
takes over a region to show data." It depends on P1 but is where the distinctive value lives.

**Independent Test**: Have the model render a grid to a named panel, then render an updated grid to the
same panel on a subsequent turn; confirm exactly one panel exists and it reflects the latest data,
while the chat history is unchanged.

**Acceptance Scenarios**:

1. **Given** a full-screen session, **When** the model renders a grid addressed to a new panel name,
   **Then** a persistent panel appears beside the chat showing that grid, and the chat is unaffected.
2. **Given** an existing named panel, **When** the model renders again to the same name, **Then** the
   panel's contents are replaced in place — no duplicate panel, no scrollback churn.
3. **Given** the model renders a grid without a panel target, **When** the reply is shown, **Then** the
   grid appears inline in the chat flow (the existing default behavior), not as a persistent panel.
4. **Given** a session with a live panel, **When** the session transcript is replayed, **Then** the
   panel's final state is reconstructed from recorded data (no live-only state is lost).
5. **Given** the model emits panel updates rapidly, **When** they arrive faster than the display
   refreshes, **Then** updates are coalesced so the UI stays responsive and shows the latest state.

---

### User Story 3 - Graceful degradation and honest fallback (Priority: P3)

Someone pipes bee's output to a file, runs it in a 60-column split, sets `NO_COLOR`, or is on a
minimal terminal. Instead of a broken layout or a hard failure, bee does the sensible thing: it falls
back to the existing inline line-based experience when full-screen isn't viable, degrades color to
monochrome when asked, collapses to a single pane on narrow terminals, and shows a clear "terminal too
small" message below the minimum size. Copying a message works even over a remote connection.

**Why this priority**: Robustness and trust. The feature must never make the tool *worse* in
environments where a full-screen UI can't run. It is lower priority because P1/P2 deliver the core
value, but it is required before the feature is on by default anywhere.

**Independent Test**: Run the same session with stdout redirected to a file, under `NO_COLOR`, and at
40×10, and confirm each produces usable, non-broken output (linear text, monochrome, and a too-small
message respectively) rather than a corrupted screen.

**Acceptance Scenarios**:

1. **Given** stdout is not an interactive terminal (piped/redirected), **When** a session runs, **Then**
   it uses the inline line-based path and its output is linear, capturable text.
2. **Given** `NO_COLOR` is set, **When** the UI renders, **Then** every state remains distinguishable
   without color (glyphs/labels/layout carry the meaning).
3. **Given** a terminal narrower than the two-pane threshold, **When** the layout renders, **Then** it
   collapses to a single pane (the grid reachable on demand) with no overlapping or truncated chrome.
4. **Given** a terminal below the minimum usable size, **When** the UI renders, **Then** a clear
   "terminal too small" message is shown instead of a broken layout.
5. **Given** the operator copies the current message, **When** they paste elsewhere — including over a
   remote session — **Then** the message text is on their clipboard.

### Edge Cases

- **Resize or suspend mid-stream**: a terminal resize or suspend/resume while a reply is streaming must
  re-layout and continue, not tear or drop tokens.
- **Panel reused with different dimensions**: re-rendering a panel with a different N×M replaces it
  cleanly (old cells cleared).
- **Panel larger than its pane**: a grid too large for the available panel area is truncated or scaled
  within the pane with a clear indication, never drawn outside it.
- **Very long conversations**: the chat area stays responsive regardless of history length (older
  content is virtualized/scrolled, not all rendered every frame).
- **Reserved keys**: terminal-reserved combinations (interrupt, suspend, flow control) keep their
  terminal meaning and are never rebound to app actions.
- **Multi-line paste** into the input line is handled without executing partial lines.
- **Minimal/dumb terminals** without Unicode or advanced features degrade to ASCII rather than emitting
  unusable glyphs.

## Requirements *(mandatory)*

### Functional Requirements

**Full-screen driver (P1)**

- **FR-001**: The system MUST provide a full-screen conversational mode that renders on a dedicated
  screen surface without polluting the terminal's normal scrollback.
- **FR-002**: The system MUST restore the terminal to a usable state on every exit path — normal quit,
  interrupt, suspend/resume, and panic — before surfacing any error text.
- **FR-003**: The system MUST re-lay-out on terminal resize without crashing or losing content.
- **FR-004**: The system MUST stream model output incrementally into the chat area without blocking
  operator input and without redrawing on a fixed timer (idle sessions do no work).
- **FR-005**: The system MUST present a persistent input line and a footer hint bar showing the most
  useful keys, with a full key reference reachable on demand.
- **FR-006**: The operator MUST be able to scroll the chat history (keyboard, and mouse where
  available) independently of ongoing output.
- **FR-007**: The system MUST keep chat, panel, input, and footer regions in fixed positions; regions
  MUST NOT reorder or move on focus changes.

**Model-owned grid panels (P2)**

- **FR-008**: The model MUST be able to render a grid (any dimensions and cell contents it chooses)
  either inline in the chat flow or addressed to a named persistent panel.
- **FR-009**: Rendering to an existing panel name MUST replace that panel's contents in place, with no
  duplicate panel and no scrollback churn; rendering to a new name MUST create a new panel.
- **FR-010**: Panel state MUST be recorded in the session transcript such that replay reconstructs each
  panel's final state; no live-only state may be required to reconstruct the view.
- **FR-011**: The system MUST coalesce panel updates that arrive faster than the display refreshes,
  always converging on the latest state.
- **FR-012**: A grid or panel whose content exceeds its available area MUST be truncated or scaled
  within that area with a clear indication, never drawn outside its region.

**Degradation, fallback, accessibility (P3)**

- **FR-013**: When a capable interactive terminal is not available (e.g. piped/redirected output), the
  system MUST fall back to the existing inline line-based experience and produce linear, capturable
  output.
- **FR-014**: The system MUST honor `NO_COLOR` and remain fully legible in monochrome; color MUST never
  be the sole carrier of a state.
- **FR-015**: The layout MUST degrade responsively — two panes on wide terminals, a single pane on
  narrow ones — and MUST show a clear "terminal too small" message below the minimum usable size
  instead of a broken layout.
- **FR-016**: The operator MUST be able to copy the current message to the system clipboard, including
  over a remote session.
- **FR-017**: Terminal-reserved key combinations MUST retain their terminal meaning and MUST NOT be
  rebound to application actions.

**Cross-cutting constraints**

- **FR-018**: Full-screen mode MUST be opt-in (explicitly selected) for its initial release; the inline
  line-based experience remains the default and the fallback.
- **FR-019**: The feature MUST reuse the existing visual vocabulary — semantic color roles, themes, the
  glyph set, and the `RenderSpec` widget/grid model — rather than introduce a parallel styling system.
- **FR-020**: The feature MUST NOT weaken the existing render-script sandbox or alter any
  security-enforcement behavior; it is a presentation layer only.
- **FR-021**: The full-screen front-end MUST live in the harness layer and MUST NOT introduce an
  async-runtime or terminal dependency into the runtime-free core (Constitution Principle V).

### Key Entities *(include if feature involves data)*

- **Session view**: the operator-facing surface for one conversation — comprising the chat history, any
  active panels, the input line, and the footer.
- **Chat message**: one turn's content (operator or model), which may embed an inline rendered widget;
  ordered, scrollable, and preserved in scroll history.
- **Panel**: a named, persistent display region owned by the model, holding one rendered widget
  (typically a grid); addressable by name and replaced in place on update.
- **Grid** *(existing)*: an N×M arrangement of cells, each holding any widget, with optional spans and
  track weights — the primary content a panel displays.
- **Transcript event**: the recorded stream of a session (messages, tool results, panel updates) from
  which the view can be reconstructed on replay.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: Across a test matrix of every exit path (quit, interrupt, suspend/resume, panic) the
  terminal is left usable (normal screen, cooked mode, visible cursor) in 100% of cases, with prior
  scrollback intact.
- **SC-002**: A model panel update is reflected in the display within 200 ms of the result being
  produced, and re-rendering an existing panel never produces a second panel.
- **SC-003**: An idle session (no input, no streaming) performs no periodic redraws and consumes
  effectively no CPU.
- **SC-004**: With stdout not an interactive terminal, a session's output is linear, capturable text
  equivalent to today's inline experience (no screen-control escapes that break capture).
- **SC-005**: Under `NO_COLOR`, every acceptance scenario remains passable using only glyphs, labels,
  and layout (verified by removing color and re-checking distinguishability of states).
- **SC-006**: At a 60-column width the layout presents a single usable pane with no overlapping or
  truncated chrome; below the minimum size a "terminal too small" message is shown instead of a broken
  layout.
- **SC-007**: Copying the current message places its text on the system clipboard, including when the
  session runs over a remote connection.
- **SC-008**: A live panel survives at least 20 consecutive update turns while chat continues, always
  showing the latest state and never leaving stale duplicate regions.
- **SC-009**: A session transcript replayed after the fact reconstructs each panel's final state with
  no divergence from what was shown live.

## Assumptions

- **Reuses the existing render stack**: the `RenderSpec` widget/grid model, the sandboxed Rhai drawing
  API (including `grid()`), the semantic color roles, and the built-in themes are reused unchanged as
  the content and styling layer. This feature adds the interactive driver and the panel-ownership
  layer, not new widgets.
- **Opt-in first**: full-screen mode is explicitly selected (e.g. a flag/mode) for this release; the
  inline REPL remains the default and the automatic fallback. Making it the default when a capable
  terminal is detected is a later decision, out of scope here.
- **Panel granularity**: an update replaces a whole panel addressed by name. Cell-level in-place
  updates are a possible future refinement and are out of scope for this feature.
- **Local single operator**: one interactive operator at a local or remote terminal; multi-user or
  networked shared views are out of scope.
- **Runtime-free core preserved**: the driver and panel logic live in the harness layer; the
  runtime-free policy core gains no async-runtime or terminal dependency (Constitution Principle V).
- **Security posture unchanged**: this is a presentation layer; the render sandbox, policy enforcement,
  and audit behavior are untouched (Constitution Principles I–IV).
- **A prior refactor** extracting a shared session/turn core (plan milestone M2) is assumed available so
  the full-screen front-end and the inline front-end drive the same conversation engine. If not yet
  present, it is a prerequisite of this feature's implementation.

## Dependencies

- **docs/grid-tui-plan.md** — the originating plan (§3 driver, §4 model-owned panels); this spec
  formalizes milestones M3–M4.
- **The grid content model (M1)** — `RenderSpec::Grid` and the Rhai `grid()` surface — already shipped
  and depended upon here.
- **Shared session engine (M2)** — the event-driven core both front-ends consume (prerequisite; see
  Assumptions).
