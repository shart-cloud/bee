# Static Vulnerability Findings: bee

Static candidates only; none have been execution-verified or rigorously triaged.

## Summary

| id | severity | confidence | category | file:line | title |
|---|---|---:|---|---|---|
| F-001 | HIGH | 1.0 | capability-widening | bee-core/src/attenuation.rs:62 | Removing all child write grants disables the parent's default-deny write boundary |
| F-002 | HIGH | 1.0 | capability-widening | bee-core/src/attenuation.rs:159 | An empty child executable allowlist turns restricted execution into unrestricted execution |
| F-003 | HIGH | 1.0 | capability-widening | bee-core/src/attenuation.rs:177 | An empty child network allowlist disables egress enforcement |
| F-004 | HIGH | 1.0 | network-policy-bypass | bee-ebpf/src/main.rs:92 | UDP sendto bypasses the network allowlist |
| F-005 | HIGH | 1.0 | filesystem-policy-bypass | bee-ebpf/src/main.rs:142 | Path rules can be bypassed by relinking or renaming denied files |
| F-006 | HIGH | 1.0 | fail-open-enforcement | bee-ebpf/src/main.rs:172 | Unresolvable long paths fail open for file and executable policy |
| F-007 | HIGH | 1.0 | arbitrary-host-write | bee-harness/src/episode.rs:424 | Repository-controlled workdir paths are written on the trusted host before sandboxing |
| F-008 | HIGH | 1.0 | capability-grant-without-invocation | bee-harness/src/episode.rs:502 | Every discovered skill receives capability grants before any skill is invoked |
| F-009 | HIGH | 1.0 | attenuation-bypass | bee-harness/src/episode.rs:614 | Skill directories are made readable after the ceiling proof |
| F-010 | HIGH | 1.0 | cleartext-credential-exposure | bee-harness/src/mcp/bridge.rs:338 | MCP Bearer tokens are sent over unrestricted plaintext HTTP endpoints |
| F-011 | HIGH | 1.0 | ssrf-allowlist-bypass | bee-harness/src/mcp/policy.rs:78 | Custom URL parsing disagrees with the HTTP client on backslash authority boundaries |
| F-012 | HIGH | 1.0 | auth-bypass | bee-userspace/src/cgroup.rs:42 | Background descendants survive scope teardown and become unsandboxed when the engine detaches |
| F-013 | HIGH | 1.0 | exec-allowlist-bypass | bee-userspace/src/plan.rs:210 | Executable entries are encoded as subtree prefixes instead of exact paths |
| F-014 | HIGH | 1.0 | privilege-escalation | bee-userspace/src/spawn.rs:92 | Sandboxed tools inherit the privileged launcher's UID and BPF/cgroup capabilities |
| F-015 | MEDIUM | 1.0 | audit-integrity | bee-ebpf/src/main.rs:383 | Full audit ring silently discards enforcement records without marking transcripts incomplete |
| F-016 | MEDIUM | 1.0 | path-traversal | bee-harness/src/bin/bee-episode.rs:334 | Scenario identifiers escape the batch transcript output directory |
| F-017 | MEDIUM | 1.0 | audit-misattribution | bee-harness/src/episode.rs:295 | Drain-window correlation attributes unrelated or late audit events to the current tool call |
| F-018 | MEDIUM | 1.0 | toctou | bee-userspace/src/spawn.rs:75 | Privileged-executable refusal races pathname replacement before exec |
| F-019 | HIGH | 0.9 | secret-exfiltration | bee-harness/src/config.rs:79 | Provider configuration can select any host environment secret and send it to an arbitrary endpoint |
| F-020 | HIGH | 0.9 | credential-boundary-bypass | bee-harness/src/mcp/transport.rs:42 | Stdio MCP servers inherit host credentials outside a small name-based denylist |
| F-021 | HIGH | 0.9 | symlink-toctou-host-read | bee-harness/src/skills.rs:109 | Lazy skill-body reads can be redirected to arbitrary host files after discovery |
| F-022 | HIGH | 0.9 | tool-authority-bypass | bee-harness/src/skills/grant.rs:142 | Tool grants are not bounded by the capability ceiling |
| F-023 | MEDIUM | 0.9 | algorithmic-complexity | bee-harness/src/render_api.rs:486 | Rhai layout cloning permits exponential in-process memory amplification before validation |
| F-024 | MEDIUM | 0.9 | cgroup-policy-confusion | bee-userspace/src/lib.rs:223 | Scope teardown leaves BPF rules keyed by reusable cgroup inode IDs |
| F-025 | LOW | 0.9 | audit-misattribution | bee-harness/src/concurrent.rs:114 | Every concurrent episode's audit records carry the same false scope identifier |
| F-026 | HIGH | 0.8 | sandbox-bypass | bee-harness/src/episode.rs:523 | Default non-enforcement builds execute model-requested tools directly on the host |
| F-027 | MEDIUM | 0.8 | exec-identity-toctou | bee-core/src/compiler.rs:108 | Executable authorization is bound only to a mutable path |

Totals: **27** findings — **19 high**, **7 medium**, **1 low**, **0 low-confidence** — across **8 focus areas** and **95 source files**.

## Findings

### F-001 — Removing all child write grants disables the parent's default-deny write boundary

- Severity: HIGH; confidence: 1.0; category: capability-widening
- Location: `bee-core/src/attenuation.rs:62`
- Description: `check_filesystem` validates only child entries, so an empty map passes. Planning arms write-default-deny only when the child retains a write rule; the kernel otherwise permits unmatched writes.
- Exploit: A parent grants write only to `/workspace`; an empty child map derives successfully and can write other same-UID paths unless explicitly denied.
- Recommendation: Represent filesystem default behavior explicitly and preserve deny-all during attenuation even with zero child write grants.
- Confidence reason: The widening is explicit end to end across attenuation, planning, and kernel decision code.

### F-002 — An empty child executable allowlist turns restricted execution into unrestricted execution

- Severity: HIGH; confidence: 1.0; category: capability-widening
- Location: `bee-core/src/attenuation.rs:159`
- Description: An empty child exec list passes; planning omits `EXEC_ALLOW`; the LSM permits all execution when that map entry is absent.
- Exploit: A parent allowing only cargo/rustc derives an empty child that can execute bash, curl, or any accessible binary.
- Recommendation: Separate exec-policy activation from list size and make an empty enforced list mean deny-all.
- Confidence reason: Missing map state is explicitly interpreted as unrestricted execution.

### F-003 — An empty child network allowlist disables egress enforcement

- Severity: HIGH; confidence: 1.0; category: capability-widening
- Location: `bee-core/src/attenuation.rs:177`
- Description: An empty child network list passes; planning omits `FLAG_NET_ENFORCED`; `socket_connect` then allows all destinations.
- Exploit: A parent restricted to `crates.io:443` derives a child with unrestricted IPv4/IPv6 egress.
- Recommendation: Model network activation independently and enforce deny-all for an empty active list.
- Confidence reason: The vacuous attenuation and flag-controlled allow path are explicit.

### F-004 — UDP sendto bypasses the network allowlist

- Severity: HIGH; confidence: 1.0; category: network-policy-bypass
- Location: `bee-ebpf/src/main.rs:92`
- Description: Network enforcement attaches only to `socket_connect`; unconnected UDP `sendto`/`sendmsg` never reaches `NET_ALLOW`.
- Exploit: A sandboxed process sends data or DNS queries directly to a disallowed destination over UDP.
- Recommendation: Enforce `socket_sendmsg` or equivalent for unconnected datagrams.
- Confidence reason: The relevant syscall uses an unhooked LSM path.

### F-005 — Path rules can be bypassed by relinking or renaming denied files

- Severity: HIGH; confidence: 1.0; category: filesystem-policy-bypass
- Location: `bee-ebpf/src/main.rs:142`
- Description: Only `file_open` is enforced by resolved pathname; link/rename/unlink/truncate/setattr and inode identity are uncovered.
- Exploit: A same-UID process aliases a protected inode beneath an allowed project path and opens it.
- Recommendation: Enforce metadata hooks and bind sensitive policy to stable inode/file-handle identity.
- Confidence reason: Alias creation and non-open mutation never encounter the file-open path check.

### F-006 — Unresolvable long paths fail open for file and executable policy

- Severity: HIGH; confidence: 1.0; category: fail-open-enforcement
- Location: `bee-ebpf/src/main.rs:172`
- Description: File and exec hooks allow on `bpf_d_path` failure; dirfd-relative operations can create accessible paths longer than the fixed 4096-byte buffer.
- Exploit: A write or executable under an over-`PATH_MAX` tree bypasses policy when rendering fails.
- Recommendation: Deny path-resolution failures or use identity-based enforcement.
- Confidence reason: Both hooks explicitly allow errors and the triggering path form is reachable.

### F-007 — Repository-controlled workdir paths are written on the trusted host before sandboxing

- Severity: HIGH; confidence: 1.0; category: arbitrary-host-write
- Location: `bee-harness/src/episode.rs:424`
- Description: Scenario directory/file/flag paths flow unchanged to host `create_dir_all` and `write` before sandbox creation.
- Exploit: A trojan scenario writes an SSH authorized key or traverses into a host autostart/config path.
- Recommendation: Confine materialization beneath a dedicated root with no-follow descriptor-relative creation.
- Confidence reason: No absolute, traversal, containment, or symlink checks occur before the trusted writes.

### F-008 — Every discovered skill receives capability grants before any skill is invoked

- Severity: HIGH; confidence: 1.0; category: capability-grant-without-invocation
- Location: `bee-harness/src/episode.rs:502`
- Description: Startup resolves grants for all discovered skills, including hidden/unselected and project-shadowing skills, using automatic within-ceiling approval.
- Exploit: Merely opening a trojan repository widens the scope for a never-invoked hidden skill.
- Recommendation: Grant only the explicitly launch-selected or dynamically invoked skill with per-invocation authorization.
- Confidence reason: The startup flow and tests show authority exists before invocation.

### F-009 — Skill directories are made readable after the ceiling proof

- Severity: HIGH; confidence: 1.0; category: attenuation-bypass
- Location: `bee-harness/src/episode.rs:614`
- Description: Episode/REPL add skill-directory read grants after `ceiling.derive`, without a second proof; specific grants can override broad denies.
- Exploit: A skill root beneath a denied private tree becomes readable despite the ceiling.
- Recommendation: Add required skill reads before attenuation and compile only the exact proven policy.
- Confidence reason: The post-proof mutation directly violates the active-policy subset invariant.

### F-010 — MCP Bearer tokens are sent over unrestricted plaintext HTTP endpoints

- Severity: HIGH; confidence: 1.0; category: cleartext-credential-exposure
- Location: `bee-harness/src/mcp/bridge.rs:338`
- Description: Domain gating ignores scheme and the transport attaches Bearer credentials to `http://` endpoints.
- Exploit: A network-adjacent attacker captures or impersonates a plaintext allowed MCP server.
- Recommendation: Require HTTPS for credentials; permit only explicit credential-free loopback development over HTTP.
- Confidence reason: Scheme is unchecked and credentials are attached regardless.

### F-011 — Custom URL parsing disagrees with the HTTP client on backslash authority boundaries

- Severity: HIGH; confidence: 1.0; category: ssrf-allowlist-bypass
- Location: `bee-harness/src/mcp/policy.rs:78`
- Description: The custom gate does not treat backslash like the standards HTTP parser, so each can resolve a different host.
- Exploit: A crafted URL passes as `trusted.example` while the transport connects to an attacker host and sends the token.
- Recommendation: Parse once with the transport's URL type; reject backslashes/userinfo and gate the parsed host.
- Confidence reason: The parser mismatch is deterministic.

### F-012 — Background descendants survive scope teardown and become unsandboxed when the engine detaches

- Severity: HIGH; confidence: 1.0; category: auth-bypass
- Location: `bee-userspace/src/cgroup.rs:42`
- Description: Teardown only removes an empty cgroup, ignores failure, and does not kill descendants; dropping `Engine` detaches the global hooks.
- Exploit: A redirected background child survives the direct tool, then continues after enforcement detaches.
- Recommendation: Kill/reap the cgroup, wait for empty, clean map state, and keep links alive until completion.
- Confidence reason: The entire survival and detach chain is explicit.

### F-013 — Executable entries are encoded as subtree prefixes instead of exact paths

- Severity: HIGH; confidence: 1.0; category: exec-allowlist-bypass
- Location: `bee-userspace/src/plan.rs:210`
- Description: Executable rules use subtree matching, so an allowlisted path also authorizes descendants.
- Exploit: Replace writable `/project/tool` with a directory and execute `/project/tool/payload`.
- Recommendation: Use exact-path matching and stable identity; reject writable executable locations until then.
- Confidence reason: Encoding and matcher semantics directly authorize descendants.

### F-014 — Sandboxed tools inherit the privileged launcher's UID and BPF/cgroup capabilities

- Severity: HIGH; confidence: 1.0; category: privilege-escalation
- Location: `bee-userspace/src/spawn.rs:92`
- Description: `pre_exec` hardens and joins the cgroup but does not drop identity, groups, capabilities, or set `no_new_privs`.
- Exploit: A root-launched tool retains BPF/admin authority and weakens maps or escapes its cgroup.
- Recommendation: Use a privileged broker and dedicated unprivileged child identity with cleared capabilities.
- Confidence reason: No privilege drop exists on the documented privileged launch path.

### F-015 — Full audit ring silently discards enforcement records without marking transcripts incomplete

- Severity: MEDIUM; confidence: 1.0; category: audit-integrity
- Location: `bee-ebpf/src/main.rs:383`
- Description: Ring reservation failure is silent; synchronous runs drain only after tool exit, yet transcripts and scores assume completeness.
- Exploit: Flood denials to hide a later targeted event, especially dangerous in observe mode.
- Recommendation: Count drops, drain continuously, and invalidate evidence when loss occurs.
- Confidence reason: The ring can be filled before any consumer drain and loss has no signal.

### F-016 — Scenario identifiers escape the batch transcript output directory

- Severity: MEDIUM; confidence: 1.0; category: path-traversal
- Location: `bee-harness/src/bin/bee-episode.rs:334`
- Description: Unrestricted scenario IDs become filenames joined to the output root.
- Exploit: `../../shared/report` writes outside the chosen directory.
- Recommendation: Require a strict slug and use descriptor-relative no-follow creation.
- Confidence reason: `Path::join` traverses or discards the base with accepted IDs.

### F-017 — Drain-window correlation attributes unrelated or late audit events to the current tool call

- Severity: MEDIUM; confidence: 1.0; category: audit-misattribution
- Location: `bee-harness/src/episode.rs:295`
- Description: Scope-wide events are drained only after a call, without call IDs, pre-drain, or sequence watermark.
- Exploit: A background process's denial is attributed to a later benign tool, corrupting CTF technique scoring.
- Recommendation: Add call generations/per-call scopes and synchronized watermarks/final drain.
- Confidence reason: Scoring trusts whichever call receives the uncorrelated event.

### F-018 — Privileged-executable refusal races pathname replacement before exec

- Severity: MEDIUM; confidence: 1.0; category: toctou
- Location: `bee-userspace/src/spawn.rs:75`
- Description: Setuid/capability checks and later exec resolve the same mutable pathname separately.
- Exploit: Swap a benign writable path for a privileged target between check and exec.
- Recommendation: Check and execute one opened descriptor with safe resolution and `no_new_privs`.
- Confidence reason: The checked object is not bound to the executed inode.

### F-019 — Provider configuration can select any host environment secret and send it to an arbitrary endpoint

- Severity: HIGH; confidence: 0.9; category: secret-exfiltration
- Location: `bee-harness/src/config.rs:79`
- Description: Provider TOML controls both `api_key_env` and an OpenAI-compatible `base_url`.
- Exploit: A trojan repository selects an AWS secret and sends it as auth to an attacker endpoint.
- Recommendation: Bind trusted credential slots to approved provider origins.
- Confidence reason: The source path is explicit; exploitability depends on repository config being runnable input.

### F-020 — Stdio MCP servers inherit host credentials outside a small name-based denylist

- Severity: HIGH; confidence: 0.9; category: credential-boundary-bypass
- Location: `bee-harness/src/mcp/transport.rs:42`
- Description: MCP children inherit the parent environment except a small denylist, then receive config overlays.
- Exploit: A compromised package reads AWS/GitHub tokens or `SSH_AUTH_SOCK` on startup.
- Recommendation: `env_clear()` and positively allow only required runtime/server values.
- Confidence reason: Leakage is direct when those ambient values are present.

### F-021 — Lazy skill-body reads can be redirected to arbitrary host files after discovery

- Severity: HIGH; confidence: 0.9; category: symlink-toctou-host-read
- Location: `bee-harness/src/skills.rs:109`
- Description: Discovery stores a mutable path; invocation later performs unrestricted host-side `read_to_string` without identity or containment checks.
- Exploit: Replace `SKILL.md` with a symlink to an SSH key, invoke it, and receive the secret as tool output.
- Recommendation: Reject symlinks, open beneath root descriptors, bind identity, or eagerly load immutable bodies.
- Confidence reason: The read oracle is direct once the path can be replaced.

### F-022 — Tool grants are not bounded by the capability ceiling

- Severity: HIGH; confidence: 0.9; category: tool-authority-bypass
- Location: `bee-harness/src/skills/grant.rs:142`
- Description: Ceiling derivation covers filesystem policy but not the requested tool set; a tool-only request always leaves the candidate policy valid.
- Exploit: A project skill adds `bash` and `write_file` to a read-only scenario at startup.
- Recommendation: Include allowed tools in the trusted authority ceiling and require explicit authorization.
- Confidence reason: Tool membership never enters the proof and is automatically registered.

### F-023 — Rhai layout cloning permits exponential in-process memory amplification before validation

- Severity: MEDIUM; confidence: 0.9; category: algorithmic-complexity
- Location: `bee-harness/src/render_api.rs:486`
- Description: Builder `add` deep-clones recursive layouts before final validation; multiple node types are omitted from structural cost.
- Exploit: Shallow repeated clones amplify to millions of Rust objects within script operation limits.
- Recommendation: Enforce an incremental checked expanded-node/allocation budget before cloning.
- Confidence reason: The amplification is visible; exact Rhai argument-copy behavior still merits execution verification.

### F-024 — Scope teardown leaves BPF rules keyed by reusable cgroup inode IDs

- Severity: MEDIUM; confidence: 0.9; category: cgroup-policy-confusion
- Location: `bee-userspace/src/lib.rs:223`
- Description: Teardown deletes only the cgroup directory; all per-ID map entries remain.
- Exploit: A reused cgroup ID inherits stale network destination keys from an older scope.
- Recommendation: Delete all installed keys transactionally before cgroup removal and roll back partial creation.
- Confidence reason: Stale state is definite; practical exploitation depends on ID reuse.

### F-025 — Every concurrent episode's audit records carry the same false scope identifier

- Severity: LOW; confidence: 0.9; category: audit-misattribution
- Location: `bee-harness/src/concurrent.rs:114`
- Description: One global decoder stamps literal `bee-concurrent`; cgroup demux routes correctly but never repairs the label.
- Exploit: Exported evidence cannot reliably associate human-readable scope IDs with concurrent runs.
- Recommendation: Stamp labels from a cgroup-to-scope mapping or use numeric IDs exclusively.
- Confidence reason: Persisted labels are definitely false, though isolation routing remains correct.

### F-026 — Default non-enforcement builds execute model-requested tools directly on the host

- Severity: HIGH; confidence: 0.8; category: sandbox-bypass
- Location: `bee-harness/src/episode.rs:523`
- Description: Without the feature, real-model scenarios construct `Sandbox::Host`; process/file tools ignore scenario policy.
- Exploit: A prompt-injected model reads host secrets, alters the checkout, or makes arbitrary network requests.
- Recommendation: Fail closed without enforcement and require a conspicuous explicit unsafe opt-in.
- Confidence reason: Behavior is documented, but the runtime provides no refusal or explicit unsafe-mode gate.

### F-027 — Executable authorization is bound only to a mutable path

- Severity: MEDIUM; confidence: 0.8; category: exec-identity-toctou
- Location: `bee-core/src/compiler.rs:108`
- Description: Compiler/planner/kernel retain only mutable pathname identity; inode pinning is rejected.
- Exploit: Replace a write-accessible allowlisted tool and execute attacker code under the approved name.
- Recommendation: Bind device/inode/file-handle identity or reject writable allowlisted locations.
- Confidence reason: Real path replacement requires overlapping write authority.
