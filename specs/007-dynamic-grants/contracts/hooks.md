# Contract: Loop Hooks Lifecycle (bee-harness)

bee's own lifecycle, Rig-shaped (`AgentHook`/`StepEvent`/`Flow`), living in `run_loop` where the
`Sandbox`/audit ring are reachable. Rig's `AgentRunner` is **not** adopted — Rig stays behind the
`Model` seam.

## Trait

```rust
#[async_trait::async_trait]
pub trait LoopHook: Send + Sync {
    /// Observe one lifecycle event; return a Flow action. Awaited inline — keep it light.
    async fn on_event(&self, ev: &StepEvent<'_>) -> Flow;

    /// Filter high-frequency events without allocating (Rig's `observes`). Default: all.
    fn observes(&self, _kind: StepEventKind) -> bool { true }
}
```

Registered as `hooks: Vec<Box<dyn LoopHook>>` on `LoopOptions` (episode) and `ReplConfig` (REPL).

## Events

```rust
pub enum StepEvent<'a> {
    TurnStart { turn: u32 },
    BeforeModelCall { convo: &'a Conversation, schemas: &'a [ToolSchema] },
    BeforeToolCall(&'a ToolCall),
    KernelDenial { op: &'a str, target: &'a str, call: &'a ToolCall },
    AfterToolResult { call: &'a ToolCall, result: &'a ToolResult, audit: &'a [AuditEvent] },
}
pub enum StepEventKind { TurnStart, BeforeModelCall, BeforeToolCall, KernelDenial, AfterToolResult }
```

## Flow

```rust
pub enum Flow {
    Continue,
    Deny(String),            // skip the tool; return reason to the model
    Escalate(GrantDelta),    // resolve → reload → auto-retry the triggering call once
    Deescalate(GrantId),     // drop the named lease; narrowing reload
    Terminate(String),       // end the episode
}
```

## Dispatch rules

- Hooks run in registration order; **the first hook returning non-`Continue` wins** for that event;
  later hooks are not called for it (Rig's rule).
- `observes(kind)` gates dispatch — a hook that returns `false` for a kind is skipped for it.
- `Escalate`/`Deescalate` are honored only where a reload is meaningful: `BeforeToolCall`,
  `KernelDenial` (escalate), and `TurnStart`/`AfterToolResult` (deescalate/expire). An `Escalate`
  returned from an event with no pending call is treated as a proactive grant with no retry.
- A hook error/panic is contained (logged as an audit note); it does not abort the episode
  (Constitution I — degrade, don't crash).

## Loop integration (episode + REPL share this)

Per turn:
1. `TurnStart` → expire due leases (narrow reload if any), run `refresh_tools`-as-hook.
2. `BeforeModelCall` → observe.
3. Per tool call: `BeforeToolCall` → if `Escalate`, run the escalation cycle (see `consent.md`) then
   auto-retry once; else execute.
4. After execute + audit drain: if any `decision=="denied"`, emit `KernelDenial` → if `Escalate`,
   run the cycle and auto-retry once; then `AfterToolResult`.

The two current loops (`episode.rs::run_loop`, `repl.rs::run_exchange`) call one shared driver so the
tool-execute/audit-drain/hook-dispatch path is not duplicated (spec SC-006). `refresh_tools` (004-mcp)
is migrated to a built-in `LoopHook`.
