# Fail-closed configuration and explicit host mode

Status: ready-for-agent

Part of [Bee application and workspace consolidation](../PRD.md).

## Problem Statement

Bee's first design principle is deny-by-default and fail-closed: refuse rather than degrade
silently. The `exec` path honours it — it refuses to run when the kernel cannot enforce, and says
so. The session paths do not.

An interactive session started without a policy becomes a host session with no kernel scope. It
runs. The only signal is a startup banner line reading `policy: none — host mode (no kernel scope)`,
sitting among five other banner lines, in a session the operator is about to scroll past. A session
started *with* a policy against a binary built without the enforcement feature is worse: the policy
is parsed, accepted, and then ignored, and the banner says so in a parenthetical. In both cases the
agent then executes tools, and the operator's belief about what was enforcing is whatever they
inferred from a line of startup text.

This is the gap ADR-0002's working design names directly: bee must not silently fall back to
unenforced execution, and host mode must require loud, explicit operator intent. Issue 02 built the
resolver that can tell whether the configuration is complete; this issue is where that becomes a
refusal.

## Solution

Make unenforced execution something the operator asks for. Add a `--host` flag to `bee run` and
`bee repl` meaning "run the agent's tools as hardened, credential-stripped host processes with no
kernel scope, and I know that". Without it, a session that cannot enforce is a startup error that
names why it cannot and what would fix it. With it, the session starts and says plainly, once, that
nothing is enforcing.

Three cases produce three distinct errors, because three distinct things are wrong:

- No policy configured and none requestable from any configuration source. The error names the
  missing requirement and the places it could be supplied — a flag, project configuration, user
  configuration — reusing issue 02's missing-requirement reporting rather than inventing a second
  error vocabulary.
- A policy configured, but the binary was built without enforcement. The error says the policy
  cannot be enforced by this build and names the two ways forward: rebuild with the enforcement
  feature, or drop the policy and pass `--host`.
- A policy configured and enforcement compiled in, but the kernel cannot enforce. This already
  fails closed in `exec`; the sessions adopt the same behaviour and the same diagnostic output.

`--host` and a configured policy are contradictory, so combining them is an error rather than a
precedence question. Asking for a policy while asking to run unenforced is a mistake worth
surfacing, not a preference to resolve.

`--host` is flag-only. It is not settable from project configuration, and not from user
configuration either — a decision to run an agent unenforced belongs to the invocation, not to a
file that might have been written months ago or by a repository. This makes it the strictest field
in the trust classification issue 02 established.

The invariant lives at the command boundary. The library entry points that run an episode or a REPL
loop keep taking whatever sandbox they are given, which is why the harness's existing integration
tests — which construct sessions directly rather than through a command — are unaffected. What does
need updating is anything that drives the commands without a policy, which after this issue must say
`--host`.

## Commits

1. **Add the flag and the conflict.** Add `--host` to both session commands and make it conflict
   with a configured policy. Add the trust rule that rejects `host` appearing in any configuration
   file. Tests for the flag alone, the flag with a policy, and the flag in each configuration layer.

2. **Refuse an incomplete session configuration.** Make a session with no policy and no `--host` a
   startup error carrying the missing-requirement report. Test the error text names at least one
   place the policy could be supplied.

3. **Refuse an unenforceable policy.** Replace the host-build path that accepts and ignores a policy
   with an error naming the build and the two remedies. Test under both feature configurations —
   this is the one behaviour that differs by build, so it needs asserting in both.

4. **Adopt the fail-closed kernel check.** Give the sessions the same unsupported-kernel refusal and
   diagnostic output `exec` already produces, so a kernel that cannot enforce stops a session the
   same way it stops a diagnostic run.

5. **State host mode plainly.** Replace the banner parenthetical with a single explicit line, on
   stderr, stating that no kernel scope is in effect and that tools run as hardened host processes.
   It should be legible in a piped log, not only in a decorated banner.

6. **Update the callers.** Sweep the repository for command invocations that relied on the silent
   fallback — the VM scripts, the README examples, the harness README, and the design-system doc —
   and add `--host` where the intent was genuinely an unenforced run.

## Verification

- `cargo test` and `cargo clippy --workspace --all-targets` green, in both the default and enforce
  builds. Both matter here: the point of commit 3 is that the two builds now behave differently and
  say so.
- The harness's existing integration tests pass unchanged — evidence that the invariant landed at
  the command boundary and not in the library.
- Every negative case has a test asserting the error text, not merely the exit code. The error text
  is the feature; an exit code alone leaves the operator exactly where the banner did.
- The live VM matrix (`test/vm/matrix.sh`) passes, including a new case that a session started
  without a policy and without `--host` refuses to start.
