# Feature Specification: bee — eBPF-Enforced Sandbox Harness for Coding Agents

**Feature Branch**: `001-ebpf-agent-sandbox`

**Created**: 2026-07-19

**Status**: Draft

**Input**: User description: "bee — eBPF-Enforced Sandbox Harness for Coding Agents. A Rust library and CLI providing kernel-enforced, capability-scoped sandboxing for AI coding agents using eBPF LSM hooks (via Aya). Enforces per-tool-call and per-subagent security policies: file access control, process exec allowlisting, network egress filtering, and cross-domain exfiltration detection — without containers, VMs, or namespace overhead."

## Overview

bee gives an AI coding agent a way to run real, untrusted tool calls (compilers, package managers, test suites) while a kernel-enforced policy constrains exactly what each call may read, write, execute, and reach over the network. Policies start with zero capabilities (deny-by-default), are declared as versionable data, and can be attenuated — never broadened — when a parent agent delegates work to a subagent. Enforcement lives in the kernel, so a compromised or malicious tool call cannot bypass it from user space.

## Clarifications

### Session 2026-07-19

- Q: Should the degraded (Landlock + seccomp) fallback ship in the MVP, or is the MVP eBPF-LSM-only? → A: MVP is eBPF-LSM-only; on kernels lacking eBPF LSM support bee refuses to start (fail-closed) with a clear diagnostic. The degraded fallback is deferred to post-MVP.
- Q: Who owns the cgroup v2 hierarchy used for per-scope confinement? → A: bee creates and manages its own cgroup v2 subtree under a configurable parent cgroup; each scope is one bee-created cgroup. The caller supplies only a policy and a command.
- Q: What interface consumes the audit event stream in the MVP? → A: A synchronous, blocking iterator/reader over audit events (no async runtime required in the core), with events serializable to JSON. Async `Stream` and socket exporters are opt-in and deferred to post-MVP.
- Q: Does the filesystem policy support glob patterns, or only exact paths and subtrees? → A: Support glob patterns (e.g. `*.log`, `**/target`) in addition to exact paths and subtree prefixes. Rule precedence remains narrowest/most-specific match wins; the compiler includes a glob-matching layer beyond longest-prefix lookup.
- Q: Does the MVP include an audit-only (dry-run) mode? → A: Yes — a per-scope audit-only flag makes enforcement emit the audit event but allow the operation, for policy development against real workloads. Enforcement programs carry a per-scope enforce/observe mode flag.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Single-Agent Tool Call Sandbox (Priority: P1)

A developer runs a coding agent in a project directory. The agent needs to execute a build/test command (e.g. `cargo test`). Before spawning that command, the operator (or the agent framework) hands bee a policy describing what the command is allowed to do: read/write within the project directory, execute a named set of build tools, reach a named set of package-registry hosts, and nothing else. bee confines the spawned process to exactly those capabilities. Any attempt to read a secret file, execute an un-listed binary, or connect to an un-listed destination is denied by the kernel and recorded in an audit log.

**Why this priority**: Sandboxing a single tool call is the core value proposition and the minimum viable product. Everything else builds on it.

**Independent Test**: Run a test binary under bee with a restrictive policy on a capable kernel. Verify allowed operations succeed and denied operations fail with a permission error, and that each denial produces a structured audit event.

**Acceptance Scenarios**:

1. **Given** a policy granting write access to the project directory and execution of `cargo`, **When** the sandboxed process attempts to open `~/.ssh/id_rsa` for reading, **Then** the operation is denied with a permission error and an audit event is emitted.
2. **Given** a policy granting network egress only to a named package registry, **When** the sandboxed process attempts to connect to an address not on the allowlist, **Then** the connection is denied.
3. **Given** a policy allowing execution of `cargo` and `rustc`, **When** the sandboxed process attempts to execute an un-listed binary (e.g. a network client), **Then** the execution is denied.
4. **Given** an enforcing policy, **When** allowed operations run (write within the project, exec `cargo`, connect to an allowed host), **Then** they all succeed without sandbox-caused failures.

---

### User Story 2 - Subagent Permission Scoping (Priority: P2)

A parent agent holding broad project access spawns a subagent to run tests in an isolated scratch directory. The subagent is granted an attenuated policy: read-only on the project source, read-write only on its own scratch directory, execution limited to the test runner, and no network. The system guarantees the subagent's policy is a strict subset of the parent's — the parent cannot grant the subagent anything the parent does not itself hold.

**Why this priority**: Per-subagent scoping is the key differentiator from existing agent harnesses, but it depends on US1's single-call enforcement working first.

**Independent Test**: Derive a subagent policy from a parent policy and confine a child process to it. Verify the child cannot exceed the parent's permissions and cannot touch paths outside its own grant. Verify that any attempt to derive a broader-than-parent policy is rejected before enforcement.

**Acceptance Scenarios**:

1. **Given** a parent policy with write access to the project and network access to a registry, **When** a subagent policy is requested that adds write access to a system file the parent cannot write, **Then** the derivation is rejected as an attenuation violation.
2. **Given** a subagent policy with read-only project source and read-write scratch directory, **When** the subagent attempts to write to a project source file, **Then** the write is denied.
3. **Given** a subagent policy with no network access, **When** the subagent attempts any outbound connection, **Then** it is denied regardless of destination.

---

### User Story 3 - Exfiltration Detection (Priority: P3)

During a build step, a poisoned dependency reads a sensitive credential file and then attempts to transmit its contents to an external server. bee correlates the sensitive-file read with the subsequent network send within the same sandboxed scope, blocks the transmission, and raises an exfiltration alert.

**Why this priority**: This is the supply-chain-attack detection scenario — high value, but it requires both file and network enforcement (US1) to be working and correlated. It is deferred to a second release.

**Independent Test**: Run a program that reads a sentinel sensitive file and then attempts to send its contents over a socket, under a policy with exfiltration detection enabled. Verify the network send is blocked and an exfiltration alert is emitted. Verify a program that reads only non-sensitive files and sends normal traffic raises no alert.

**Acceptance Scenarios**:

1. **Given** exfiltration detection enabled with a sensitive-paths list, **When** a sandboxed process reads a listed file and subsequently attempts a network send, **Then** the send is blocked and an exfiltration event is emitted to the audit log.
2. **Given** a sandboxed process that reads only non-sensitive project files and sends traffic to an allowed host, **When** it transmits, **Then** no exfiltration alert is raised (false-positive avoidance).

---

### Edge Cases

- **Kernel lacks eBPF LSM support** (older kernel or feature disabled): the MVP MUST detect this at startup and refuse to start with a clear diagnostic — never silently run with no enforcement. (A degraded fallback is a post-MVP option, not MVP behavior.)
- **Privileged target process** (the process to be sandboxed is setuid or holds broad system capabilities): bee MUST refuse to attach a policy to it rather than provide false assurance.
- **Symlink traversal**: a writable path contains a symlink pointing at a protected path. Enforcement operates on resolved paths; this behavior MUST be documented and covered by tests.
- **cgroup v2 unavailable** (cgroup v1-only host): per-scope confinement requires cgroup v2. bee MUST detect its absence and fail with a clear message.
- **Glob patterns and attenuation**: proving a derived policy's glob rules are a subset of a parent's glob rules is non-trivial. The attenuation validator MUST be conservative — when subset containment cannot be proven, the derivation MUST be rejected rather than assumed valid (fail-closed), so SC-002 continues to hold.
- **Network allowlist by hostname**: destination hosts are named, but enforcement acts on addresses. Address resolution happens outside the kernel at policy-load time and is refreshed on an interval; policy authors MUST be able to understand and audit which addresses a named host resolved to.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: bee MUST enforce per-scope file access policy (read / write / deny) on file-access operations, resolving each rule by path with narrower (more specific) rules overriding broader ones. Rules MUST support exact paths, subtree prefixes (a directory rule covers everything beneath it), and glob patterns (e.g. `*.log`, `**/target`). When multiple rules match a path, the most specific match wins.
- **FR-002**: bee MUST enforce a per-scope process-execution allowlist, permitting only explicitly listed executables to run.
- **FR-003**: bee MUST enforce per-scope network egress policy, allowing outbound connections only to explicitly allowed destinations (host:port), with deny-all implied whenever an allowlist is present.
- **FR-004**: bee MUST scope every policy to a single tool call or subagent, so that distinct calls are independently confined and cannot interfere with each other's policy. bee MUST create and manage its own cgroup v2 subtree under a configurable parent cgroup, allocating one cgroup per scope and reclaiming it when the scope ends; the caller supplies only a policy and a command.
- **FR-005**: bee MUST support policy attenuation: any derived (subagent) policy MUST be validated to be a subset of its parent, and derivations that would grant more than the parent holds MUST be rejected before enforcement.
- **FR-006**: bee MUST support updating an active policy's allowed-capability data without tearing down and restarting the sandboxed process.
- **FR-007**: bee MUST emit a structured, machine-readable audit event (JSON-serializable) for every denied (or flagged) operation, including the operation type, the target path or address, the scope identifier, and a timestamp. bee MUST expose these events through a synchronous, blocking iterator/reader that does not require an async runtime; async-stream and socket-based exporters are out of MVP scope.
- **FR-008**: bee MUST, by default within any writable root, protect version-control metadata, bee's own configuration, and well-known credential directories (SSH, cloud credentials) as read-only or denied unless a policy explicitly grants access.
- **FR-009**: bee MUST detect kernels lacking eBPF LSM enforcement support at startup and report a clear diagnostic rather than running without enforcement.
- **FR-010**: (Post-MVP) bee SHOULD provide a degraded enforcement mode (filesystem + network + no-new-privileges restrictions using platform primitives) when full eBPF LSM enforcement is unavailable. The MVP does NOT implement this fallback; on kernels lacking eBPF LSM support the MVP refuses to start per FR-009.
- **FR-011**: bee MUST harden the target process before its main program runs: disable core dumps, mark the process non-dumpable, and strip dynamic-linker-influencing environment variables.
- **FR-012**: bee MUST (in a later release) correlate a sensitive-file read with a subsequent network send within the same scope and block the send, emitting an exfiltration alert.
- **FR-013**: bee MUST accept policy as declarative, human-readable, versionable data that can be diffed and audited.
- **FR-014**: bee MUST be embeddable by an agent framework as a library, with the CLI being a thin wrapper over the same library entry points.
- **FR-015**: bee MUST refuse to attach a policy to a privileged target process (setuid or holding broad system capabilities).
- **FR-016**: bee MUST support a per-scope audit-only (dry-run) mode in which enforcement emits the audit event for an operation that policy would deny but allows the operation to proceed, enabling policy development against real workloads without breaking them.

### Key Entities *(include if feature involves data)*

- **Policy**: A declarative description of the capabilities a scope is allowed — file paths with access modes, an executable allowlist, allowed network destinations, and (for exfiltration detection) a list of sensitive paths. Human-readable, versionable, diffable.
- **Scope**: The confinement boundary associated with one loaded policy, corresponding to a single tool call or subagent. Distinct scopes are independently enforced.
- **Audit Event**: A structured record of a denied or flagged operation — operation type, target path/address, scope identifier, and timestamp — surfaced to user space for logging and alerting.
- **Attenuation Relationship**: The subset constraint between a parent policy and any policy derived from it; a derived policy is valid only if every capability it grants is also granted by its parent.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: A tool call sandboxed by bee cannot read a well-known secret file (SSH private key, cloud credentials) under any policy that does not explicitly grant access, verified by an integration test suite.
- **SC-002**: A derived subagent policy can provably never exceed its parent's permissions, verified by property-based tests over the attenuation validator.
- **SC-003**: Per-operation enforcement overhead stays under 5 microseconds per hooked operation, measured across at least 10,000 file-open operations under an enforcing policy.
- **SC-004**: bee can sandbox a full build-and-test run of a medium-sized real project (hundreds of files, dozens of dependencies) with zero test failures caused by the sandbox itself — no false denials of legitimate operations.
- **SC-005**: Exfiltration detection flags a simulated supply-chain attack (sensitive read followed by network send) with zero false positives on normal build traffic.
- **SC-006**: Compiling a policy into enforceable form completes in under 50 milliseconds for policies with up to 100 path rules and 50 network rules.
- **SC-007**: On a kernel without eBPF LSM support, the MVP never runs with silently-absent enforcement — it refuses to start with a clear diagnostic identifying the missing kernel capability.

## Assumptions

- Target hosts run a Linux kernel with eBPF LSM enforcement enabled and cgroup v2 available (the systemd default on modern distributions). macOS and Windows are out of scope for the MVP.
- Loading kernel enforcement programs requires elevated privilege (appropriate capabilities or root); once loaded, enforcement is transparent to the unprivileged sandboxed process.
- Agent frameworks integrate bee as a library, establishing a scope from a policy before spawning each tool-call subprocess; the CLI is a thin wrapper for standalone use.
- Named network destinations are resolved to addresses in user space at policy-load time and refreshed on a configurable interval, not resolved inside the kernel.
- Full eBPF LSM enforcement is the only enforcement mode in the MVP; the degraded (Landlock+seccomp) fallback is deferred to post-MVP.
- Exfiltration detection (User Story 3 / FR-012) is deferred to a second release; the MVP delivers User Stories 1 and 2 and the single-call/subagent enforcement they require.
- Policy is authored and stored as declarative text files under the project or operator's control.
