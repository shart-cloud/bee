# Contract: `bee` CLI

**Feature**: 001-ebpf-agent-sandbox

The CLI is a thin wrapper over the library API (Principle V). It adds no enforcement logic of its own.

## `bee run`

```
bee run --policy <FILE> [--mode enforce|observe] [--parent-cgroup <PATH>]
        [--audit <FILE|-] [--audit-format json] -- <COMMAND> [ARGS...]
```

- `--policy <FILE>` (required): TOML policy (source of truth).
- `--mode` (default `enforce`): `observe` = audit-only / dry-run (FR-016).
- `--parent-cgroup` (default `/sys/fs/cgroup/bee`): where bee creates the scope cgroup (FR-004).
- `--audit <FILE|->` (default `-` = stderr): stream JSON audit events (FR-007).
- Everything after `--` is the sandboxed command.

**Behavior**: compile Policy → prepare Enforcement Plan → `Engine::init` → `create_scope` → spawn →
stream audit events →
exit with the child's exit status.

**Exit codes**:
| Code | Meaning |
|------|---------|
| 0 | child exited 0 (and, in enforce mode, no policy violation aborted it) |
| child status | child's own non-zero exit |
| 64 | policy parse/compile/plan error (including capabilities this backend cannot enforce) |
| 65 | unsupported kernel / fail-closed init (FR-009) — clear diagnostic to stderr |
| 66 | refused: privileged target (FR-015) |
| 70 | internal/load error |

## `bee check`

```
bee check                 # print support diagnostics and exit
```
Reports each gate from R3 (kernel version, `bpf` in `/sys/kernel/security/lsm`, cgroup v2, BTF) as
pass/fail. Non-zero exit if enforcement is unavailable. Never attaches programs.

## `bee validate`

```
bee validate --policy <FILE>                     # prove the policy is runnable by this backend
bee validate --policy <CHILD> --parent <PARENT>  # prove CHILD ⊆ PARENT and runnable
```
Exits 0 only after parsing, optional attenuation, backend-neutral compilation, and eBPF enforcement
planning all succeed. Exits 64 with a specific error otherwise. No privileges are required.

## Global flags
`-v/--verbose`, `--version`, `--help`. Human-readable to stderr, machine output (audit JSON) to the
chosen sink — text-in/text-out discipline.
