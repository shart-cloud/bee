# Bee application and workspace simplification

Working design following [ADR-0002](adr/0002-present-bee-as-one-user-facing-application.md).

## Outcome

Bee should present one installed host executable and one session model. Its two primary user journeys are:

- `bee run`: run the harness headlessly for sandbox testing.
- `bee repl`: interact with the harness, with enforcement enabled by default or explicitly disabled by the operator.

Running commands such as `cargo test` is an agent action inside one of these sessions. A raw host-process runner is not a primary user journey.

## Current spread

The repository currently has seven package directories and four installed host-facing binaries.

| Package | Current executable or role | Direction |
| --- | --- | --- |
| `bee-cli` | `bee`: check, validate, and raw `run -- COMMAND` | Merge into the application package |
| `bee-harness` | `bee-episode`, `bee-repl`, and `bee-metrics` | Become the application package and sole host executable |
| `bee-hardening` | Userspace hardening helpers | Move into `bee-userspace` |
| `bee-core` | Runtime-free policy, compilation, and attenuation | Keep separate |
| `bee-userspace` | Kernel enforcement adapter | Keep separate |
| `bee-common` | Shared `no_std` layouts and matching | Keep separate |
| `bee-ebpf` | BPF-target program | Keep separate from host packages |

The `bee-ebpf` executable target is an internal BPF artifact, not another user-facing command. Its artifact name can be clarified independently of the host command consolidation.

## Target command interface

```text
bee run       # headless harness session
bee repl      # interactive harness session
bee <support> # validation, diagnostics, and metrics
```

Headless versus interactive is a presentation choice over the same session behavior. Single, batch, and concurrent execution are run cardinality or scheduling choices, not reasons for separate executables. Inline and TUI presentation belong within the REPL rather than separate executables.

The exact names and nesting of support commands remain open. If the raw process runner is retained for engineering diagnostics, it should be clearly secondary rather than occupying `bee run`.

## Configuration and enforcement invariant

Both primary commands resolve the same effective configuration. Inputs may include explicit arguments, an explicitly supplied file, project configuration under `.bee/`, and user configuration under `~/.bee/`; no individual file is mandatory.

Before contacting a provider or executing a tool, Bee must either produce a complete effective configuration or report the missing requirements. Project configuration may request authority only within the protected ceiling established by user configuration. Bee must not silently fall back to unenforced execution; a host mode must require loud, explicit operator intent.

This makes the normal journey:

```text
operator configuration + project request + command intent
                         |
                         v
              effective configuration
                         |
                         v
             headless run or interactive REPL
                         |
                         v
             agent tools execute under enforcement
```

## Target workspace

The target is five packages with one host application:

```text
Cargo.toml
src/                 # bee application and sole host executable
crates/
  core/              # bee-core
  userspace/         # bee-userspace, including hardening
  common/            # bee-common
  ebpf/              # bee-ebpf
```

The directory move under `crates/` is organizational, not required for command consolidation, and can occur later to avoid combining mechanical churn with behavior changes.

## Migration sequence

1. Introduce the consolidated `bee` command adapter and shared effective-configuration resolver while retaining old executables temporarily.
2. Move `bee-episode` behavior behind `bee run`, preserving headless behavior with command-level parity tests.
3. Move `bee-repl` behavior behind `bee repl`, using the same session construction and enforcement path as `bee run`.
4. Fold metrics, policy checking, and validation into supporting subcommands.
5. Update VM scripts and documentation, then remove `bee-episode`, `bee-repl`, and `bee-metrics` executable targets.
6. Absorb `bee-hardening` into a focused hardening module in `bee-userspace`.
7. Reorganize the remaining packages only after the command interface and session path are stable.

Verification should include workspace tests, command-parity tests for the migrated journeys, fail-closed configuration tests, explicit host-mode tests, and the existing live-VM enforcement matrix.

## Open decisions

All seven are now resolved. See the decision table in
[`.scratch/application-consolidation/PRD.md`](../.scratch/application-consolidation/PRD.md), which
also carries the migration sequence above as eight independently-landable issues.

One resolution supersedes what is written elsewhere: user configuration lives at
`~/.config/bee/config.toml`, not `~/.bee/`, matching the XDG paths the code already ships. ADR-0001
and `CONTEXT.md` are amended accordingly as part of that work.
