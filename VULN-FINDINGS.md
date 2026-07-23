# Static Vulnerability Candidates

Target: `/home/jg/git/bee`  
Scanned: 2026-07-23  
Scope: 140 Rust/JavaScript source files across 10 focus areas  
Summary: 53 candidates — 31 high, 20 medium, 2 low; 2 below 0.4 confidence

These are static candidates, not verified vulnerabilities.

| id | severity | confidence | category | file:line | title |
|---|---:|---:|---|---|---|
| F-001 | HIGH | 1.0 | attenuation-bypass | crates/core/src/attenuation.rs:63 | Child policies can omit parent deny regions and regain default-allowed reads |
| F-002 | HIGH | 1.0 | attenuation-bypass | crates/core/src/compiler.rs:86 | A child policy can override protected defaults absent from the attenuation ceiling |
| F-003 | HIGH | 1.0 | network-policy-bypass | crates/ebpf/src/main.rs:92 | UDP sendto bypasses the connect-only egress allowlist |
| F-004 | HIGH | 1.0 | network-policy-bypass | crates/ebpf/src/main.rs:99 | Empty network allowlist disables all egress enforcement |
| F-005 | HIGH | 1.0 | filesystem-policy-bypass | crates/ebpf/src/main.rs:142 | File-open-only mediation permits metadata mutation and hard-link path aliasing |
| F-006 | HIGH | 1.0 | exec-policy-bypass | crates/ebpf/src/main.rs:201 | Empty executable allowlist disables execution enforcement |
| F-007 | HIGH | 1.0 | exec-policy-bypass | crates/ebpf/src/main.rs:224 | Attacker-triggerable path-resolution failure fails open |
| F-008 | HIGH | 1.0 | filesystem-policy-bypass | crates/ebpf/src/main.rs:331 | Zero-capability filesystem policy defaults to broad read/write access |
| F-009 | HIGH | 1.0 | filesystem-policy-bypass | crates/ebpf/src/main.rs:363 | Kernel subtree matcher mishandles root and trailing-slash rules |
| F-010 | HIGH | 1.0 | privilege-escalation | crates/userspace/src/hardening.rs:29 | Tool children retain launcher privileges and can execute privileged descendants |
| F-011 | HIGH | 1.0 | incomplete-cleanup | crates/userspace/src/lib.rs:332 | Background descendants survive teardown and become unrestricted |
| F-012 | HIGH | 1.0 | network-policy-bypass | crates/userspace/src/plan.rs:45 | An empty child network list disables the parent egress allowlist |
| F-013 | HIGH | 1.0 | capability-widening | crates/userspace/src/plan.rs:48 | Removing the final child write grant disables default-deny writes |
| F-014 | HIGH | 1.0 | exec-allowlist-bypass | crates/userspace/src/plan.rs:63 | An empty child executable list turns a restricted parent into unrestricted execution |
| F-015 | HIGH | 1.0 | privilege-retention | crates/userspace/src/spawn.rs:92 | Privileged children can migrate outside the exact cgroup policy key |
| F-016 | HIGH | 1.0 | auth-bypass | src/app/config/mod.rs:364 | Untrusted project config becomes an unbounded execution policy without a user ceiling |
| F-017 | HIGH | 1.0 | attenuation-bypass | src/app/session.rs:152 | Discovered skill directories are made readable after attenuation |
| F-018 | HIGH | 1.0 | credential-exposure | src/batch.rs:133 | Provider TOML can send an arbitrary environment secret to an attacker endpoint |
| F-019 | HIGH | 1.0 | path-traversal | src/episode.rs:520 | Scenario workdir setup performs host writes before sandbox creation |
| F-020 | HIGH | 1.0 | path-traversal | src/episode.rs:579 | Repository-controlled workdir paths permit arbitrary host overwrite before sandboxing |
| F-021 | HIGH | 1.0 | auth-bypass | src/skills/grant.rs:161 | Tool grants bypass the attenuation ceiling and scenario tool allowlist |
| F-022 | HIGH | 1.0 | sandbox-bypass | src/tools/exec.rs:38 | Background descendants survive tool deadlines and outlive enforcement |
| F-023 | MEDIUM | 1.0 | path-traversal | src/app/run.rs:407 | Scenario ID escapes the batch transcript output directory |
| F-024 | MEDIUM | 1.0 | path-traversal | src/app/run.rs:408 | Repository-controlled scenario IDs escape batch output containment |
| F-025 | HIGH | 0.9 | executable-identity-bypass | crates/core/src/attenuation.rs:161 | A child can remove an executable inode-pin requirement |
| F-026 | HIGH | 0.9 | auth-bypass | crates/ebpf/src/main.rs:95 | Exact cgroup-ID lookup lets migrated processes leave enforcement |
| F-027 | HIGH | 0.9 | sandbox-bypass | crates/ebpf/src/main.rs:126 | Unix-domain sockets bypass the network policy |
| F-028 | HIGH | 0.9 | privileged-target-bypass | crates/userspace/src/spawn.rs:74 | Privileged-target refusal checks only the first executable |
| F-029 | HIGH | 0.9 | consent-bypass | src/episode.rs:599 | Headless episodes grant every discovered skill before invocation |
| F-030 | HIGH | 0.9 | sensitive-data-exposure | src/sandbox.rs:20 | Credential stripping denylist exposes ambient secrets to model tools |
| F-031 | HIGH | 0.9 | prompt-injection | src/tools/skill.rs:48 | Unselected repository skills inject instructions into every tool schema |
| F-032 | MEDIUM | 0.9 | attenuation-bypass | crates/core/src/attenuation.rs:57 | A child can disable the parent exfiltration controls |
| F-033 | MEDIUM | 0.9 | policy-validation-bypass | crates/core/src/policy.rs:104 | Unknown policy fields are silently ignored |
| F-034 | MEDIUM | 0.9 | network-policy-bypass | crates/ebpf/src/main.rs:126 | Network-enforced scopes allow every non-IP socket family |
| F-035 | MEDIUM | 0.9 | authorization-state-desynchronization | crates/userspace/src/lib.rs:200 | Failed scope reload can retain kernel grants after userspace rollback |
| F-036 | MEDIUM | 0.9 | audit-loss | src/episode.rs:334 | Fixed-delay audit draining can lose or misattribute records |
| F-037 | MEDIUM | 0.9 | audit-misattribution | src/episode.rs:386 | Reactive retry removes the triggering denial from call evidence |
| F-038 | MEDIUM | 0.9 | destination-validation-bypass | src/mcp/policy.rs:78 | Hand-written URL parsing can authorize a different host than the transport |
| F-039 | MEDIUM | 0.9 | algorithmic-complexity | src/render_api.rs:831 | Post-hoc nesting validation permits recursive clone amplification |
| F-040 | MEDIUM | 0.9 | terminal-injection | src/repl/terminal.rs:122 | Model and sandbox output is interpreted as terminal control sequences |
| F-041 | MEDIUM | 0.9 | unbounded-allocation | src/viz/animator.rs:32 | Unbounded animation cycles trigger attacker-sized playback allocation |
| F-042 | LOW | 0.9 | audit-misattribution | src/concurrent.rs:114 | Concurrent audit records share one synthetic scope ID |
| F-043 | MEDIUM | 0.8 | filesystem-policy-bypass | crates/core/src/compiler.rs:160 | Relative filesystem restrictions cannot match resolved kernel paths |
| F-044 | MEDIUM | 0.8 | stale-authorization | crates/userspace/src/lib.rs:331 | Scope teardown leaves cgroup-keyed authorization entries behind |
| F-045 | MEDIUM | 0.8 | stale-policy-state | crates/userspace/src/lib.rs:332 | Cgroup ID reuse can combine new scopes with stale map state |
| F-046 | MEDIUM | 0.8 | consent-bypass | src/app/repl.rs:380 | Terminal control characters in skill metadata can forge consent displays |
| F-047 | MEDIUM | 0.8 | consent-spoofing | src/app/repl.rs:380 | Unescaped project skill metadata can forge the capability prompt |
| F-048 | MEDIUM | 0.8 | sensitive-data-in-logs | src/episode.rs:284 | Default progress logs expose tool arguments, results, and CTF flags |
| F-049 | MEDIUM | 0.8 | ssrf | src/mcp/bridge.rs:332 | Remote MCP redirects bypass the destination allowlist |
| F-050 | HIGH | 0.7 | path-traversal | src/skills.rs:110 | Lazy skill-body reads can be redirected to arbitrary host files |
| F-051 | MEDIUM | 0.7 | resource-exhaustion | src/render_api.rs:1023 | Repeated panel commits clone large widgets without an aggregate limit |
| F-052 | LOW | 0.3 | audit-integrity | crates/userspace/src/async_events.rs:77 | Concurrent audit events receive a shared synthetic scope identifier |
| F-053 | HIGH | 0.2 | sandbox-bypass | src/episode.rs:620 | Stdio MCP servers run with host-user authority in non-enforce builds |

### F-001

**Description:** Filesystem attenuation checks only child rules. A parent deny outside a child prefix can disappear, while unmatched reads are allowed by the kernel matcher.

**Exploit scenario:** A ceiling denies `/home/user/private`, but a child retaining only a `/workspace` read rule derives successfully and can read the omitted private path.

**Recommendation:** Compare effective decisions over inherited policy state, preserving every parent restriction unless the child is demonstrably stricter.

### F-002

**Description:** Protected `.git`, `.bee`, `.ssh`, and `.aws` denies are injected only during compilation. A child-specific grant can pass attenuation and later replace the injected deny.

**Exploit scenario:** A broad HOME write ceiling relies on the injected `~/.ssh` deny; a repository child adds a specific write rule that overrides it.

**Recommendation:** Include protected defaults in effective parent and child policies before attenuation.

### F-003

**Description:** Only `socket_connect` is mediated; unconnected UDP `sendto` and `sendmsg` do not consult `NET_ALLOW`.

**Exploit scenario:** A confined process transmits data to any IP and port using an unconnected UDP socket.

**Recommendation:** Mediate datagram destinations at `socket_sendmsg` or a packet-level cgroup hook.

### F-004

**Description:** An empty network list clears enforcement rather than installing deny-all.

**Exploit scenario:** A child omits destinations and receives unrestricted IP egress.

**Recommendation:** Track enforcement activation independently from allowlist cardinality.

### F-005

**Description:** Only `file_open` is mediated; metadata operations and hard-link aliases bypass pathname rules. Duplicate F-08-07 was merged here.

**Exploit scenario:** A workload hard-links a protected file under an allowed path or renames/unlinks protected files without an open.

**Recommendation:** Cover inode operations and use stable object identity.

### F-006

**Description:** An empty executable list omits the map entry that activates enforcement.

**Exploit scenario:** A no-exec child can run any accessible binary.

**Recommendation:** Encode active empty exec policy as deny-all.

### F-007

**Description:** `bpf_d_path` errors allow access, and an attacker can construct resolved paths beyond the fixed buffer.

**Exploit scenario:** A deeply nested relative executable triggers path failure and bypasses the allowlist.

**Recommendation:** Fail closed for managed scopes on path-helper errors.

### F-008

**Description:** Empty filesystem state and unmatched paths default to broad access.

**Exploit scenario:** A zero-capability workload reads or overwrites unprotected host files.

**Recommendation:** Make unmatched reads and writes deny by default.

### F-009

**Description:** Kernel subtree semantics disagree with the shared matcher for `/` and trailing slashes.

**Exploit scenario:** Root or trailing-slash denies miss descendants.

**Recommendation:** Canonicalize and test one shared matching contract.

### F-010

**Description:** Child hardening does not drop identity or capabilities and does not set `no_new_privs`.

**Exploit scenario:** A model tool inherits root or later invokes a privileged executable.

**Recommendation:** Drop to a dedicated identity, clear capabilities and groups, and set `no_new_privs`.

### F-011

**Description:** Nonempty cgroup removal failure is ignored before enforcement is detached.

**Exploit scenario:** A background descendant survives the session and becomes unrestricted.

**Recommendation:** Kill and reap the cgroup and verify emptiness before detaching.

### F-012

**Description:** Empty derived network state disables a parent's egress restriction.

**Exploit scenario:** A restricted child omits `network.allow` and gains unrestricted egress.

**Recommendation:** Preserve parent activation and encode deny-all.

### F-013

**Description:** Empty derived filesystem state clears the unmatched-write deny flag.

**Exploit scenario:** A child removes its final positive grant and gains writes elsewhere.

**Recommendation:** Preserve default-deny state independently of grants.

### F-014

**Description:** Empty derived exec state disables a parent's executable allowlist.

**Exploit scenario:** A child omits executables and can run arbitrary programs.

**Recommendation:** Preserve active exec policy and encode deny-all.

### F-015

**Description:** Privileged children can move to cgroups absent from exact-ID policy maps.

**Exploit scenario:** A workload migrates then performs denied operations.

**Recommendation:** Drop privilege, prevent migration, and use ancestry-aware enforcement.

### F-016

**Description:** Untrusted project policy is accepted verbatim when no user ceiling exists. Duplicate F-06-07 was merged here.

**Exploit scenario:** Opening a trojan repository silently grants broad host authority.

**Recommendation:** Require a trusted ceiling or explicit approval.

### F-017

**Description:** Skill directories receive read grants after attenuation and consent.

**Exploit scenario:** Discovery exposes content below a ceiling-denied directory.

**Recommendation:** Prove resource grants before skill registration.

### F-018

**Description:** Provider config selects both a host secret variable and the endpoint receiving it.

**Exploit scenario:** A repository sends `GITHUB_TOKEN` to an attacker-compatible endpoint.

**Recommendation:** Bind credentials to operator-approved origins.

### F-019

**Description:** Scenario filesystem effects occur host-side before sandbox creation.

**Exploit scenario:** Absolute, parent, or symlink paths overwrite host files.

**Recommendation:** Materialize beneath an opened directory with no-follow operations.

### F-020

**Description:** `run_episode` invokes unrestricted workdir materialization before building the scope.

**Exploit scenario:** A repository scenario writes a Git hook or launcher-user configuration.

**Recommendation:** Validate and contain every path component.

### F-021

**Description:** Requested tools are excluded from the policy object checked against the ceiling.

**Exploit scenario:** A tool-only skill request adds `bash` past the scenario allowlist.

**Recommendation:** Add a protected tool ceiling and explicit approval.

### F-022

**Description:** Direct-child waiting does not cover background descendants.

**Exploit scenario:** A redirected background process outlives enforcement.

**Recommendation:** Manage full process groups and cgroups.

### F-023

**Description:** Unsanitized scenario IDs form transcript paths. Duplicate F-09-03 was merged here.

**Exploit scenario:** `../../outside/report` escapes the output directory.

**Recommendation:** Encode IDs into safe basenames.

### F-024

**Description:** The `Path::join` sink accepts traversal-bearing scenario IDs.

**Exploit scenario:** Batch output overwrites a file outside `--out`.

**Recommendation:** Verify descriptor-relative containment.

### F-025

**Description:** Attenuation ignores the executable inode-pin marker.

**Exploit scenario:** A child removes the pin and replaces an allowed mutable path.

**Recommendation:** Make stable identity monotonic.

### F-026

**Description:** Exact cgroup lookup fails open for migrated processes.

**Exploit scenario:** A privileged child moves to an unregistered scope.

**Recommendation:** Prevent migration and enforce managed ancestry.

### F-027

**Description:** Non-IP socket families bypass `NET_ALLOW`.

**Exploit scenario:** A tool uses SSH-agent or container-runtime sockets.

**Recommendation:** Add AF_UNIX authorization and deny unsupported families.

### F-028

**Description:** Privileged-target checks cover only the initial executable.

**Exploit scenario:** An allowed shell later runs a setuid or file-capability binary.

**Recommendation:** Apply `no_new_privs` and enforce every transition.

### F-029

**Description:** Headless startup grants all discovered skill requests inside the ceiling.

**Exploit scenario:** An unused project skill silently widens the episode.

**Recommendation:** Grant only explicitly selected or invoked skills.

### F-030

**Description:** Tool environments are filtered by a small secret-name denylist.

**Exploit scenario:** Model shell output returns ambient cloud credentials to the provider.

**Recommendation:** Clear and reconstruct a minimal environment.

### F-031

**Description:** Untrusted skill metadata enters every model request before invocation.

**Exploit scenario:** A repository skill description steers the model toward unsafe tool use.

**Recommendation:** Require trust and quote metadata as data.

### F-032

**Description:** Child derivation does not compare exfiltration controls.

**Exploit scenario:** A child disables a parent-required detector.

**Recommendation:** Enforce monotonic exfiltration state.

### F-033

**Description:** Unknown security fields are ignored by deserialization.

**Exploit scenario:** A misspelled allowlist section silently becomes unrestricted.

**Recommendation:** Deny unknown fields throughout the policy schema.

### F-034

**Description:** AF_UNIX and other non-IP families are explicitly allowed.

**Exploit scenario:** A confined process reaches a local control socket.

**Recommendation:** Deny unsupported families and add explicit grants.

### F-035

**Description:** Reload publishes map changes before all fallible updates succeed.

**Exploit scenario:** A failed network update leaves an earlier filesystem grant active.

**Recommendation:** Stage atomically or fully roll back.

### F-036

**Description:** A fixed sleep is used as an audit-delivery barrier.

**Exploit scenario:** Delayed records attach to the wrong call or vanish after the final call.

**Recommendation:** Use watermarks and a synchronized final drain.

### F-037

**Description:** Retry audit replaces initial denial evidence at call scope.

**Exploit scenario:** Derived scores omit the denial that triggered a grant.

**Recommendation:** Preserve every attempt and correlate complete evidence.

### F-038

**Description:** Policy and HTTP transports parse URL authorities differently.

**Exploit scenario:** The gate sees an allowed host while the bearer token goes to an attacker host.

**Recommendation:** Parse once with the transport URL type.

### F-039

**Description:** Nested render trees are cloned before depth validation.

**Exploit scenario:** Repeated wrapping causes quadratic work or stack exhaustion.

**Recommendation:** Check depth before mutation and avoid deep clones.

### F-040

**Description:** Untrusted text reaches the host terminal without control filtering.

**Exploit scenario:** OSC and CSI sequences alter clipboard or screen.

**Recommendation:** Escape all external control characters.

### F-041

**Description:** Animation cycles directly multiply an eagerly allocated playback vector.

**Exploit scenario:** A script requests billions of cycles and exhausts memory.

**Recommendation:** Bound cycles and make playback lazy.

### F-042

**Description:** Concurrent events retain a shared synthetic scope label.

**Exploit scenario:** Downstream grouping merges evidence from distinct episodes.

**Recommendation:** Stamp the real scope after demultiplexing.

### F-043

**Description:** Relative policy paths cannot match absolute kernel paths.

**Exploit scenario:** A restrictive-looking `secrets` deny is ineffective.

**Recommendation:** Reject or resolve relative rules.

### F-044

**Description:** Scope teardown does not clear authorization maps.

**Exploit scenario:** A reused cgroup ID inherits stale network keys.

**Recommendation:** Delete all keys transactionally.

### F-045

**Description:** New scopes may combine with stale map entries from reused IDs.

**Exploit scenario:** Old destinations remain authorized.

**Recommendation:** Verify clean state before creation.

### F-046

**Description:** Raw control characters appear in the consent display.

**Exploit scenario:** ANSI sequences hide a dangerous request.

**Recommendation:** Escape fields and bind approval to a digest.

### F-047

**Description:** Project metadata can redraw the capability prompt.

**Exploit scenario:** The operator approves a forged benign display.

**Recommendation:** Render a canonical control-safe request.

### F-048

**Description:** Raw tool arguments and results are logged to stderr.

**Exploit scenario:** CTF flags or credentials leak into CI logs.

**Recommendation:** Redact secret fields and content.

### F-049

**Description:** Only the initial MCP destination is gated.

**Exploit scenario:** An allowed server redirects to an internal service.

**Recommendation:** Disable or validate every redirect.

### F-050

**Description:** Mutable symlink-following skill paths are reopened host-side.

**Exploit scenario:** Invocation reads an SSH key through a swapped symlink.

**Recommendation:** Use no-follow containment and stable identity.

### F-051

**Description:** Repeated panel commits retain redundant deep clones.

**Exploit scenario:** A script multiplies a large widget until the process exhausts memory.

**Recommendation:** Cap, coalesce, and budget aggregate output.

### F-052

**Description:** Concurrent events carry an inaccurate shared label, although numeric routing remains correct.

**Exploit scenario:** Label-only consumers confuse episode evidence.

**Recommendation:** Rewrite the label after demultiplexing.

### F-053

**Description:** Explicit non-enforce builds launch configured stdio MCP servers with host-user authority.

**Exploit scenario:** A compromised server reads same-user files or uses the network.

**Recommendation:** Require a distinct unsafe opt-in or enforced containment.
