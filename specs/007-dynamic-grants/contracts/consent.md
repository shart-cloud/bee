# Contract: Async Consent + the Escalation Cycle (bee-harness)

Evolves 006-skills' synchronous `ConsentSink` into an async, pollable seam, and specifies the
escalate/reload/retry and narrow cycles that consume it.

## Async ConsentSink

```rust
#[async_trait::async_trait]
pub trait ConsentSink: Send + Sync {
    /// Approve or deny a within-ceiling capability request. MAY poll internally (prompt, queue,
    /// webhook). The loop bounds the await with a timeout that maps elapse → Denied.
    async fn confirm(&self, req: &GrantRequest<'_>) -> Decision;
}
pub enum Decision { Granted, Denied }
```

Built-ins (non-interactive, resolve immediately):
- `DenyAll` → `Denied` (default for automated runs with no ceiling authorization).
- `AllowWithinCeiling` → `Granted` (the ceiling file *is* the authorization; batch/CTF).
- `PromptConsent` (REPL) → interactive y/N, now async.

## The escalation cycle (widen)

Given `Flow::Escalate(delta)` with an optional triggering `ToolCall`:

```text
1. candidate = ActivePolicy.active ∪ delta
2. ceiling.derive(candidate)                 # attenuation — NOT overridable
     Err  → refuse: audit "beyond ceiling"; feed original denial/result to model; STOP
3. plan = EnforcementPlan::prepare(compile(candidate))
     Err (e.g. > DENY_MAX_RULES) → refuse: audit "rule cap"; STOP        # FR-016
4. decision = timeout(escalation_timeout, consent.confirm(req)).await
     Denied/elapsed → audit "denied"; feed original denial/result to model; STOP   # fail-closed
5. Sandbox.reload(&plan); ActivePolicy.apply(delta) → push GrantLease{origin, delta, ttl, turn}
6. register delta.tools (Layer 1)
7. if triggering ToolCall: re-run registry.execute(call) ONCE; record the retry result + escalation
     audit events (grant delta, reload). Model sees the single (successful-or-final) result.  # R8
```

Attenuation (step 2) precedes consent (step 4): a beyond-ceiling request is refused without ever
prompting (spec SC-002, US1-AS-2).

## The narrowing cycle

At `TurnStart` (expiry) or on `Flow::Deescalate(id)`:

```text
1. drop matching lease(s) from ActivePolicy.leases
2. active' = base ∪ (remaining live leases' deltas)
3. ceiling.derive(active')                   # trivially Ok (narrowing); asserted for INV safety
4. plan = EnforcementPlan::prepare(compile(active'))
5. Sandbox.reload(&plan)                      # NET_ALLOW diff removes dropped keys
6. de-register tools no longer granted by any live lease (Layer 1)
7. audit "narrowed" with the dropped lease id(s) + reason (expiry | deescalate)
```

## Configuration knobs (LoopOptions / ReplConfig)

| Knob | Default | Notes |
|---|---|---|
| `escalation_timeout` | conservative (e.g. 60s) | Bounds `consent.confirm`; elapse → deny. |
| `consent` | `DenyAll` (episode w/o ceiling), `AllowWithinCeiling` (episode w/ ceiling), `PromptConsent` (REPL) | The async sink. |
| `default_lease_ttl` | `Turns(1)` | Applied when a delta carries no explicit TTL. |
| `reactive_escalation` | on iff a ceiling is set | Gates the `DenialEscalationHook`. |

## Invariants

- **INV-C1**: `consent.confirm` is consulted **only** for a candidate already proven `⊆ ceiling`.
- **INV-C2**: A timeout, `Denied`, compile error, or reload error leaves `ActivePolicy` and the
  enforced maps unchanged (fail-closed).
- **INV-C3**: A triggering call is retried at most once per escalation; an approved-but-still-denied
  retry is returned to the model, not re-escalated (spec SC-005).
