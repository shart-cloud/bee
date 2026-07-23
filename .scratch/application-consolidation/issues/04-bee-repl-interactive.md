# `bee repl` — the interactive session

Status: ready-for-agent

Part of [Bee application and workspace consolidation](../PRD.md).

## Problem Statement

`bee-repl` is the second primary journey and the larger of the two binaries. Beyond the startup
sequence issue 03 extracted, it owns four things a headless session has a legitimate claim to and
currently cannot reach: skills discovery across the project and user skill roots, interactive
resolution of skill capability grants against a ceiling, MCP server connection with the per-turn
tool-refresh hook, and sandbox teardown ordered so the MCP children die inside the scope before the
scope itself goes away.

That last ordering is the kind of detail that gets lost in a rewrite. The bridge holds the stdio
children, the refresh hook holds a clone of the bridge, and the hook lives inside the session
config — so the config is dropped, then the bridge, then the scope is torn down. Nothing in the type
system enforces that sequence; it is held by a comment. Moving this code means moving it, not
reimplementing it.

Presentation is the part that genuinely belongs to the REPL alone. Inline versus full-screen is
chosen from an explicit request plus terminal capability — a pipe, `TERM=dumb`, or a terminal below
the hard floor all fall back to inline with a one-line note, so the full-screen request never
silently does nothing. That decision stays inside `bee repl` and does not rise into shared session
construction.

## Solution

Add `bee repl` on the session construction from issue 03, and move the four interactive concerns
into it: skills discovery and warnings, grant resolution with its interactive consent prompt, MCP
bridge connection and tool refresh, and the ordered teardown.

Place each concern by whether a headless session could want it. Skills discovery and grant
resolution are shared: a headless run has scenarios that already carry a skills list, and the grant
resolution is the same attenuation-bounded operation in both. What differs is the consent sink —
interactive prompting in the REPL, and in headless runs the scenario's declared grants with no
prompt — so the consent sink becomes the injected part rather than the whole path being duplicated.
MCP bridge wiring is shared too; the harness already carries MCP configuration on a scenario. Only
the front-end choice, the startup banner, and the transcript-on-exit behaviour stay REPL-local.

Carry the whole flag surface: provider, policy, ceiling policy, system prompt override, tool list,
turn budget, transcript save path, mascot suppression, theme, MCP config, full-screen request and
its override, visual level, and animation suppression. Each of these now has a configuration-file
equivalent through issue 02's resolver, with the flag still winning.

Preserve the two startup-failure asymmetries exactly as they are, because they encode a deliberate
distinction: an unrecognised theme warns and falls back, an unrecognised visual level fails startup.
The visual level is a ceiling on how much screen the agent may claim, and falling back to a default
would risk falling back to something wider than the operator asked for. The theme is decoration.

`bee-repl` stays installed and unchanged through this issue, for the parity tests. Issue 06 removes
it.

## Commits

1. **Move skills discovery and grant resolution into shared construction.** Lift both out of
   `bee-repl`'s binary, with the consent sink as an injected dependency so the interactive prompt is
   the REPL's contribution rather than a fixed part of the path. Keep deny-by-default: any non-`yes`
   answer, including end-of-input, refuses. Keep the readable-scope widening that makes each
   discovered skill directory readable so bundled resources resolve.

2. **Move MCP bridge wiring into shared construction.** Lift the connection, tool registration, the
   per-turn refresh hook, and the drop ordering. The ordering is a correctness property, so add a
   test that asserts the children are gone before the scope is torn down rather than trusting the
   comment.

3. **Add `bee repl` with the inline front-end.** Wire the full flag surface onto the resolver and
   the shared construction, plus the startup banner and the transcript-on-exit save. Inline only, so
   this commit is reviewable without the terminal feature.

4. **Add the full-screen front-end.** Wire the front-end choice behind the `tui` feature, including
   the capability-based fallback and its one-line note, and the behaviour without the feature.
   Preserve that the full-screen path currently produces no transcript, so the save flag writes
   nothing there — port the limitation with its explanation rather than silently changing it.

5. **Add parity tests.** Drive scripted sessions through `bee-repl` and `bee repl` with a mock
   provider and assert identical output and exit codes: a plain session, a session with a skill
   requesting capabilities (both granted and refused), a session with MCP configured, and the
   transcript-save path. Extend the existing front-end fallback and terminal-restore tests to cover
   the new command.

## Verification

- `cargo test` and `cargo clippy --workspace --all-targets` green, and with `--features tui` and
  `--features mcp`.
- Parity tests pass — `bee repl` and `bee-repl` indistinguishable on identical scripted input.
- Manual check on a real terminal: `bee repl --tui` against a mock provider enters full-screen and
  restores the terminal cleanly on quit and on panic; piping the same command falls back to inline
  with the note. The terminal-restore behaviour is the one thing an automated test proves poorly.
- The live VM matrix (`test/vm/matrix.sh`) passes.
