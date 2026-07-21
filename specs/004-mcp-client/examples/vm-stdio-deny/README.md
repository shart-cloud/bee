# VM-verified fixture: stdio MCP kernel-denial (T013 / SC-020, SC-023)

The exact scenario/policy/provider that proves a **stdio MCP server child, running in the episode's
scope cgroup, is denied by the kernel** when it reads a policy-denied path — and the denial is
correlated into the transcript.

The `@modelcontextprotocol/server-filesystem` server is rooted at `/home/ubuntu/mcpws` (so it *allows*
the read at the application level); the policy denies only `…/mcpws/secrets`, so the kernel is the one
that returns `EACCES`. Reads elsewhere default-allow so `node` can run.

## Run (on the BPF-LSM VM, as root)

```
# bee-episode built with --features enforce,mcp; server installed: npm i -g @modelcontextprotocol/server-filesystem
sudo bee-episode --scenario scenario.toml --provider provider.toml --out transcript.json
```

## Verified result (ac-matrix-vm, kernel 6.8, 2026-07-20)

```
call   mcp__fs__read_text_file {"path":"/home/ubuntu/mcpws/secrets/key.pem"}
result DENIED: EACCES  (is_error: true)
audit  file_open denied /home/ubuntu/mcpws/secrets/key.pem  errno -13
status completed
```
