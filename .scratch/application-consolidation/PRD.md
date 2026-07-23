# Bee application and workspace consolidation

Status: ready-for-agent

Implements [ADR-0002](../../docs/adr/0002-present-bee-as-one-user-facing-application.md) and the
working design in [docs/application-and-workspace-simplification.md](../../docs/application-and-workspace-simplification.md).

## Problem Statement

Bee presents itself as four installed host commands across seven packages. An operator who installs
bee gets `bee`, `bee-episode`, `bee-repl`, and `bee-metrics`, and has to learn which of them is the
product and which are development artifacts. The answer is not obvious from the names: `bee run`
today runs a single host command in a scope — a diagnostic — while the two journeys that actually
define the product, a headless harness session and an interactive one, live in binaries whose names
read like internals.

The spread is not only presentational. `bee-episode` and `bee-repl` each build a session from
scratch. Both set the process non-dumpable, resolve and install a theme, load a provider TOML,
read the key from its named environment variable, construct a model, build a tool registry, and
construct a sandbox — in two places, in two orders, with two sets of error strings. `bee-repl`
additionally owns skills discovery, capability-grant resolution, and MCP bridge wiring that a
headless session has every reason to want and cannot currently reach. When the two paths drift, the
enforcement behaviour of a headless run and an interactive run drift with them, and nothing in the
test suite notices.

Configuration is spread the same way. The theme resolves from a flag, then an environment variable,
then a config file. The visual level resolves from a flag, then a different environment variable.
The policy, the ceiling policy, the provider, the tool list, and the MCP configuration each resolve
from their own flag and nowhere else. There is no single point at which bee can say "this is the
complete configuration for this session" or "this is what is missing" — so there is no point at
which it can refuse to start. Today an interactive session with no policy silently becomes a host
session with no kernel scope, announced only by a line in the startup banner.

The desired outcome is one installed executable, one session model, one configuration resolution,
and one enforcement decision, with the diagnostic runner clearly secondary.

## Target command interface

```text
bee                # prints help
bee run            # headless harness session
bee repl           # interactive harness session
bee exec           # run one host command in a scope (diagnostic, secondary)
bee check          # kernel support gates
bee validate       # compile + plan a policy, or check attenuation
bee metrics        # report recorded LLM usage, cost, and latency
```

Headless versus interactive is a presentation choice over the same session behaviour. Single, batch,
and concurrent execution are run-cardinality choices, not reasons for separate executables. Inline
versus full-screen presentation belongs within `bee repl`.

## Decisions

These were the open decisions in the working design. All are now settled; the design doc's "Open
decisions" section is superseded by this table.

| Decision | Resolution |
| --- | --- |
| Raw process runner | Becomes `bee exec --policy P -- CMD`. Visible and documented, but described as a diagnostic rather than a primary journey. It is load-bearing: the live VM enforcement matrix drives nearly every case through it, harness-free, and that harness-free signal is worth keeping. |
| Configuration paths | XDG wins. User configuration is `~/.config/bee/config.toml`, project configuration is `.bee/config.toml`, metrics state stays under `~/.local/state/bee/`. ADR-0001 and `CONTEXT.md` are amended to name these paths instead of `~/.bee/`. |
| Support command names | Flat, no nesting: `bee check`, `bee validate`, `bee metrics`, `bee exec`. |
| Bare `bee` | Prints help. Starting a session is always explicit. |
| Compatibility shims | None. The workspace is `0.1.0` and unreleased; each issue updates its own call sites in the same commit. |
| `--config` | Participates in resolution as the highest-precedence file source. Explicit flags still outrank it. |
| `crates/` move | Lands last, on its own, so mechanical churn stays out of the behaviour changes. |

## Configuration and enforcement invariant

Both primary commands resolve the same effective configuration from the same sources, in this order
of precedence:

```text
explicit flags  >  --config <file>  >  .bee/config.toml  >  ~/.config/bee/config.toml
```

No individual file is mandatory. Before contacting a provider or executing a tool, bee either
produces a complete effective configuration or reports what is missing.

Project configuration may request authority only within the ceiling established by user
configuration. That check is not new machinery: it is `Policy::derive` in `bee-core`, the same
attenuation validator that already backs subagent policies and skill capability grants.

Bee must not silently fall back to unenforced execution. Host mode requires loud, explicit operator
intent, and its absence is a startup error naming the missing requirement rather than a banner line.

## Target workspace

Five packages, one host application:

```text
Cargo.toml           # workspace root and the `bee` application package
src/                 # the application: commands, session construction, harness library
crates/
  core/              # bee-core
  userspace/         # bee-userspace, including hardening
  common/            # bee-common
  ebpf/              # bee-ebpf
```

`bee-core`, `bee-userspace`, `bee-common`, and `bee-ebpf` stay separate because their runtime,
kernel, `no_std`, and BPF-target constraints provide real leverage. `bee-ebpf` remains excluded from
the workspace and built out of band by `bee-userspace`'s build script.

## Issues

Ordered. Each is independently landable, and each leaves the tree green.

| # | Issue | Migration step |
| --- | --- | --- |
| 01 | [The `bee` application package and `bee exec`](issues/01-bee-application-package.md) | 1 (command adapter), plus the raw-runner decision |
| 02 | [Effective configuration resolver](issues/02-effective-configuration-resolver.md) | 1 (shared resolver) |
| 03 | [`bee run` — the headless session](issues/03-bee-run-headless.md) | 2 |
| 04 | [`bee repl` — the interactive session](issues/04-bee-repl-interactive.md) | 3 |
| 05 | [Fail-closed configuration and explicit host mode](issues/05-fail-closed-and-explicit-host-mode.md) | the enforcement invariant |
| 06 | [Retire the old binaries](issues/06-retire-old-binaries.md) | 4, 5 |
| 07 | [Absorb `bee-hardening` into `bee-userspace`](issues/07-absorb-bee-hardening.md) | 6 |
| 08 | [Workspace reorganisation](issues/08-workspace-reorg.md) | 7 |

Issue 01 must precede everything else: it moves the raw runner off the `run` name, so nothing else
has to work around `run` meaning two things.

## Verification bar

Every issue carries its own verification section. The standing bar across all of them:

- `cargo test` and `cargo clippy --workspace --all-targets` stay green.
- Command-parity tests for each migrated journey: identical inputs through the old binary and the
  new subcommand produce identical output and exit codes.
- The dependency-boundary guards (`core_deps_guard`, `mcp_dep_boundary`) keep passing — the
  consolidation must not push terminal, runtime, or MCP dependencies into `bee-core` or
  `bee-common`.
- Anything touching enforcement runs the live VM matrix (`test/vm/matrix.sh`).
