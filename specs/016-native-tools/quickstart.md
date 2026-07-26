# Quickstart: Harness-Native Security Tooling

Runnable validation for the first slice (US1–US4). Everything here works **without kernel
enforcement and without the VM** (SC-011) — that is a deliberate property of this feature, so
contributors can iterate on a laptop.

## Prerequisites

- Rust **1.88+** to build the `bee` application package (its declared MSRV, set by `ast-grep-core`
  0.45 — research R3). Already satisfied: `rust-toolchain.toml` is unpinned `stable`, currently
  1.96, so no `rustup` action is needed. The embeddable core crates (`bee-core`, `bee-common`,
  `bee-userspace`) keep their 1.85 floor and still build on it.
- For the scanner-adapter checks only: `opengrep` on the host. Verified against **1.22.0**.
  Everything else runs with no external tooling installed — which is the point (SC-001).

## Build

```bash
# Default build — unchanged dependency graph, proves SC-009
cargo build

# Security tooling, Rust + Python grammars only
cargo build --features sec,astgrep-rust,astgrep-python

# One family at a time, to confirm the gates are independent (FR-006)
cargo build --features cvss
cargo build --features findings
cargo build --features astgrep,astgrep-rust
```

**Expected**: the default build's `Cargo.lock` resolution and binary size are unchanged from `main`.
Each feature builds standalone. `cargo build --features astgrep` without any `astgrep-<lang>` builds
and reports every language as unsupported at runtime rather than failing to compile.

## Test

```bash
cargo test --features sec,astgrep-rust,astgrep-python
cargo test --test astgrep_tool    --features astgrep,astgrep-rust
cargo test --test finding_ledger  --features findings
cargo test --test cvss_tool       --features cvss
cargo test --test scanner_adapter --features scanners      # skips gracefully without opengrep
cargo test --test core_deps_guard                          # Constitution V
```

---

## US1 — structural search beats text search

The acceptance criterion is that decoys inside comments and string literals are **not** returned.

```bash
mkdir -p /tmp/bee-astgrep && cat > /tmp/bee-astgrep/decoy.rs <<'EOF'
fn real() -> u32 {
    let v: Option<u32> = None;
    v.unwrap()                       // ← the only true match
}
// v.unwrap() in a comment
fn quoted() -> &'static str { "v.unwrap() in a string literal" }
EOF

cargo run --features astgrep,astgrep-rust -- \
    astgrep-worker --lang rust --path /tmp/bee-astgrep '$X.unwrap()'
```

**Expected**: exactly one match, at `decoy.rs:3`. The comment and the string literal do not appear —
`search` (regex) would return all three. That difference is SC-007.

```bash
# Unsupported language names what IS available, rather than returning empty (FR-012)
cargo run --features astgrep,astgrep-rust -- \
    astgrep-worker --lang python --path /tmp/bee-astgrep 'foo()'
```

**Expected**: `did not run: language 'python' unsupported; compiled in: rust`. Not an empty result.

## US2 — findings survive, merge, and keep their verdicts

```bash
# Run 1: record two findings
bee run --features findings ...           # or drive record_finding from a REPL session
cat .bee/findings/ledger.jsonl            # two `observed` events, one per finding

# Mark one as a false positive (a human act)
bee findings adjudicate <id> --state false-positive --note "sanitised upstream"

# Run 2: rediscover the same two findings
cat .bee/findings/ledger.jsonl            # now: 2 observed + 1 adjudicated + 2 observed
```

**Expected**:
- `view.json` holds **two** findings, not four (FR-019, SC-003).
- Each carries **two** sightings.
- The false-positive verdict is intact (FR-020, SC-004) — the re-discovery appended a sighting and
  had no power to clear it.
- `ledger.jsonl` is readable and diffable line by line (FR-023).

```bash
# A record missing required evidence is rejected, ledger unchanged (FR-021)
# → expect a Failed outcome naming the missing field, and no new line in ledger.jsonl
```

## US3 — the external tier

```bash
# Reproduce the measurement that forced the two-child pipeline (research R4)
mkdir -p /tmp/bee-og && printf 'import subprocess\ndef f(x):\n    subprocess.call(x, shell=True)\n' \
    > /tmp/bee-og/vuln.py
opengrep scan --config auto --sarif --quiet /tmp/bee-og > /tmp/og.json
python3 -c "
import json,os; d=json.load(open('/tmp/og.json')); r=d['runs'][0]
print('results:', len(r['results']), 'rules:', len(r['tool']['driver']['rules']))
print('bytes:', os.path.getsize('/tmp/og.json'), 'results bytes:', len(json.dumps(r['results'])))"
```

**Expected** (measured 2026-07-26): `results: 1  rules: 1074`, `bytes: 1912546  results bytes: 839`
— 99.96% rules, and **19×** the 102,400-byte `DEFAULT_OUTPUT_CAP`. This is why the scanner writes to
a file and `bee sarif-worker` normalises it in-scope rather than piping stdout.

```bash
# Through bee, with a granted scanner and a LOCAL ruleset
cargo run --features scanners -- scan --scanner opengrep --target /tmp/bee-og
```

**Expected**: one normalised finding in the ledger, sourced `scanner:opengrep`, bounded output.

The fail-closed cases — each must be **distinguishable from a clean scan** (FR-012, SC-002):

| Setup | Expected outcome |
|---|---|
| No grant in policy | `did not run: not granted: opengrep` |
| Granted path does not exist | `did not run: binary missing: <path>` |
| Binary replaced after grant | `did not run: pin mismatch` — never executed (SC-010) |
| `--config auto` requested | refused at argv construction, not at runtime (research R6) |
| Report unparseable | `failed: unparseable report` — **not** an empty result |
| Timeout exceeded | `failed: timeout` — no partial findings presented as complete |
| Built without `--features scanners` | `did not run: not compiled in: scanners` |

```bash
# Untrusted output cannot drive the terminal (FR-015, SC-005)
printf 'x = "\033[2J\033[1;1H PWNED"  # bidi: ‮\n' > /tmp/bee-og/nasty.py
# scan it, then confirm the rendered finding shows the escapes inert — the screen does not clear
```

**Expected**: control and bidi characters render inert. This is the 014 `safe_text` property applied
to a new input channel.

## US4 — severity is computed, not guessed

```bash
cargo run --features cvss -- cvss "CVSS:3.1/AV:N/AC:L/PR:N/UI:N/S:U/C:H/I:H/A:H"
```

**Expected**: `9.8 (critical)` — matching the published v3.1 calculation. The test suite checks a
reference set of vectors with independently known scores (SC-008).

```bash
cargo run --features cvss -- cvss "CVSS:3.1/AV:X/nonsense"
```

**Expected**: `failed: <parse diagnostic>`. Not a zero, not a guess.

**Also verify**: submitting a finding with a pre-populated `severity.score` is **rejected**, not
silently recomputed (FR-005) — silent recomputation would hide the violation.

---

## Constitution verification

```bash
# V — no new crate reached bee-core / bee-common
cargo test --test core_deps_guard

# V — the core still builds without an async runtime and without the new families
cargo build -p bee-core

# VI (SC-006) — inspect a scanning episode's exec allowlist
# Expected: bee's own inode, plus exactly the scanners explicitly granted. Nothing else.
```

## Deferred to follow-on

- **US5** (`git_log` via `gix`) and **US6** (CodeQL bundle) are planned in
  [`contracts/native-tools.md`](./contracts/native-tools.md) and
  [`contracts/scanner-adapter.md`](./contracts/scanner-adapter.md) but are not in the first slice.
- **VM case `scanner-escape-denied`** — proving the kernel refuses a scanner child's out-of-scope
  read — belongs on the `ac-matrix-vm` matrix (currently 35/35). Not required for the first slice to
  land, since everything above is host-testable.
