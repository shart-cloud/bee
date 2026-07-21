# Contract: the `render` tool (Slice 1)

The agent-facing I/O contract for the `render` tool. Rhai script in → `RenderSpec` + text summary
out. Implements FR-019…FR-024. Slice 1 supports the static widgets only (no sprites/animation).

---

## Registration & availability

- **Name**: `render`
- **Opt-in** (NFR-003): **not** in `DEFAULT_TOOLS`. A scenario enables it by listing `"render"` in
  its `tools` array; the REPL config may also enable it. `is_known_tool("render")` returns `true` so
  scenario validation accepts the name; `registry_for` builds a `RenderTool` when `"render"` is
  listed.
- **Sandbox**: the `sandbox` parameter to `Tool::call` is **ignored** (FR-022). The tool does no I/O.

## Input schema (advertised to the model)

```json
{
  "type": "object",
  "properties": {
    "script": {
      "type": "string",
      "description": "A Rhai script that builds one visualization by calling the drawing API and ending with render(widget). See the function list and primer below."
    }
  },
  "required": ["script"]
}
```

The tool's `schema()` description embeds a **Rhai primer** (JavaScript-adjacent: `let x = …;`,
`for i in 0..n {}`, `[..]` arrays, `#{}` maps, method calls) and the **full Slice-1 function list**
from `rhai-api.md`, so the model has the API surface in its tool definition (spec Assumption).

## Output (`ToolResult`)

On success:

| Field | Value |
|-------|-------|
| `content` | a **text summary** of what was rendered (FR-023) — never the ANSI art |
| `is_error` | `false` |
| `render_spec` | `Some(RenderSpec)` — the validated widget (FR-023) |

**Summary format** (`content`): one line naming the widget kind, title, and salient counts, e.g.
- `Rendered a bar chart 'File Sizes' with 5 bars.`
- `Rendered a 3-column table 'Results' with 12 rows.`
- `Rendered a vsplit layout (gauge + table).`

When earlier `render()` calls were discarded (multiple renders), append: ` (note: 2 earlier renders
discarded; showing the last).` When data was elided to fit width/height, append: ` (note: 130 of 200
bars shown).`

## Error cases (all return `ToolResult { is_error: true }`, never panic — FR-016/FR-021)

| Condition | Trigger | `content` (sent to model, so it can retry) |
|-----------|---------|--------------------------------------------|
| Operation limit (SC-009) | `loop {}` / runaway compute hits `max_operations` | `render: script exceeded the operation limit (10000 ops)` |
| Unregistered function (SC-010) | `std::fs::read(...)`, `import "std"`, any un-registered call | `render: <Rhai "function not found"/parse error verbatim>` |
| Syntax error | malformed Rhai | `render: <Rhai parse error verbatim>` |
| No visualization | script commits no `render()` | `render: script produced no visualization` |
| Constraint exceeded | nesting > 3, > 500 elements, width > 120, height > 40 | `render: <which cap> exceeded (<limit>)` |
| Other resource limit | max string/array/map/call-depth | `render: script exceeded a resource limit (<which>)` |

The model can retry with a corrected script (same feedback loop as a malformed `bash` command).

## Invariants

- **INV-1** `content` never contains ANSI escape sequences (it is a summary, not the render).
- **INV-2** the tool is pure and in-process: no filesystem, process, socket, or env access is
  reachable from a script (FR-021). The `sandbox` argument is untouched (FR-022).
- **INV-3** evaluation completes within 50 ms for ≤ 10,000 ops; it never blocks the tokio runtime
  (synchronous, fast — NFR-001).
- **INV-4** a successful result always carries `render_spec: Some(_)`; an error result always carries
  `render_spec: None`.

## Test obligations (Slice 1)

- SC-008 — `MockModel` exchange: model calls `render` with a 5-bar script; the test `Collector`
  receives `render_widget` with a `RenderSpec::BarChart` of 5 bars (correct labels/values); the
  returned `ToolResult.content` is the text summary, not ANSI.
- SC-009 — `loop {}` → `is_error: true`, message names the operation limit, within 50 ms.
- SC-010 — unregistered function / `import "std"` → `is_error: true`, Rhai "function not found".
- Edge — no `render()` → "script produced no visualization"; multiple `render()` → last wins + note.
