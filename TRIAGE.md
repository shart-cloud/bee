# Triage Report

53 in → 12 duplicates, 11 false positives, 30 confirmed (6 high / 24 medium / 0 low), 1 needs manual test.

Context: interactive; environment = CLI/batch tool and interactive REPL, with operator inputs trusted and repository/model/tool/skill/MCP content untrusted; scoring = derived HIGH/MEDIUM/LOW; 3-vote verification; precision tie-breaking.

## Remediated

This report is a snapshot of the triage run; the findings below have since been fixed. The entries
are left in place unedited for provenance.

| Findings | Fix |
|----------|-----|
| f018, f020, f022, f026, f028, f030 (all six HIGHs) | `8e2cdbb` + `3225d44` — closed and VM-verified (31/31 matrix) |
| f001, f002, f012, f014, f025 (+ absorbed f006, f032) | branch `013-attenuation-inheritance` — one root cause: attenuation validated only what a child *stated*, so omission widened authority. `Policy::derive` now returns the *effective* child policy (silence inherits, it does not reset) and refuses child grants reaching into FR-008 protected regions. See research R15; regression tests in `crates/core/tests/attenuation.rs`. |
| f040, f046 (+ absorbed f047) | branch `014-terminal-safety` — untrusted text is escaped where it enters a front-end (`safe_text`, applied in `repl::terminal` and `tui::app::handle_session`) and only then styled; the consent prompt escapes every field it prints; skill frontmatter carrying control or bidi characters is refused at load. |
| f007, f034 | branch `015-hooks-fail-closed` — LSM hooks refuse what they cannot evaluate: a `bpf_d_path` failure, a null struct argument, an unavailable scratch slot, and any non-IP address family are denied and audited instead of allowed. Research R16; VM cases `exec-unresolvable-denied`, `file-unresolvable-denied`, `net-unix-denied` (35/35). |
| f010 | **Not a defect — stale report.** `hardening.rs::drop_privileges` already sets `PR_SET_NO_NEW_PRIVS` and empties the capability bounding set except the DAC pair; VM case `priv-drop` asserts `NoNewPrivs=1 CapBnd=0x6`. Closed by `8e2cdbb`/`3225d44`, which the report predates. |

## Act on these
### [HIGH] Provider TOML can send an arbitrary environment secret to an attacker endpoint  (f018)
`src/batch.rs:133` | credential-exposure | claimed HIGH (alignment +4) | confidence 10.0/10
**Owner:** top committer: jg (1/1 recent commits); no CODEOWNERS entry
**Verdict:** exploitable, votes {"true_positive":3,"false_positive":0,"cannot_verify":0}
**Preconditions (2):**
- attacker-controlled provider TOML selected in batch
- named host secret exists
**Threat-model match:** credential disclosure and unauthorized egress
**Why:** Batch mode passes repository provider files directly to run_batch, which reads the configured api_key_env at src/batch.rs:128-134. The same object controls base_url and src/provider/rig_model.rs:209-220 binds the selected secret to that arbitrary compatible endpoint without the ordinary project-config trust rejection.

Ranking: authenticated access; 2 preconditions; threat match: credential disclosure and unauthorized egress. Derived HIGH.
**Reachability evidence:** src/app/run.rs:334

### [HIGH] Repository-controlled workdir paths permit arbitrary host overwrite before sandboxing  (f020)
`src/episode.rs:579` | path-traversal | claimed HIGH (alignment +4) | confidence 10.0/10
**Owner:** top committer: jg (1/1 recent commits); no CODEOWNERS entry
**Verdict:** exploitable, votes {"true_positive":3,"false_positive":0,"cannot_verify":0}
**Preconditions (2):**
- attacker-controlled scenario selected
- launcher can write target
**Threat-model match:** host modification and persistence
**Why:** Scenario validation does not constrain create_dirs, create_files, or flag paths at src/scenario.rs:144-200. src/episode.rs:579-621 materializes them with host create_dir_all and write before constructing the sandbox, allowing absolute, parent-traversal, and symlink-crossing overwrites.

Ranking: authenticated access; 2 preconditions; threat match: host modification and persistence. Derived HIGH.
**Reachability evidence:** src/app/run.rs:241

### [HIGH] Background descendants survive tool deadlines and outlive enforcement  (f022)
`src/tools/exec.rs:38` | sandbox-bypass | claimed HIGH (alignment +4) | confidence 10.0/10
**Owner:** top committer: jg (1/1 recent commits); no CODEOWNERS entry
**Verdict:** exploitable, votes {"true_positive":3,"false_positive":0,"cannot_verify":0}
**Preconditions (2):**
- process tool daemonizes redirected descendant
- episode ends while descendant lives
**Threat-model match:** direct sandbox escape
**Why:** run_child waits and kills only the direct shell at src/tools/exec.rs:30-55. A redirected background descendant keeps the cgroup populated; src/sandbox.rs:241-250 ignores remove_dir failure and dropping the sandbox detaches its Engine, leaving the descendant alive without enforcement.

Ranking: authenticated access; 2 preconditions; threat match: direct sandbox escape. Derived HIGH.
**Reachability evidence:** src/tools/bash.rs:48

### [HIGH] Credential stripping denylist exposes ambient secrets to model tools  (f030)
`src/sandbox.rs:20` | sensitive-data-exposure | claimed HIGH (alignment +4) | confidence 9.6/10
**Owner:** top committer: jg (1/1 recent commits); no CODEOWNERS entry
**Verdict:** exploitable, votes {"true_positive":3,"false_positive":0,"cannot_verify":0}
**Preconditions (2):**
- untrusted model can invoke process tool
- useful ambient credential outside strip list
**Threat-model match:** credential disclosure and downstream access
**Why:** Tool children inherit the parent environment because hardened_command does not env_clear. src/sandbox.rs:18-25,103-111,254-258 removes only a short provider/MCP list, so model-controlled bash can read other ambient cloud, GitHub, Kubernetes, or agent credentials and return them to the model.

Ranking: authenticated access; 2 preconditions; threat match: credential disclosure and downstream access. Derived HIGH.
**Reachability evidence:** src/sandbox.rs:169, src/tools/exec.rs:25

### [HIGH] Privileged-target refusal checks only the first executable  (f028)
`crates/userspace/src/spawn.rs:74` | privileged-target-bypass | claimed HIGH (alignment +3) | confidence 9.5/10
**Owner:** top committer: jg (1/1 recent commits); no CODEOWNERS entry
**Verdict:** exploitable, votes {"true_positive":3,"false_positive":0,"cannot_verify":0}
**Preconditions (2):**
- attacker controls process tool
- usable setid/capability binary accessible
**Threat-model match:** privilege escalation
**Why:** hardened_command checks privileged metadata only on the initial program at crates/userspace/src/spawn.rs:74-80. Pre-exec hardening sets neither no_new_privs nor credential drops, so an ordinary shell can later execute a setid or file-capability image, especially when exec policy is absent.

Ranking: authenticated access; 2 preconditions; threat match: privilege escalation. Derived HIGH.
**Reachability evidence:** src/sandbox.rs:179, src/tools/exec.rs:25

### [HIGH] Exact cgroup-ID lookup lets migrated processes leave enforcement  (f026)
`crates/ebpf/src/main.rs:95` | auth-bypass | claimed HIGH (alignment +4) | confidence 9.2/10
**Owner:** top committer: jg (1/1 recent commits); no CODEOWNERS entry
**Verdict:** exploitable, votes {"true_positive":3,"false_positive":0,"cannot_verify":0}
**Preconditions (2):**
- attacker-controlled scoped process
- inherited or delegated cgroup migration authority
**Threat-model match:** direct sandbox escape
**Why:** All three LSM hooks use only the exact current cgroup ID and allow when SCOPES lacks it at crates/ebpf/src/main.rs:92-100,142-151,193-204. Child hardening drops no privilege, so a privileged or delegated workload can migrate to another cgroup without any ancestor lookup.

Ranking: authenticated access; 2 preconditions; threat match: direct sandbox escape. Derived HIGH.
**Reachability evidence:** src/episode.rs:787, src/sandbox.rs:179, src/tools/exec.rs:25

### [MEDIUM] Scenario ID escapes the batch transcript output directory  (f023)
`src/app/run.rs:407` | path-traversal | claimed MEDIUM (alignment +2) | confidence 10.0/10
**Owner:** top committer: jg (1/1 recent commits); no CODEOWNERS entry
**Verdict:** exploitable, votes {"true_positive":3,"false_positive":0,"cannot_verify":0}
**Preconditions (3):**
- attacker scenario id processed in batch
- escaped parent writable
- model suffix predictable
**Threat-model match:** host-file and transcript integrity
**Why:** Scenario validation only rejects an empty ID at src/scenario.rs:144-155. Batch output interpolates the untrusted ID and joins it beneath --out at src/app/run.rs:395-409, so parent or absolute components escape before std::fs::write.

Ranking: local access; 3 preconditions; threat match: host-file and transcript integrity. Derived MEDIUM.
**Reachability evidence:** src/app/run.rs:350, src/scenario.rs:104

### [MEDIUM] Child policies can omit parent deny regions and regain default-allowed reads  (f001)
`crates/core/src/attenuation.rs:63` | attenuation-bypass | claimed HIGH (alignment -2) | confidence 10.0/10
**Owner:** top committer: jg (1/1 recent commits); no CODEOWNERS entry
**Verdict:** exploitable, votes {"true_positive":3,"false_positive":0,"cannot_verify":0}
**Preconditions (4):**
- enforced filesystem mediation
- trusted parent denial
- untrusted child omits denial
- target readable under host DAC
**Threat-model match:** sandbox-policy bypass and host data/credential exposure
**Why:** Untrusted project configuration can reach ceiling.derive at src/app/config/mod.rs:353-362. crates/core/src/attenuation.rs:62-90 checks only child filesystem entries and returns the unmerged request, while crates/ebpf/src/main.rs:331-335 allows unmatched reads, so an omitted parent deny concretely widens authority.

Ranking: local access; 4 preconditions; threat match: sandbox-policy bypass and host data/credential exposure. Derived MEDIUM.
**Reachability evidence:** src/app/config/mod.rs:361

### [MEDIUM] A child policy can override protected defaults absent from the attenuation ceiling  (f002)
`crates/core/src/compiler.rs:86` | attenuation-bypass | claimed HIGH (alignment -2) | confidence 10.0/10
**Owner:** top committer: jg (1/1 recent commits); no CODEOWNERS entry
**Verdict:** exploitable, votes {"true_positive":3,"false_positive":0,"cannot_verify":0}
**Preconditions (4):**
- broad trusted parent grant
- protected default exists only at compile time
- untrusted child adds specific protected-path grant
- host DAC permits access
**Threat-model match:** host credential exposure and policy bypass
**Why:** Protected defaults are introduced only during compilation at crates/core/src/compiler.rs:83-100, after attenuation. A specific child grant can pass crates/core/src/attenuation.rs:101-155 and then outrank or replace the injected protected rule during crates/userspace/src/plan.rs:151-173.

Ranking: local access; 4 preconditions; threat match: host credential exposure and policy bypass. Derived MEDIUM.
**Reachability evidence:** src/app/config/mod.rs:361, src/app/session.rs:311, src/app/session.rs:312

### [MEDIUM] An empty child network list disables the parent egress allowlist  (f012)
`crates/userspace/src/plan.rs:45` | network-policy-bypass | claimed HIGH (alignment -2) | confidence 10.0/10
**Owner:** top committer: jg (1/1 recent commits); no CODEOWNERS entry
**Verdict:** exploitable, votes {"true_positive":3,"false_positive":0,"cannot_verify":0}
**Preconditions (4):**
- restrictive parent network list
- untrusted empty child list
- child planning clears enforcement
- reachable destination
**Threat-model match:** attenuation bypass and unauthorized network access
**Why:** Network attenuation checks only child destinations at crates/core/src/attenuation.rs:177-186, so an empty child passes. crates/userspace/src/plan.rs:45-47 clears FLAG_NET_ENFORCED and crates/ebpf/src/main.rs:94-101 then allows all connections for the exact child cgroup.

Ranking: local access; 4 preconditions; threat match: attenuation bypass and unauthorized network access. Derived MEDIUM.
**Reachability evidence:** src/app/config/mod.rs:361, src/main.rs:194, src/main.rs:221

### [MEDIUM] An empty child executable list turns a restricted parent into unrestricted execution  (f014)
`crates/userspace/src/plan.rs:63` | exec-allowlist-bypass | claimed HIGH (alignment -2) | confidence 10.0/10
**Owner:** top committer: jg (1/1 recent commits); no CODEOWNERS entry
**Verdict:** exploitable, votes {"true_positive":3,"false_positive":0,"cannot_verify":0}
**Preconditions (4):**
- restrictive parent exec list
- untrusted empty child list
- missing child map
- accessible disallowed binary
**Threat-model match:** exec attenuation failure and escape
**Why:** Executable attenuation validates only child-listed entries at crates/core/src/attenuation.rs:159-174, so an empty list passes. Planning installs no EXEC_ALLOW and crates/ebpf/src/main.rs:193-204 interprets the absent exact-child entry as unrestricted execution.

Ranking: local access; 4 preconditions; threat match: exec attenuation failure and escape. Derived MEDIUM.
**Reachability evidence:** src/app/config/mod.rs:361, src/main.rs:194, src/main.rs:221

### [MEDIUM] Kernel subtree matcher mishandles root and trailing-slash rules  (f009)
`crates/ebpf/src/main.rs:363` | filesystem-policy-bypass | claimed HIGH (alignment -3) | confidence 10.0/10
**Owner:** top committer: jg (1/1 recent commits); no CODEOWNERS entry
**Verdict:** exploitable, votes {"true_positive":3,"false_positive":0,"cannot_verify":0}
**Preconditions (4):**
- root or trailing-slash restrictive rule
- raw spelling reaches kernel
- attacker accesses missed descendant
- host DAC permits access
**Threat-model match:** policy/enforcement divergence
**Why:** The shared matcher normalizes root and trailing slashes at crates/common/src/matcher.rs:8-25, while crates/ebpf/src/main.rs:363-379 compares raw rule bytes and requires another separator. Compilation preserves these forms, so valid deny or read-only rules can silently miss descendants.

Ranking: local access; 4 preconditions; threat match: policy/enforcement divergence. Derived MEDIUM.
**Reachability evidence:** crates/ebpf/src/main.rs:260

### [MEDIUM] Model and sandbox output is interpreted as terminal control sequences  (f040)
`src/repl/terminal.rs:122` | terminal-injection | claimed MEDIUM (alignment +5) | confidence 9.9/10
**Owner:** top committer: jg (1/1 recent commits); no CODEOWNERS entry
**Verdict:** exploitable, votes {"true_positive":3,"false_positive":0,"cannot_verify":0}
**Preconditions (3):**
- live terminal output enabled
- attacker text contains controls
- terminal interprets controls
**Threat-model match:** terminal-state integrity
**Why:** Raw model deltas and tool results reach ExternalPrinter::print through src/repl.rs:352-375 and src/repl/terminal.rs:119-123,241-312 without control-character escaping. Color wrappers and caps do not neutralize embedded ESC, CSI, or OSC sequences.

Ranking: authenticated access; 3 preconditions; threat match: terminal-state integrity. Derived MEDIUM.
**Reachability evidence:** src/repl.rs:375

### [MEDIUM] Unbounded animation cycles trigger attacker-sized playback allocation  (f041)
`src/viz/animator.rs:32` | unbounded-allocation | claimed MEDIUM (alignment +5) | confidence 9.9/10
**Owner:** top committer: jg (1/1 recent commits); no CODEOWNERS entry
**Verdict:** exploitable, votes {"true_positive":3,"false_positive":0,"cannot_verify":0}
**Preconditions (3):**
- render tool and inline REPL
- nonempty animation
- large cycles value
**Threat-model match:** algorithmic denial of service
**Why:** Model Rhai can set cycles to an unrestricted u32 at src/render_api.rs:982-985. Inline terminal rendering calls playback, which allocates and materializes period_len times cycles at src/viz/animator.rs:26-36 outside Rhai operation and array limits, enabling deterministic process OOM.

Ranking: authenticated access; 3 preconditions; threat match: algorithmic denial of service. Derived MEDIUM.
**Reachability evidence:** src/repl/terminal.rs:183

### [MEDIUM] A child can remove an executable inode-pin requirement  (f025)
`crates/core/src/attenuation.rs:161` | executable-identity-bypass | claimed HIGH (alignment -3) | confidence 9.9/10
**Owner:** top committer: jg (1/1 recent commits); no CODEOWNERS entry
**Verdict:** exploitable, votes {"true_positive":3,"false_positive":0,"cannot_verify":0}
**Preconditions (3):**
- pinned parent entry
- attacker controls child policy
- allowlisted path attacker-mutable
**Threat-model match:** executable substitution
**Why:** crates/core/src/attenuation.rs:159-167 strips ! before comparing parent and child executable entries, but compilation preserves the distinction as pin_inode. An unpinned child therefore passes a pinned ceiling and avoids the backend's fail-closed rejection at crates/userspace/src/plan.rs:195-201.

Ranking: local access; 3 preconditions; threat match: executable substitution. Derived MEDIUM.
**Reachability evidence:** src/app/config/mod.rs:361, src/main.rs:194, src/skills/grant.rs:160

### [MEDIUM] Tool children retain launcher privileges and can execute privileged descendants  (f010)
`crates/userspace/src/hardening.rs:29` | privilege-escalation | claimed HIGH (alignment -1) | confidence 9.7/10
**Owner:** top committer: jg (1/1 recent commits); no CODEOWNERS entry
**Verdict:** exploitable, votes {"true_positive":3,"false_positive":0,"cannot_verify":0}
**Preconditions (4):**
- privileged bee launcher
- model-controlled tool child
- initial executable passes check
- no external privilege drop
**Threat-model match:** root/capability retention and sandbox escape
**Why:** Every model tool reaches pre_exec_hardening through crates/userspace/src/spawn.rs:91-98, but crates/userspace/src/hardening.rs:29-32 only changes dump settings. No UID/GID/capability drop or no_new_privs exists, and the privileged-image check covers only the initial executable.

Ranking: local access; 4 preconditions; threat match: root/capability retention and sandbox escape. Derived MEDIUM.
**Reachability evidence:** crates/userspace/src/spawn.rs:93

### [MEDIUM] UDP sendto bypasses the connect-only egress allowlist  (f003)
`crates/ebpf/src/main.rs:92` | network-policy-bypass | claimed HIGH (alignment -2) | confidence 9.7/10
**Owner:** top committer: jg (1/1 recent commits); no CODEOWNERS entry
**Verdict:** exploitable, votes {"true_positive":3,"false_positive":0,"cannot_verify":0}
**Preconditions (4):**
- enforced network policy
- model-controlled scoped process
- connectionless UDP send
- reachable destination
**Threat-model match:** unauthorized network access and exfiltration
**Why:** The loader attaches only socket_connect, file_open, and bprm_check_security at crates/userspace/src/loader.rs:13-32. Model-controlled scoped children from src/tools/bash.rs:43-55 can use connectionless UDP sendto/sendmsg without traversing crates/ebpf/src/main.rs:92-139.

Ranking: local access; 4 preconditions; threat match: unauthorized network access and exfiltration. Derived MEDIUM.
**Reachability evidence:** crates/userspace/src/loader.rs:14, crates/userspace/src/loader.rs:23, src/tools/bash.rs:48

### [MEDIUM] Untrusted project config becomes an unbounded execution policy without a user ceiling  (f016)
`src/app/config/mod.rs:364` | auth-bypass | claimed HIGH (alignment -2) | confidence 9.7/10
**Owner:** top committer: jg (1/1 recent commits); no CODEOWNERS entry
**Verdict:** exploitable, votes {"true_positive":3,"false_positive":0,"cannot_verify":0}
**Preconditions (4):**
- attacker-controlled repository config
- operator opens repository
- no trusted ceiling
- model exercises requested authority
**Threat-model match:** malicious-repository supply chain and authority widening
**Why:** Project configuration is explicitly untrusted yet may set policy.path and is auto-loaded from .bee/config.toml at src/app/config/file.rs:21-27,79-90,265-285. src/app/config/mod.rs:353-367 accepts that requested policy unchanged when no trusted ceiling exists, and REPL/session startup compiles it into authority.

Ranking: local access; 4 preconditions; threat match: malicious-repository supply chain and authority widening. Derived MEDIUM.
**Reachability evidence:** src/app/repl.rs:120

### [MEDIUM] Reactive retry removes the triggering denial from call evidence  (f037)
`src/episode.rs:386` | audit-misattribution | claimed MEDIUM (alignment +5) | confidence 9.5/10
**Owner:** top committer: jg (1/1 recent commits); no CODEOWNERS entry
**Verdict:** exploitable, votes {"true_positive":2,"false_positive":1,"cannot_verify":0}
**Preconditions (4):**
- initial denial
- reactive escalation enabled
- grant within ceiling
- retry completes
**Threat-model match:** audit and evaluation integrity
**Why:** After a denial-triggered grant, src/episode.rs:376-387 replaces call-local audit with retry_audit. RecordedCall, ScoreReport, and enforcement_trace read call.audit rather than the preserved global audit trail, so a successful retry deterministically removes the triggering denial from derived evidence and scores.

Ranking: authenticated access; 4 preconditions; threat match: audit and evaluation integrity. Derived MEDIUM.
**Reachability evidence:** src/episode.rs:338, src/episode.rs:380, src/episode.rs:700

### [MEDIUM] Unknown policy fields are silently ignored  (f033)
`crates/core/src/policy.rs:104` | policy-validation-bypass | claimed MEDIUM (alignment +2) | confidence 9.5/10
**Owner:** top committer: jg (1/1 recent commits); no CODEOWNERS entry
**Verdict:** exploitable, votes {"true_positive":2,"false_positive":1,"cannot_verify":0}
**Preconditions (3):**
- misspelled security field
- policy launched without detection
- workload exercises defaulted capability
**Threat-model match:** policy integrity and fail-open authority
**Why:** Policy and nested security structs lack deny_unknown_fields at crates/core/src/policy.rs:78-125. A misspelled restrictive key defaults the section empty, which clears network enforcement or omits EXEC_ALLOW and reaches fail-open hooks; two verifiers found this a concrete enforcement loss rather than typo-only hardening.

Ranking: local access; 3 preconditions; threat match: policy integrity and fail-open authority. Derived MEDIUM.
**Reachability evidence:** src/app/config/mod.rs:397

### [MEDIUM] File-open-only mediation permits metadata mutation and hard-link path aliasing  (f005)
`crates/ebpf/src/main.rs:142` | filesystem-policy-bypass | claimed HIGH (alignment -3) | confidence 9.3/10
**Owner:** top committer: jg (1/1 recent commits); no CODEOWNERS entry
**Verdict:** exploitable, votes {"true_positive":3,"false_positive":0,"cannot_verify":0}
**Preconditions (4):**
- pathname-only enforcement
- model-controlled process
- host DAC permits metadata or hard-link operation
- allowed alias path
**Threat-model match:** host data disclosure or integrity loss
**Why:** Only file_open mediates filesystem access in crates/userspace/src/loader.rs:13-32. The hook authorizes only the resolved alias path at crates/ebpf/src/main.rs:142-180, leaving metadata operations uncovered and allowing hard-link aliases to escape protected-name rules.

Ranking: local access; 4 preconditions; threat match: host data disclosure or integrity loss. Derived MEDIUM.
**Reachability evidence:** crates/userspace/src/loader.rs:14, crates/userspace/src/loader.rs:23, src/tools/bash.rs:48

### [MEDIUM] Network-enforced scopes allow every non-IP socket family  (f034)
`crates/ebpf/src/main.rs:126` | network-policy-bypass | claimed MEDIUM (alignment +2) | confidence 9.2/10
**Owner:** top committer: jg (1/1 recent commits); no CODEOWNERS entry
**Verdict:** exploitable, votes {"true_positive":3,"false_positive":0,"cannot_verify":0}
**Preconditions (3):**
- network-enforced attacker process
- sensitive AF_UNIX endpoint
- DAC permits connect
**Threat-model match:** local-service access and possible escape
**Why:** With FLAG_NET_ENFORCED, crates/ebpf/src/main.rs:93-126 checks only IPv4/IPv6 and explicitly allows every other family. Model/tool processes in the enforced cgroup can therefore reach accessible AF_UNIX agents or local control services without NET_ALLOW or a compensating hook.

Ranking: local access; 3 preconditions; threat match: local-service access and possible escape. Derived MEDIUM.
**Reachability evidence:** crates/userspace/src/loader.rs:23, src/tools/exec.rs:25

### [MEDIUM] Post-hoc nesting validation permits recursive clone amplification  (f039)
`src/render_api.rs:831` | algorithmic-complexity | claimed MEDIUM (alignment +5) | confidence 9.0/10
**Owner:** top committer: jg (1/1 recent commits); no CODEOWNERS entry
**Verdict:** exploitable, votes {"true_positive":3,"false_positive":0,"cannot_verify":0}
**Preconditions (3):**
- render tool enabled
- script reuses nested builders
- clone growth precedes validation
**Threat-model match:** algorithmic denial of service
**Why:** Model-controlled Rhai runs in-process. layout.add converts and deep-clones existing RenderSpec trees at src/render_api.rs:395-400,627-633,829-835, while structural validation happens only at final commit; cheap repeated reuse can amplify native clone work beyond Rhai operation accounting.

Ranking: authenticated access; 3 preconditions; threat match: algorithmic denial of service. Derived MEDIUM.
**Reachability evidence:** src/tools/render.rs:259

### [MEDIUM] Relative filesystem restrictions cannot match resolved kernel paths  (f043)
`crates/core/src/compiler.rs:160` | filesystem-policy-bypass | claimed MEDIUM (alignment +5) | confidence 9.0/10
**Owner:** top committer: jg (1/1 recent commits); no CODEOWNERS entry
**Verdict:** exploitable, votes {"true_positive":2,"false_positive":1,"cannot_verify":0}
**Preconditions (4):**
- enforcement active
- relative restrictive rule
- no matching absolute protection
- attacker accesses absolute target
**Threat-model match:** policy divergence and host data exposure
**Why:** Policy validation accepts relative filesystem keys; resolve_tokens leaves them unchanged and planning installs those bytes, while enforcement compares absolute bpf_d_path output. Two verifiers found this silently defeats restrictive rules, although one treated the source as trusted operator misconfiguration.

Ranking: authenticated access; 4 preconditions; threat match: policy divergence and host data exposure. Derived MEDIUM.
**Reachability evidence:** crates/core/src/compiler.rs:97, src/app/session.rs:312

### [MEDIUM] Lazy skill-body reads can be redirected to arbitrary host files  (f050)
`src/skills.rs:110` | path-traversal | claimed HIGH (alignment -3) | confidence 9.0/10
**Owner:** top committer: jg (2/2 recent commits); no CODEOWNERS entry
**Verdict:** exploitable, votes {"true_positive":2,"false_positive":1,"cannot_verify":0}
**Preconditions (4):**
- skill discovered
- attacker mutates path before invocation
- target readable UTF-8
- skill invoked
**Threat-model match:** skill TOCTOU and confused-deputy host read
**Why:** Discovery follows links and stores a mutable pathname, while invocation later performs a fresh host-side read without canonical containment or inode pin at src/skills.rs:106-114. Two verifiers found the long model/tool-turn window attacker-controllable; one considered enforced directory read-only policy sufficient and the race theoretical.

Ranking: local access; 4 preconditions; threat match: skill TOCTOU and confused-deputy host read. Derived MEDIUM.
**Reachability evidence:** src/tools/skill.rs:109

### [MEDIUM] Terminal control characters in skill metadata can forge consent displays  (f046)
`src/app/repl.rs:380` | consent-bypass | claimed MEDIUM (alignment +5) | confidence 8.7/10
**Owner:** top committer: jg (1/1 recent commits); no CODEOWNERS entry
**Verdict:** exploitable, votes {"true_positive":3,"false_positive":0,"cannot_verify":0}
**Preconditions (4):**
- malicious project skill metadata
- request within ceiling
- interactive terminal prompt
- operator approves forged display
**Threat-model match:** consent-decision integrity
**Why:** Project skill metadata is accepted without control-character validation and printed verbatim at src/app/repl.rs:378-391 in the authoritative y/N consent prompt. ANSI or newline sequences can conceal or forge the capability request immediately before approval; the ceiling does not replace this second consent boundary.

Ranking: local access; 4 preconditions; threat match: consent-decision integrity. Derived MEDIUM.
**Reachability evidence:** src/app/repl.rs:179, src/app/repl.rs:184

### [MEDIUM] Repeated panel commits clone large widgets without an aggregate limit  (f051)
`src/render_api.rs:1023` | resource-exhaustion | claimed MEDIUM (alignment +4) | confidence 8.5/10
**Owner:** top committer: jg (1/1 recent commits); no CODEOWNERS entry
**Verdict:** exploitable, votes {"true_positive":2,"false_positive":1,"cannot_verify":0}
**Preconditions (4):**
- render tool enabled
- large valid spec
- many commits under op limit
- clones exhaust memory
**Threat-model match:** algorithmic denial of service
**Why:** Every render_to appends an owned RenderSpec to an uncapped panel_ops vector, while validation is per-widget rather than aggregate. Two verifiers found thousands of large retained commits a valid algorithmic memory amplification despite the 10,000-operation cap; one treated the cap as bounded volumetric DoS.

Ranking: authenticated access; 4 preconditions; threat match: algorithmic denial of service. Derived MEDIUM.
**Reachability evidence:** src/tools/render.rs:259

### [MEDIUM] Fixed-delay audit draining can lose or misattribute records  (f036)
`src/episode.rs:334` | audit-loss | claimed MEDIUM (alignment +5) | confidence 8.4/10
**Owner:** top committer: jg (1/1 recent commits); no CODEOWNERS entry
**Verdict:** exploitable, votes {"true_positive":3,"false_positive":0,"cannot_verify":0}
**Preconditions (4):**
- concurrent async-audit path
- audit event emitted
- delivery exceeds five milliseconds
- drain or teardown occurs first
**Threat-model match:** incomplete enforcement evidence
**Why:** run_loop treats a fixed five-millisecond sleep and nonblocking drain as a call boundary at src/episode.rs:323-338 and src/sandbox.rs:196-205. Asynchronous kernel-to-demux-to-channel delivery has no acknowledgement or watermark, so delayed denials can attach to later calls or disappear at teardown.

Ranking: authenticated access; 4 preconditions; threat match: incomplete enforcement evidence. Derived MEDIUM.
**Reachability evidence:** src/episode.rs:336

### [MEDIUM] Attacker-triggerable path-resolution failure fails open  (f007)
`crates/ebpf/src/main.rs:224` | exec-policy-bypass | claimed HIGH (alignment -3) | confidence 8.3/10
**Owner:** top committer: jg (1/1 recent commits); no CODEOWNERS entry
**Verdict:** exploitable, votes {"true_positive":3,"false_positive":0,"cannot_verify":0}
**Preconditions (4):**
- nonempty exec policy
- writable path over 4096 resolved bytes
- reachable disallowed executable
- relative invocation
**Threat-model match:** executable-policy bypass and escape
**Why:** Exec enforcement returns allow on bpf_d_path error at crates/ebpf/src/main.rs:218-226 and uses a fixed 4096-byte buffer. A model-controlled process reachable from src/tools/bash.rs:43-55 can build a longer resolved directory chain and invoke a short relative executable without any fallback identity check.

Ranking: local access; 4 preconditions; threat match: executable-policy bypass and escape. Derived MEDIUM.
**Reachability evidence:** crates/userspace/src/loader.rs:14, src/tools/bash.rs:48

### [MEDIUM] Remote MCP redirects bypass the destination allowlist  (f049)
`src/mcp/bridge.rs:332` | ssrf | claimed MEDIUM (alignment +3) | confidence 7.5/10
**Owner:** top committer: jg (1/1 recent commits); no CODEOWNERS entry
**Verdict:** needs_manual_test, votes {"true_positive":2,"false_positive":0,"cannot_verify":1}
**Preconditions (4):**
- allowlisted attacker MCP server
- redirect to denied destination
- transport follows redirect
- destination reachable
**Threat-model match:** redirect destination revalidation
**Why:** connect_remote validates only the configured initial URL at src/mcp/bridge.rs:321-349 and provides no per-hop policy callback. Two verifiers concluded an allowed but untrusted MCP endpoint can redirect toward a denied/internal destination; one could not statically verify whether the external transport follows redirects.

Ranking: authenticated access; 4 preconditions; threat match: redirect destination revalidation. Derived MEDIUM.
**Reachability evidence:** src/app/repl.rs:209, src/mcp/bridge.rs:156
> Recommend a human build a PoC; static reasoning hit its limit.

## Dropped

| id | title | file:line | why dropped |
|---|---|---|---|
| f004 | Empty network allowlist disables all egress enforcement | crates/ebpf/src/main.rs:99 | duplicate of f003 |
| f006 | Empty executable allowlist disables execution enforcement | crates/ebpf/src/main.rs:201 | duplicate of f014 |
| f011 | Background descendants survive teardown and become unrestricted | crates/userspace/src/lib.rs:332 | duplicate of f022 |
| f013 | Removing the final child write grant disables default-deny writes | crates/userspace/src/plan.rs:48 | duplicate of f008 |
| f015 | Privileged children can migrate outside the exact cgroup policy key | crates/userspace/src/spawn.rs:92 | duplicate of f026 |
| f019 | Scenario workdir setup performs host writes before sandbox creation | src/episode.rs:520 | duplicate of f020 |
| f024 | Repository-controlled scenario IDs escape batch output containment | src/app/run.rs:408 | duplicate of f023 |
| f027 | Unix-domain sockets bypass the network policy | crates/ebpf/src/main.rs:126 | duplicate of f034 |
| f032 | A child can disable the parent exfiltration controls | crates/core/src/attenuation.rs:57 | duplicate of f001 |
| f042 | Concurrent audit records share one synthetic scope ID | src/concurrent.rs:114 | duplicate of f052 |
| f044 | Scope teardown leaves cgroup-keyed authorization entries behind | crates/userspace/src/lib.rs:331 | duplicate of f045 |
| f047 | Unescaped project skill metadata can forge the capability prompt | src/app/repl.rs:380 | duplicate of f046 |
| f008 | Zero-capability filesystem policy defaults to broad read/write access | crates/ebpf/src/main.rs:331 | intentional_behavior; exclusion rule 3 |
| f017 | Discovered skill directories are made readable after attenuation | src/app/session.rs:152 | intentional_behavior, not_actionable; exclusion rule 3 |
| f021 | Tool grants bypass the attenuation ceiling and scenario tool allowlist | src/skills/grant.rs:161 | intentional_behavior; exclusion rule 3 |
| f029 | Headless episodes grant every discovered skill before invocation | src/episode.rs:599 | intentional_behavior; exclusion rule 3 |
| f031 | Unselected repository skills inject instructions into every tool schema | src/tools/skill.rs:48 | not_actionable; exclusion rule 6 |
| f035 | Failed scope reload can retain kernel grants after userspace rollback | crates/userspace/src/lib.rs:200 | implausible_trigger, not_actionable; exclusion rule 8 |
| f038 | Hand-written URL parsing can authorize a different host than the transport | src/mcp/policy.rs:78 | implausible_trigger; exclusion rule 8 |
| f045 | Cgroup ID reuse can combine new scopes with stale map state | crates/userspace/src/lib.rs:332 | implausible_trigger; exclusion rule 16 |
| f048 | Default progress logs expose tool arguments, results, and CTF flags | src/episode.rs:284 | intentional_behavior, not_actionable; exclusion rule 12 |
| f052 | Concurrent audit events receive a shared synthetic scope identifier | crates/userspace/src/async_events.rs:77 | not_actionable; exclusion rule 12 |
| f053 | Stdio MCP servers run with host-user authority in non-enforce builds | src/episode.rs:620 | intentional_behavior; exclusion rule 3 |

