# SARIF fixtures

## `opengrep-shell-true.json`

Real output, not hand-written. Produced on 2026-07-26 with **Opengrep 1.22.0**:

```bash
printf 'import subprocess\ndef f(x):\n    subprocess.call(x, shell=True)\n' > vuln.py
cat > rules.yml <<'EOF'
rules:
  - id: subprocess-shell-true
    patterns:
      - pattern: subprocess.call(..., shell=True, ...)
    message: "subprocess call with shell=True can execute attacker-controlled input"
    languages: [python]
    severity: ERROR
EOF
opengrep scan --sarif --sarif-output=out.sarif --quiet --config rules.yml .
```

This is the **local-ruleset** variant: 1 result, 1 embedded rule, 1,316 bytes (3,130 after
pretty-printing, which is how it is stored here for reviewability).

### The proportions that forced the two-child pipeline

The same target scanned with `--config auto` — the registry ruleset — measured (research R4):

| Quantity | Value |
|---|---|
| Results | 1 |
| Rules embedded in `tool.driver.rules` | 1074 |
| Total document | 1,912,546 bytes |
| The `results` array alone | 839 bytes |
| Share of the document that is rules | **99.96%** |
| `DEFAULT_OUTPUT_CAP` | 102,400 bytes |

The `auto` document is **19× the output cap**, which is why the scanner writes to a file and
`bee sarif-worker` normalises it in-scope rather than the harness capturing stdout: a capture would
truncate 1.9 MB into unparseable JSON and surface as "the scanner found nothing" — the exact failure
FR-012 exists to prevent. It is not committed here because a 1.9 MB fixture that is 99.96% rule text
is not reviewable, and the trimmed variant exercises every field the normaliser reads.

### Two things this fixture pins that are easy to get wrong

1. **`result.level` is absent.** Opengrep puts the severity on the *rule*
   (`tool.driver.rules[].defaultConfiguration.level`), not the result. A normaliser that skipped the
   catalogue entirely could therefore never populate `advisory_level` at all, so `src/sarif.rs`
   streams that array and retains only `id → level`, bounded by `MAX_RULE_LEVELS` — the
   descriptions, help text, and tags that are the 99.96% are still never materialised (research R5).
   This fixture pins that path: its one rule carries `"level": "error"`, and the finding it produces
   comes out `advisory_level: Some("error")` with `severity: None`, because a scanner's level is
   advisory only and never becomes a `Severity` (FR-005).
2. **`executionSuccessful: true` sits in `runs[].invocations[]`.** It, not the exit status, is what
   says the scan actually ran (research R6).
