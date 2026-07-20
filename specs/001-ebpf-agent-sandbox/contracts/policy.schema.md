# Contract: Policy TOML Schema

**Feature**: 001-ebpf-agent-sandbox · Source of truth for a scope's capabilities (Principle IV).

## Example

```toml
[policy]
name = "cargo-test"
description = "Run cargo test in a project directory"
mode = "enforce"                 # or "observe" (audit-only / dry-run)

[policy.filesystem]
# Access modes: "read" | "write" | "deny". Most-specific match wins; deny beats grant at a tie.
# Pattern shapes the language lowers: exact path, subtree (dir), "*.ext" (postfix),
# "**/name" (segment), single-segment "*" (bounded). Irreducible mid-path "**" => compile error.
# NOTE: segment ("**/name") and bounded-star filesystem rules are not yet enforceable in-kernel and
# are refused during enforcement planning for every mode (fail-closed); use subtree or "*.ext" today.
":project_root"        = "write"
":project_root/.git"   = "read"
":project_root/.bee"   = "deny"
"~/.ssh"               = "deny"
"~/.aws"               = "deny"
"~/.cargo/registry"    = "read"
"*.log"                = "write"
"/tmp"                 = "write"

[policy.exec]
# Names resolve to absolute paths via PATH at compile time. Prefix "!" expresses inode-pinning intent,
# but the current eBPF backend rejects it until inode enforcement is implemented.
allow = ["cargo", "rustc", "cc", "ld", "ar", "git"]

[policy.network]
# host:port. host = domain | ip | cidr. deny-all is implicit once allow is non-empty.
allow = ["crates.io:443", "static.crates.io:443", "index.crates.io:443"]

[policy.exfiltration]                       # metadata only while disabled; enabled is rejected
enabled = false
sensitive_paths = ["~/.ssh", "~/.aws", "~/.gnupg", "~/.config/gcloud"]
```

## Rules & validation
- `[policy].name` required, non-empty. `mode` defaults to `enforce`.
- **Deny-by-default** (exec/network): any exec/host not granted is denied. No `allow` ⇒ nothing permitted.
- **Filesystem access modes** (`read` | `write` | `deny`): most-specific match wins; `deny` beats a
  grant at equal specificity, and postfix (`*.ext`) rules outrank subtree rules. The default when **no
  rule matches** an open (research R13):
  - **Reads** are allowed unless an explicit `deny` matches (allow-by-default — a process must be able
    to load its interpreter/libraries and read `/etc`, `/proc`, …).
  - **Writes** to a `read`-only or `deny` path are always blocked. Writes to any *other* path are
    **denied by default when the policy declares at least one `write` grant** (the scope then manages
    its writable surface); a policy that declares no `write` grant does not restrict writes beyond its
    explicit `deny`/`read` rules. So an explicitly `read`-marked tree is *always* write-blocked (US2
    AS-2), independent of whether the policy has other writable roots.
  - Segment (`**/name`) and single-`*` glob **filesystem** rules are not enforceable in-kernel and are
    refused (fail-closed) for every mode — see compile/scope errors below.
- **Protected defaults (FR-008)**: within any writable root, VCS metadata (`.git`), bee's own config
  (`.bee`), `~/.ssh`, and `~/.aws` are read-only/denied unless a rule explicitly grants otherwise.
- **Path tokens**: `:project_root` and `~` resolve to absolute paths at compile time.
- **Compile errors** (exit 64): unknown mode, malformed host:port, unresolvable exec name, and
  **unsupported glob** (irreducible `**`) — all fail-closed.
- **Enforcement-plan errors** (exit 64): segment/bounded-star rules, inode pinning, enabled
  exfiltration, overlong encoded rules, or backend capacity overflow. Planning completes before
  Engine initialization or cgroup creation.
- **Attenuation**: a derived policy is valid only if every grant is provably ⊆ its parent (FR-005);
  glob-derived grants that cannot be proven contained are rejected (conservative fail-closed).
