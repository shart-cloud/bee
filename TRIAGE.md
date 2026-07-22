# Triage Report

27 in -> 0 duplicates, 9 false positives, 18 confirmed (6 high / 11 med / 1 low), 2 need manual test.

Context: interactive; environment = CLI/agent harness; operator configuration is trusted, but repositories, models, skills, tool output, and MCP responses may be malicious.; scoring = Derived HIGH/MEDIUM/LOW from preconditions; 3-vote verification.

## Act on these


### [HIGH] Every discovered skill receives capability grants before any skill is invoked  (f008)

`bee-harness/src/episode.rs:502` | capability-grant-without-invocation | claimed HIGH (alignment +3) | confidence 10/10

**Owner:** top committer: jg (9/9 recent commits); no CODEOWNERS entry

**Verdict:** exploitable, votes {"true_positive":3,"false_positive":0,"cannot_verify":0}

**Preconditions (2):**

- A configured skill root includes an attacker-controlled skill with a tool request
- An attacker-controlled model invokes the registered tool

**Threat-model match:** Unauthorized command/tool authority

**Why:** run_episode resolves grants for every discovered skill before invocation (bee-harness/src/episode.rs:596-608; bee-harness/src/skills/grant.rs:147-183). Hidden or uninvoked malicious project skills can therefore globally register tools, and tool-only requests are outside the policy ceiling.

Two realistic conditions yield MEDIUM, raised to HIGH because the defect directly grants unauthorized command/tool authority.

**Reachability evidence:** bee-harness/src/episode.rs:597


### [HIGH] Background descendants survive scope teardown and become unsandboxed when the engine detaches  (f012)

`bee-userspace/src/cgroup.rs:42` | auth-bypass | claimed HIGH (alignment +3) | confidence 10/10

**Owner:** top committer: jg (1/1 recent commits); no CODEOWNERS entry

**Verdict:** exploitable, votes {"true_positive":3,"false_positive":0,"cannot_verify":0}

**Preconditions (2):**

- An attacker-controlled tool daemonizes a descendant and redirects inherited pipes
- The episode ends while the descendant remains alive

**Threat-model match:** Sandbox escape and unauthorized command execution

**Why:** Tool execution tracks only the direct child, while teardown merely removes the cgroup and ignores a populated-cgroup failure (bee-harness/src/tools/exec.rs:38-55; bee-userspace/src/cgroup.rs:41-44; bee-harness/src/sandbox.rs:240-249). Dropping Engine then detaches enforcement, leaving a daemonized descendant alive.

Two realistic conditions yield MEDIUM, raised to HIGH for a direct enforcement escape.

**Reachability evidence:** bee-userspace/src/lib.rs:329, bee-harness/src/sandbox.rs:244


### [HIGH] Stdio MCP servers inherit host credentials outside a small name-based denylist  (f020)

`bee-harness/src/mcp/transport.rs:42` | credential-boundary-bypass | claimed HIGH (alignment +3) | confidence 10/10

**Owner:** top committer: jg (2/2 recent commits); no CODEOWNERS entry

**Verdict:** exploitable, votes {"true_positive":3,"false_positive":0,"cannot_verify":0}

**Preconditions (2):**

- Trusted configuration launches a malicious or compromised stdio MCP
- The harness has a useful ambient credential outside the strip list

**Threat-model match:** Secret exposure

**Why:** spawn_stdio creates an ordinary inherited-environment command (bee-harness/src/mcp/bridge.rs:291-301; bee-harness/src/mcp/transport.rs:40-48). The sandbox removes only a small list of provider and configured token names and never env_clear's, so a malicious MCP child receives unrelated cloud, Git, proxy, and agent credentials.

Two realistic preconditions yield MEDIUM, raised to HIGH for direct secret exposure.

**Reachability evidence:** bee-harness/src/mcp/bridge.rs:296


### [HIGH] Sandboxed tools inherit the privileged launcher's UID and BPF/cgroup capabilities  (f014)

`bee-userspace/src/spawn.rs:92` | privilege-escalation | claimed HIGH (alignment +4) | confidence 9.7/10

**Owner:** top committer: jg (2/2 recent commits); no CODEOWNERS entry

**Verdict:** exploitable, votes {"true_positive":3,"false_positive":0,"cannot_verify":0}

**Preconditions (2):**

- Bee is launched with root or equivalent enforcement capabilities
- An attacker-controlled source reaches a process-backed tool

**Threat-model match:** Privilege escalation and unauthorized command execution

**Why:** Model-controlled tool execution reaches hardened_command in the enforced sandbox (bee-harness/src/tools/bash.rs:43-52; bee-harness/src/sandbox.rs:164-190). Its pre-exec hardening disables dumps but never drops UID/GID/capabilities or sets no_new_privs (bee-userspace/src/spawn.rs:92-97; bee-hardening/src/lib.rs:29-32), so a privileged loader spawns privileged tools.

Two conditions yield MEDIUM, raised to HIGH for the exact privileged confused-deputy threat.

**Reachability evidence:** bee-harness/src/sandbox.rs:178, bee-harness/src/sandbox.rs:168


### [HIGH] Tool grants are not bounded by the capability ceiling  (f022)

`bee-harness/src/skills/grant.rs:142` | tool-authority-bypass | claimed HIGH (alignment +3) | confidence 9.7/10

**Owner:** top committer: jg (2/2 recent commits); no CODEOWNERS entry

**Verdict:** exploitable, votes {"true_positive":3,"false_positive":0,"cannot_verify":0}

**Preconditions (2):**

- A malicious skill requests a recognized security-relevant tool absent from the base registry
- Grant resolution uses AllowWithinCeiling or receives approval

**Threat-model match:** Unauthorized command execution

**Why:** The ceiling derivation covers only filesystem Policy, while requested tools are appended after approval (bee-harness/src/skills/grant.rs:152-182,195-207). Episode setup uses AllowWithinCeiling and registers those tools before the loop, so a malicious tool-only skill can add bash or write_file outside the operator's tool set.

Two realistic conditions yield MEDIUM, raised to HIGH for confused-deputy command authority.

**Reachability evidence:** bee-harness/src/episode.rs:743, bee-harness/src/episode.rs:597


### [HIGH] Repository-controlled workdir paths are written on the trusted host before sandboxing  (f007)

`bee-harness/src/episode.rs:424` | arbitrary-host-write | claimed HIGH (alignment +1) | confidence 9/10

**Owner:** top committer: jg (9/9 recent commits); no CODEOWNERS entry

**Verdict:** exploitable, votes {"true_positive":2,"false_positive":1,"cannot_verify":0}

**Preconditions (2):**

- A trusted relative materialization target crosses a repository-controlled symlink
- The escaped host target is writable by the launcher

**Threat-model match:** Unauthorized host access

**Why:** materialize_workdir performs unanchored host writes before sandbox construction (bee-harness/src/episode.rs:518-538,577-619). Although scenario paths are trusted config, an untrusted repository can pre-place a symlink beneath a trusted relative target, so the winning votes found a reachable host-write escape.

Two conditions yield MEDIUM, raised to HIGH for direct unauthorized host access; only the repository-symlink variant survives the trusted-config boundary.

**Reachability evidence:** bee-harness/src/episode.rs:577


### [MEDIUM] Removing all child write grants disables the parent's default-deny write boundary  (f001)

`bee-core/src/attenuation.rs:62` | capability-widening | claimed HIGH (alignment -3) | confidence 10/10

**Owner:** top committer: jg (2/2 recent commits); no CODEOWNERS entry

**Verdict:** exploitable, votes {"true_positive":3,"false_positive":0,"cannot_verify":0}

**Preconditions (4):**

- Enforcement mode with a parent write grant
- Attacker-controlled child policy removes all filesystem grants
- Operator launches the derived child
- A same-UID writable host path exists outside the grant

**Threat-model match:** Unauthorized host access

**Why:** check_filesystem iterates only child entries, so an empty child succeeds (bee-core/src/attenuation.rs:63). The live CLI compiles the derived child directly (bee-cli/src/main.rs:169-184). Planning leaves FLAG_FS_WRITE_DEFAULT_DENY unset without a child write rule (bee-userspace/src/plan.rs:48), and unmatched writes are then allowed (bee-ebpf/src/main.rs:331); the parent boundary does not survive.

Multiple local preconditions yield LOW, raised once to MEDIUM for direct unauthorized host access; claimed HIGH is inflated.

**Reachability evidence:** bee-cli/src/main.rs:104, bee-cli/src/main.rs:169, bee-core/src/attenuation.rs:54


### [MEDIUM] An empty child executable allowlist turns restricted execution into unrestricted execution  (f002)

`bee-core/src/attenuation.rs:159` | capability-widening | claimed HIGH (alignment -3) | confidence 10/10

**Owner:** top committer: jg (2/2 recent commits); no CODEOWNERS entry

**Verdict:** exploitable, votes {"true_positive":3,"false_positive":0,"cannot_verify":0}

**Preconditions (4):**

- Enforcement mode with a parent exec allowlist
- Attacker controls a child policy with an empty exec list
- Operator launches the derived child
- A disallowed executable is accessible

**Threat-model match:** Unauthorized command execution

**Why:** The production child path passes the child through parent.derive at bee-cli/src/main.rs:169. check_exec iterates only child entries (bee-core/src/attenuation.rs:160-174), so an empty child succeeds; compilation/planning omit EXEC_ALLOW (bee-core/src/compiler.rs:102-113; bee-userspace/src/plan.rs:61-64), and the LSM permits all execution when the map entry is absent (bee-ebpf/src/main.rs:200-204).

Local delegated-policy control and an accessible binary yield LOW, raised to MEDIUM for direct command execution.

**Reachability evidence:** bee-cli/src/main.rs:169, bee-cli/src/main.rs:104


### [MEDIUM] An empty child network allowlist disables egress enforcement  (f003)

`bee-core/src/attenuation.rs:177` | capability-widening | claimed HIGH (alignment -3) | confidence 10/10

**Owner:** top committer: jg (2/2 recent commits); no CODEOWNERS entry

**Verdict:** exploitable, votes {"true_positive":3,"false_positive":0,"cannot_verify":0}

**Preconditions (4):**

- Enforcement mode with a parent network allowlist
- Attacker controls an empty child network list
- Operator launches the derived child
- A disallowed destination is reachable

**Threat-model match:** Unauthorized network egress

**Why:** check_network vacuously accepts an empty child list (bee-core/src/attenuation.rs:177; confirmed by bee-core/tests/attenuation.rs:95). Compilation produces no network rules, planning leaves FLAG_NET_ENFORCED unset (bee-userspace/src/plan.rs:45), and socket_connect permits every connection when that flag is absent (bee-ebpf/src/main.rs:99). The production path invokes derive at bee-cli/src/main.rs:169.

Local scoped execution and delegated-policy control yield LOW, raised to MEDIUM for direct network egress.

**Reachability evidence:** bee-cli/src/main.rs:104, bee-cli/src/main.rs:169


### [MEDIUM] Path rules can be bypassed by relinking or renaming denied files  (f005)

`bee-ebpf/src/main.rs:142` | filesystem-policy-bypass | claimed HIGH (alignment -3) | confidence 10/10

**Owner:** top committer: jg (1/1 recent commits); no CODEOWNERS entry

**Verdict:** exploitable, votes {"true_positive":3,"false_positive":0,"cannot_verify":0}

**Preconditions (4):**

- Enforced pathname policy
- Attacker controls a same-UID scoped process
- Hardlink or rename is permitted
- Target DAC permissions permit access

**Threat-model match:** Unauthorized host access or secret exposure

**Why:** The loader attaches only socket_connect, file_open, and bprm_check_security (bee-userspace/src/loader.rs:14), leaving link and rename operations uncovered. file_open authorizes only the rendered path (bee-ebpf/src/main.rs:169-180), so a permitted hardlink alias is evaluated under its allowed name rather than the protected source path; README.md:72 acknowledges this gap.

Filesystem and permission prerequisites yield LOW, raised to MEDIUM for host access or secret exposure.

**Reachability evidence:** bee-userspace/src/loader.rs:14, bee-ebpf/src/main.rs:142


### [MEDIUM] UDP sendto bypasses the network allowlist  (f004)

`bee-ebpf/src/main.rs:92` | network-policy-bypass | claimed HIGH (alignment -2) | confidence 9.7/10

**Owner:** top committer: jg (1/1 recent commits); no CODEOWNERS entry

**Verdict:** exploitable, votes {"true_positive":3,"false_positive":0,"cannot_verify":0}

**Preconditions (4):**

- Enforced scope with a network allowlist
- Attacker controls a scoped process
- Process uses unconnected UDP
- A disallowed UDP destination is reachable

**Threat-model match:** Unauthorized network egress

**Why:** The loader attaches socket_connect but no socket_sendmsg or packet-egress hook (bee-userspace/src/loader.rs:14-23). NET_ALLOW is consulted only in socket_connect (bee-ebpf/src/main.rs:92-129), while the design explicitly defers connectionless sendto filtering (specs/001-ebpf-agent-sandbox/research.md:189), leaving a concrete bypass for an untrusted scoped process.

Local sandbox execution and reachable UDP yield LOW, raised to MEDIUM for direct egress; HIGH is inflated.

**Reachability evidence:** bee-userspace/src/loader.rs:14, bee-userspace/src/loader.rs:23, bee-harness/src/episode.rs:324


### [MEDIUM] Full audit ring silently discards enforcement records without marking transcripts incomplete  (f015)

`bee-ebpf/src/main.rs:383` | audit-integrity | claimed MEDIUM (alignment +2) | confidence 9.5/10

**Owner:** top committer: jg (1/1 recent commits); no CODEOWNERS entry

**Verdict:** exploitable, votes {"true_positive":2,"false_positive":1,"cannot_verify":0}

**Preconditions (2):**

- An attacker-controlled tool floods enough denials to fill the audit ring
- The event to hide occurs after saturation and before drain

**Threat-model match:** none

**Why:** Denied operations emit audit records into a bounded 256-KiB ring, but reservation failure silently drops the record without a loss marker (bee-ebpf/src/main.rs:88-90,382-415). Synchronous episodes drain after attacker-controlled tool completion (bee-harness/src/episode.rs:323-338), so a denial flood can make transcripts silently incomplete.

Two realistic conditions derive MEDIUM; silent audit loss does not directly match the stated authority threats.

**Reachability evidence:** bee-ebpf/src/main.rs:134, bee-ebpf/src/main.rs:185


### [MEDIUM] Rhai layout cloning permits exponential in-process memory amplification before validation  (f023)

`bee-harness/src/render_api.rs:486` | algorithmic-complexity | claimed MEDIUM (alignment +2) | confidence 9/10

**Owner:** top committer: jg (3/3 recent commits); no CODEOWNERS entry

**Verdict:** needs_manual_test, votes {"true_positive":3,"false_positive":0,"cannot_verify":0}

**Preconditions (2):**

- The render tool is enabled
- An untrusted model submits an amplifying Rhai script

**Threat-model match:** none

**Why:** Model-controlled Rhai reaches layout add, whose conversion deep-clones existing child trees before storage (bee-harness/src/tools/render.rs:101-111; bee-harness/src/render_api.rs:205-210,483-489). Repeated reuse doubles native Rust-owned structures, while structural validation occurs only at final render after allocation (bee-harness/src/render_api.rs:596-603).

Two preconditions derive MEDIUM, but a human PoC is needed to confirm material amplification under Rhai operation and copy semantics.

**Reachability evidence:** bee-harness/src/tools/render.rs:111, bee-harness/src/tools/render.rs:72

> Recommend a human build a PoC; static reasoning hit its limit.


### [MEDIUM] Lazy skill-body reads can be redirected to arbitrary host files after discovery  (f021)

`bee-harness/src/skills.rs:109` | symlink-toctou-host-read | claimed HIGH (alignment -2) | confidence 9/10

**Owner:** top committer: jg (2/2 recent commits); no CODEOWNERS entry

**Verdict:** exploitable, votes {"true_positive":3,"false_positive":0,"cannot_verify":0}

**Preconditions (4):**

- A repository-controlled skill is discovered by mutable path
- The attacker can replace that path after discovery
- The skill is later invoked
- A sensitive target is readable and observable

**Threat-model match:** Unauthorized host access and secret exposure

**Why:** Discovery follows links and stores a mutable SKILL.md pathname, while model invocation later reopens it with host authority (bee-harness/src/skills.rs:109-114,184-190,269-274; bee-harness/src/tools/skill.rs:109-111). A writable project entry can be replaced by a symlink before invocation to disclose an arbitrary host-readable file.

Four preconditions yield LOW, raised to MEDIUM for host read and secret exposure; claimed HIGH is inflated.

**Reachability evidence:** bee-harness/src/tools/skill.rs:109


### [MEDIUM] Unresolvable long paths fail open for file and executable policy  (f006)

`bee-ebpf/src/main.rs:172` | fail-open-enforcement | claimed HIGH (alignment -3) | confidence 9/10

**Owner:** top committer: jg (1/1 recent commits); no CODEOWNERS entry

**Verdict:** exploitable, votes {"true_positive":3,"false_positive":0,"cannot_verify":0}

**Preconditions (5):**

- Active filesystem or exec enforcement
- Attacker controls a scoped process
- A rendered path can exceed 4096 bytes
- Target DAC permissions permit the operation
- bpf_d_path fails on that path

**Threat-model match:** Unauthorized host access or command execution

**Why:** Both file_open and bprm_check_security use a fixed 4096-byte buffer and return allow when bpf_d_path fails (bee-common/src/lib.rs:20-21; bee-ebpf/src/main.rs:169-174,222-225). No depth restriction or fallback identity check closes the path for model-controlled scoped children reached through bee-harness/src/tools/bash.rs:43-48.

Several local path and filesystem prerequisites yield LOW, raised to MEDIUM for the matched host-access/exec threat.

**Reachability evidence:** bee-harness/src/tools/bash.rs:48, bee-ebpf/src/main.rs:172, bee-ebpf/src/main.rs:171


### [MEDIUM] Executable authorization is bound only to a mutable path  (f027)

`bee-core/src/compiler.rs:108` | exec-identity-toctou | claimed MEDIUM (alignment +2) | confidence 8.7/10

**Owner:** top committer: jg (1/1 recent commits); no CODEOWNERS entry

**Verdict:** exploitable, votes {"true_positive":3,"false_positive":0,"cannot_verify":0}

**Preconditions (3):**

- An ordinary path-only exec rule is used
- The attacker can replace the allowlisted executable or parent entry
- Replacement occurs after compilation before invocation

**Threat-model match:** Unauthorized command execution

**Why:** Ordinary exec entries resolve once but store only mutable path bytes, and the backend rejects inode pinning (bee-core/src/compiler.rs:102-112; bee-userspace/src/plan.rs:186-215). The LSM permits whatever file currently occupies the matching pathname, so a writable allowlisted executable can be replaced persistently with attacker-controlled content.

Three preconditions yield LOW, raised to MEDIUM for unauthorized command execution.

**Reachability evidence:** bee-harness/src/episode.rs:770


### [MEDIUM] Scope teardown leaves BPF rules keyed by reusable cgroup inode IDs  (f024)

`bee-userspace/src/lib.rs:223` | cgroup-policy-confusion | claimed MEDIUM (alignment +2) | confidence 8/10

**Owner:** top committer: jg (2/2 recent commits); no CODEOWNERS entry

**Verdict:** needs_manual_test, votes {"true_positive":2,"false_positive":1,"cannot_verify":0}

**Preconditions (4):**

- The enforcing BPF backend is active
- An earlier scope installs rules and tears down
- The kernel reuses its cgroup ID
- The new scope does not overwrite every stale key

**Threat-model match:** Unauthorized network egress

**Why:** Scope teardown removes only the cgroup directory and does not delete SCOPES, FS, EXEC, or NET map entries (bee-userspace/src/lib.rs:118-169,327-330). Concurrent teardown occurs while the shared Engine remains alive, so cgroup-ID reuse can misapply stale policy without a generation check.

Four preconditions yield LOW, raised to MEDIUM for unauthorized egress; runtime testing is needed to demonstrate practical ID reuse.

**Reachability evidence:** bee-harness/src/sandbox.rs:244, bee-harness/src/concurrent.rs:214

> Recommend a human build a PoC; static reasoning hit its limit.


### [LOW] Drain-window correlation attributes unrelated or late audit events to the current tool call  (f017)

`bee-harness/src/episode.rs:295` | audit-misattribution | claimed MEDIUM (alignment -2) | confidence 8.5/10

**Owner:** top committer: jg (9/9 recent commits); no CODEOWNERS entry

**Verdict:** exploitable, votes {"true_positive":2,"false_positive":1,"cannot_verify":0}

**Preconditions (3):**

- An earlier tool leaves a background descendant
- It emits after the earlier drain and before a later drain
- A later call consumes the undifferentiated event

**Threat-model match:** none

**Why:** After each call, run_loop drains all cgroup events and assigns them to the current ToolCall without a pre-call watermark or call identifier (bee-harness/src/episode.rs:323-338,444-448; bee-harness/src/sandbox.rs:210-235). A background child can therefore cause delayed events to be attributed to a later call and influence reactive escalation.

Three sequencing conditions force LOW and attribution corruption alone does not directly match the stated authority threats.

**Reachability evidence:** bee-harness/src/episode.rs:337


## Dropped

| id | title | file:line | why dropped |
|---|---|---|---|
| f009 | Skill directories are made readable after the ceiling proof | bee-harness/src/episode.rs:614 | intentional_behavior; exclusion rule 3 |
| f010 | MCP Bearer tokens are sent over unrestricted plaintext HTTP endpoints | bee-harness/src/mcp/bridge.rs:338 | implausible_trigger; exclusion rule 8 |
| f011 | Custom URL parsing disagrees with the HTTP client on backslash authority boundaries | bee-harness/src/mcp/policy.rs:78 | implausible_trigger; exclusion rule 8 |
| f013 | Executable entries are encoded as subtree prefixes instead of exact paths | bee-userspace/src/plan.rs:210 | intentional_behavior; exclusion rule 3 |
| f016 | Scenario identifiers escape the batch transcript output directory | bee-harness/src/bin/bee-episode.rs:334 | implausible_trigger; exclusion rule 8 |
| f018 | Privileged-executable refusal races pathname replacement before exec | bee-userspace/src/spawn.rs:75 | implausible_trigger; exclusion rule 16 |
| f019 | Provider configuration can select any host environment secret and send it to an arbitrary endpoint | bee-harness/src/config.rs:79 | implausible_trigger, intentional_behavior; exclusion rule 8 |
| f025 | Every concurrent episode's audit records carry the same false scope identifier | bee-harness/src/concurrent.rs:114 | not_actionable; exclusion rule 12 |
| f026 | Default non-enforcement builds execute model-requested tools directly on the host | bee-harness/src/episode.rs:523 | intentional_behavior; exclusion rule 3 |

