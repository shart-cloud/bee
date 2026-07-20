<!--
Sync Impact Report
==================
Version change: (uninitialized template) → 1.0.0
Bump rationale: Initial ratification of the project constitution (first concrete version).

Modified principles: N/A (initial adoption). Placeholder principles replaced with:
  I. Deny-by-Default & Fail-Closed
  II. Capability Attenuation
  III. Kernel Enforcement Is Authoritative
  IV. Policy-as-Data
  V. Library-First, Runtime-Free Core

Added sections:
  - Security & Platform Constraints (replaces [SECTION_2_NAME])
  - Development Workflow & Quality Gates (replaces [SECTION_3_NAME])

Removed sections: None.

Templates requiring updates:
  ✅ .specify/templates/plan-template.md — "Constitution Check" reads gates dynamically; no hardcoded
     principle names, no edit required. Plan authors must populate gates from Principles I–V + gates below.
  ✅ .specify/templates/spec-template.md — principle-agnostic; no change required.
  ✅ .specify/templates/tasks-template.md — principle-agnostic; no change required.
  ✅ .claude/skills/speckit-*/SKILL.md — load the constitution generically; no outdated references.

Follow-up TODOs: None. Ratification date set to project creation date (2026-07-19).
-->

# bee Constitution

## Core Principles

### I. Deny-by-Default & Fail-Closed

Every scope begins with zero capabilities. Access is granted only by an explicit policy rule;
the absence of a rule is a denial, never a permission. When bee cannot prove that an operation
is safe — a kernel capability is missing, an attenuation subset cannot be verified, a target
process is privileged, a policy fails to compile — it MUST fail closed: refuse to run or deny
the operation, never degrade silently into weaker or absent enforcement.

Rationale: A sandbox that fails open provides false assurance, which is worse than no sandbox.
Correctness of the security boundary is the product; convenience never overrides it.

### II. Capability Attenuation

A derived (subagent) policy MUST be a strict subset of its parent. bee MUST reject any
derivation that would grant a capability the parent does not hold, and MUST make that rejection
before enforcement begins. When subset containment cannot be decided (e.g. overlapping glob
rules), the validator treats the derivation as invalid (fail-closed per Principle I). The
attenuation relationship MUST be covered by property-based tests, not only examples.

Rationale: Monotonic narrowing of authority is the invariant that makes subagent delegation
trustworthy; it is bee's core differentiator and must be provable, not merely intended.

### III. Kernel Enforcement Is Authoritative

Policy decisions that constrain a sandboxed process MUST be enforced in the kernel (eBPF LSM
hooks), where unprivileged user-space code cannot bypass them. User-space components compile,
load, update, and observe policy; they MUST NOT be the enforcement point. Any capability that
cannot be enforced in the kernel MUST NOT be presented to callers as an enforced guarantee.

Rationale: The threat model assumes the sandboxed tool call may be malicious. Only a boundary
below the process it constrains can hold.

### IV. Policy-as-Data

Policy MUST be declarative, human-readable, versionable, diffable, and auditable text — never
imperative code or opaque binary. The compiled/kernel representation is derived from that source
of truth and is never authored directly. A policy's meaning MUST be reviewable without running it.

Rationale: Security policy that cannot be read, diffed, and reviewed in a pull request cannot be
trusted or governed.

### V. Library-First, Runtime-Free Core

bee is a library that agent frameworks embed; the CLI is a thin wrapper over the same public
entry points and MUST NOT contain enforcement logic the library lacks. The core policy engine
MUST NOT require an async runtime — async and external I/O exporters are opt-in layers built
atop a synchronous core. Each crate MUST be independently testable with a clear, single purpose.

Rationale: Embeddability into diverse harnesses, and freedom from a forced runtime dependency,
are preconditions for adoption; the CLI must never become a privileged side channel.

## Security & Platform Constraints

- **Target platform**: Linux with eBPF LSM (`CONFIG_BPF_LSM=y`) and cgroup v2. The MVP is
  eBPF-LSM-only; on unsupported kernels bee refuses to start with a clear diagnostic. macOS and
  Windows are out of scope.
- **Scope isolation**: Each tool call or subagent is confined to its own bee-managed cgroup v2
  group. bee owns the lifecycle of the cgroup subtree it creates and reclaims.
- **Privileged targets**: bee MUST refuse to attach a policy to a setuid target or a process
  holding broad system capabilities, rather than provide false assurance.
- **Process hardening**: Before a sandboxed process runs its main program, bee MUST disable core
  dumps, mark the process non-dumpable, and strip dynamic-linker-influencing environment variables.
- **Protected paths**: Within any writable root, version-control metadata, bee's own
  configuration, and well-known credential directories (SSH, cloud credentials) are read-only or
  denied by default unless a policy explicitly grants access.
- **Auditability**: Every denied or flagged operation MUST emit a structured, machine-readable
  audit event. Audit-only (dry-run) mode observes-and-records without blocking, but never changes
  the enforcement default of a non-dry-run scope.

## Development Workflow & Quality Gates

- **Test-first for the security boundary**: Enforcement behavior (allow/deny outcomes, attenuation
  rejection, fail-closed paths) MUST have tests written and failing before implementation. The
  attenuation validator MUST have property-based tests. Enforcement claims MUST be verified by
  integration tests on a BPF_LSM-capable kernel, not by unit mocks alone.
- **Every security-relevant behavior is a testable requirement**: A denial that no test exercises
  is not considered enforced.
- **Performance budgets are gates, not aspirations**: Per-hook enforcement overhead and policy
  compilation time carry explicit measurable limits (see spec Success Criteria) and are validated
  by benchmark tests.
- **Toolchain discipline**: User-space crates MUST build on stable Rust. Nightly is permitted only
  where the eBPF probe toolchain genuinely requires it, confined to the eBPF crate.
- **Constitution Check in planning**: Each `/speckit-plan` MUST evaluate the design against
  Principles I–V and the constraints above, and record any deviation with written justification in
  the plan's Complexity Tracking. Unjustified violations block the plan.

## Governance

This constitution supersedes other process conventions for the bee project. It governs how
features are specified, planned, implemented, and reviewed.

- **Amendments**: Proposed as a change to this file with rationale. Amendments that remove or
  redefine a principle require explicit acknowledgement of the security implications.
- **Versioning policy** (semantic):
  - MAJOR — a principle is removed or redefined in a backward-incompatible way.
  - MINOR — a principle or materially new section is added, or guidance is materially expanded.
  - PATCH — clarifications, wording, and non-semantic refinements.
- **Compliance review**: Every plan and its resulting implementation MUST be checked against these
  principles. Where a principle and expedience conflict, the principle wins or the work does not
  ship. Complexity that violates a principle MUST be justified in writing or removed.

**Version**: 1.0.0 | **Ratified**: 2026-07-19 | **Last Amended**: 2026-07-19
