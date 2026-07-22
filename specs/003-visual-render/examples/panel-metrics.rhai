// A model-owned live panel (008-grid-tui, US2; quickstart Scenario B): the model addresses a widget
// to a NAMED panel with render_to(panel_id, widget) instead of render(widget). The panel persists
// beside the chat flow, and re-rendering the same id ("metrics") replaces it in place across turns —
// no scrollback churn, no duplicate panels. Untargeted render() still commits inline into chat.
//
// panel_id must be 1-32 chars of [a-z0-9_-]; a bad id is a script error (fail-closed).

let g = grid(2, 2);

let p50 = gauge("p50", 0.24);
p50.label("120ms");
p50.color("pollen");
g.cell(0, 0, p50);

let p99 = gauge("p99", 0.71);
p99.label("880ms");
p99.color("honey");
g.cell(0, 1, p99);

let errs = table("errors");
errs.header(["route", "5xx"]);
errs.row_colored(["/checkout", "3"], "sting");
errs.row_colored(["/search", "0"], "pollen");
g.span(1, 0, 1, 2, errs); // full-width bottom row

render_to("metrics", g); // → the "metrics" panel, updated in place on every re-render
