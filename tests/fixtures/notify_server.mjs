#!/usr/bin/env node
// Minimal MCP stdio server (test fixture, 004-mcp-client FR-043): line-delimited JSON-RPC. It serves
// one tool `alpha`; after the first tools/list it emits a `notifications/tools/list_changed` and then
// serves `alpha` + `beta`, so a client that honors the notification re-fetches and sees `beta`.
let listCount = 0;
const send = (o) => process.stdout.write(JSON.stringify(o) + "\n");
const tool = (name, d) => ({ name, description: d, inputSchema: { type: "object", properties: {} } });
const tools = () => (listCount >= 1 ? [tool("alpha", "a"), tool("beta", "b")] : [tool("alpha", "a")]);

let buf = "";
process.stdin.on("data", (d) => {
  buf += d;
  let i;
  while ((i = buf.indexOf("\n")) >= 0) {
    const line = buf.slice(0, i).trim();
    buf = buf.slice(i + 1);
    if (line) { try { handle(JSON.parse(line)); } catch {} }
  }
});

function handle(m) {
  if (m.method === "initialize") {
    send({ jsonrpc: "2.0", id: m.id, result: {
      protocolVersion: (m.params && m.params.protocolVersion) || "2025-06-18",
      capabilities: { tools: { listChanged: true } },
      serverInfo: { name: "notify-fixture", version: "0.0.1" },
    }});
  } else if (m.method === "tools/list") {
    send({ jsonrpc: "2.0", id: m.id, result: { tools: tools() } });
    if (listCount === 0) {
      listCount = 1;
      setTimeout(() => send({ jsonrpc: "2.0", method: "notifications/tools/list_changed" }), 20);
    }
  } else if (m.id !== undefined && m.id !== null) {
    send({ jsonrpc: "2.0", id: m.id, result: {} });
  }
}
