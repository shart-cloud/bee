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

**Expected**: the default build's dependency resolution and binary size are unchanged from `main`.
(`Cargo.lock` is gitignored, so this is checked by comparing `cargo tree` across the two branches,
not by diffing a committed lockfile — and structurally by the `Cargo.toml` diff, where every crate
016 adds is `optional = true` and `default` is untouched.) Each feature builds standalone. `cargo build --features astgrep` without any `astgrep-<lang>` builds
and reports every language as unsupported at runtime rather than failing to compile.

## Test

```bash
cargo test --features sec,astgrep-rust,astgrep-python
cargo test --test astgrep_tool    --features astgrep,astgrep-rust
cargo test --test finding_ledger  --features findings
cargo test --lib  --features cvss cvss::                   # the cvss tests are unit tests
cargo test --test scanner_adapter --features scanners      # skips gracefully without opengrep
cargo test --test core_deps_guard                          # Constitution V
```

`cvss` has no integration-test target: the tool opens no files and spawns no child, so there is
nothing for an out-of-process test to observe that a unit test cannot. Its tests live beside it in
`src/tools/cvss.rs`.

**Known unrelated failure**: `repl_command::no_provider_at_all_reports_what_is_missing` fails on a
non-`enforce` build — the unenforced-session refusal fires before the configuration-completeness
check the test asserts on. It fails identically on `main` and is not a 016 regression.

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

**Expected**: `bee astgrep-worker: language `python` is unsupported; compiled in: rust`, exit 1. Not
an empty result. Called as a *tool* rather than as the worker, the same refusal renders through
`ToolOutcome` as `did not run: language `python` is unsupported; compiled in: rust` — the
`did not run:` prefix belongs to the tool seam, not the worker.

```bash
# Built with `astgrep` but no grammar at all — compiles, and says so at runtime
cargo run --features astgrep -- \
    astgrep-worker --lang rust --path /tmp/bee-astgrep '$X.unwrap()'
```

**Expected**: `language `rust` is unsupported; no grammars are compiled into this build`.

## US2 — findings survive, merge, and keep their verdicts

`record_finding` is a model-facing tool, so the round trip is driven by an episode. The `mock`
provider scripts one deterministically, with no key and no network:

```bash
cat > prov.toml <<'EOF'
[provider]
provider = "mock"

[[provider.script]]
tool = "record_finding"
args = { path = "src/db.rs", class = "sql-injection", title = "Query built by string concatenation", evidence = "user input flows into format! then executes as SQL at src/db.rs:42" }

[[provider.script]]
tool = "record_finding"
args = { path = "src/upload.rs", class = "path-traversal", title = "Upload name used unsanitised", evidence = "the multipart filename is joined onto the storage root with no normalisation" }

[[provider.script]]
text = "recorded two findings"
EOF

# Run 1: record two findings
cargo run --features findings -- run --provider prov.toml --task audit \
    --tools record_finding,list_findings --host --quiet --out run1.json
cat .bee/findings/ledger.jsonl            # two `observed` events, one per finding

# Mark one as a false positive (a human act)
bee findings adjudicate <id> --state false-positive --note "sanitised upstream"

# Run 2: rediscover the same two findings
cargo run --features findings -- run --provider prov.toml --task audit \
    --tools record_finding,list_findings --host --quiet --out run2.json
cat .bee/findings/ledger.jsonl            # now: 2 observed + 1 adjudicated + 2 observed
bee findings list
```

**Expected**:
- `view.json` holds **two** findings, not four (FR-019, SC-003).
- Each carries **two** sightings.
- The false-positive verdict is intact (FR-020, SC-004) — the re-discovery appended a sighting and
  had no power to clear it.
- `ledger.jsonl` is readable and diffable line by line (FR-023).

```bash
# A record missing required evidence is rejected, ledger unchanged (FR-021)
# Script a record_finding step with `evidence` omitted, then:
# → `record_finding: invalid arguments: missing field `evidence``, and ledger.jsonl byte-identical
md5sum .bee/findings/ledger.jsonl        # before and after — must match
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

**Expected** (measured 2026-07-26, re-measured 2026-07-27 on opengrep 1.22.0): `results: 1
rules: 1074`, `bytes: ~1912550  results bytes: ~840`. The byte counts drift by a few bytes with the
absolute path of the target; the counts and the proportions are what matter — **99.96%** rules, and
**18.7×** the 102,400-byte `DEFAULT_OUTPUT_CAP`. This is why the scanner writes to a file and
`bee sarif-worker` normalises it in-scope rather than piping stdout.

`scan` is a model-facing tool, not a CLI subcommand — there is deliberately no `bee scan`, because a
scan is something an episode does under a policy, and the grant that authorises it lives in that
policy. So the live run goes through a scenario. Note that `--policy` cannot be combined with
`--host`; a scenario carries its own `policy_path`, which is how an unenforced build still exercises
the grant path.

```bash
cat > rules.yaml <<'EOF'
rules:
  - id: subprocess-shell-true
    patterns: [{pattern: "subprocess.call(..., shell=True, ...)"}]
    message: subprocess called with shell=True
    languages: [python]
    severity: ERROR
EOF

cat > policy.toml <<EOF
[policy]
name = "scan"
mode = "observe"
[policy.filesystem]
":project_root" = "write"
"/tmp" = "write"
[policy.exec]
allow = ["!$(command -v opengrep)"]        # the `!` is the inode pin — an unpinned entry is NOT a grant
[policy.network]
allow = []
[policy.exfiltration]
enabled = false
EOF

cat > scenario.toml <<EOF
[scenario]
id = "scan-og"
policy_path = "$PWD/policy.toml"
system_prompt = "you scan things"
task = "scan /tmp/bee-og"
turn_limit = 4
timeout_secs = 120
tools = ["scan"]

[security.scanners.opengrep]
rules = "$PWD/rules.yaml"                  # a LOCAL ruleset; `auto` is refused at argv construction
EOF

cat > scan-prov.toml <<'EOF'
[provider]
provider = "mock"
[[provider.script]]
tool = "scan"
args = { scanner = "opengrep", target = "/tmp/bee-og" }
[[provider.script]]
text = "scanned"
EOF

cargo run --features sec,astgrep-rust -- run --scenario scenario.toml \
    --provider scan-prov.toml --host --quiet --out scan.json
```

Two things this `--host` example does not have to get right, and an enforcing build does (017):

* **The policy must grant bee itself** — `allow = ["!$(command -v opengrep)", "!$(which bee)"]`. The
  pipeline's second child is `bee sarif-worker`; nothing admits it implicitly.
* **The report channel is granted for you.** `.bee/scan` sits inside `:project_root/.bee`, which is a
  default protection, so a resolved scanner grant widens the policy by exactly that subtree — the
  same mechanism a skill grant uses. `.bee/findings` stays denied: the ledger is written by the
  harness, never by a child.

**Expected**: `opengrep: 1 finding(s), 1 recorded in the ledger`, the finding sourced
`scanner:opengrep` in `.bee/findings/ledger.jsonl`, and `.bee/scan/` empty afterwards — the SARIF
report is removed once normalised.

The fail-closed cases — each must be **distinguishable from a clean scan** (FR-012, SC-002):

| Setup | Expected outcome |
|---|---|
| No grant in policy, or an **unpinned** `exec.allow` entry | `did not run: scanner `opengrep` is not granted to this episode (a binary on PATH is not a grant)` |
| Granted path does not exist | `did not run: binary missing: <path>` |
| Binary replaced after grant | `did not run: pin mismatch` — never executed (SC-010) |
| `--config auto` requested | refused at argv construction, not at runtime (research R6) |
| Report unparseable | `failed: unparseable report` — **not** an empty result |
| Timeout exceeded | `failed: timeout` — no partial findings presented as complete |
| Built without `--features scanners` | `did not run: not compiled in: scanners` |

```bash
# Untrusted output cannot drive the terminal (FR-015, SC-005)
printf 'x = "\033[2J\033[1;1H PWNED"  # bidi: ‮\n' > /tmp/bee-og/nasty.py
# add a rule that matches it, re-scan, then render the finding:
bee findings list
```

**Expected**: the ledger stores the bytes verbatim — it is evidence, and altering it would be
falsifying the record — while every rendering escapes them: `x = "\x1b[2J\x1b[1;1H PWNED"  # bidi:
\u{202e}`. The screen does not clear. This is the 014 `safe_text` property applied to a new input
channel.

**Where the advisory level comes from**: Opengrep reports severity on the *rule*
(`tool.driver.rules[].defaultConfiguration.level`), not on the result. That array is the 99.96% of
the report research R4 says not to read, so `src/sarif.rs` streams it and retains only `id → level`
— two short strings per rule, bounded by `MAX_RULE_LEVELS` (4096) — while the descriptions, help
text, and tags are parsed and dropped. A result carrying its own `level` overrides the rule default.
The level is recorded and rendered as advisory and never becomes a `Severity` (FR-005); an
over-cap catalogue costs advisory levels for the overflow and never a finding or a scan.

## US4 — severity is computed, not guessed

`cvss` is a model-facing tool, not a CLI subcommand — there is no `bee cvss`. Script it:

```bash
cat > cv.toml <<'EOF'
[provider]
provider = "mock"
[[provider.script]]
tool = "cvss"
args = { vector = "CVSS:3.1/AV:N/AC:L/PR:N/UI:N/S:U/C:H/I:H/A:H" }
[[provider.script]]
tool = "cvss"
args = { vector = "CVSS:3.1/AV:X/nonsense" }
[[provider.script]]
tool = "cvss"
args = { vector = "CVSS:4.0/AV:N/AC:L/AT:N/PR:N/UI:N/VC:H/VI:H/VA:H/SC:N/SI:N/SA:N" }
[[provider.script]]
text = "scored"
EOF

cargo run --features cvss,findings -- run --provider cv.toml --task score \
    --tools cvss --host --quiet --out cv.json
```

**Expected**, in order:

- `9.8 (critical) — CVSS:3.1/…` — matching the published v3.1 calculation (SC-008).
- `failed: could not parse `CVSS:3.1/AV:X/nonsense`: invalid CVSS metric group component:
  `nonsense``. Not a zero, not a guess.
- `9.3 (critical) — CVSS:4.0/…`, the v4.0 path.

**Also verify**: a `record_finding` carrying `severity_score` is **rejected** with `failed: a
severity score may not be supplied: state `severity_vector` instead and the score is computed from
it` (FR-005) — silent recomputation would hide the violation.

`RecordArgs` carries `#[serde(deny_unknown_fields)]`, so a caller who nests the score under some
other name — `severity = { score = … }` — is refused by name too, rather than having the object
dropped in silence. There is no spelling of "here is my score" that bee accepts quietly.

---

## US5 — read the repository's history

```bash
# In any git repository (this one will do):
cargo run --features gitlog -- gitlog-worker --path src/tools.rs --mode log --limit 5
cargo run --features gitlog -- gitlog-worker --path src/tools.rs --mode blame --line 1
```

**Expected**: five `<sha> <date> <author> <summary>` lines, newest first, for `log`; one
`<sha> <date> <author> line 1: <summary>` for `blame`. The dates are RFC 3339 in UTC — a repository
records a per-commit offset, and normalising means two commits from two timezones sort the way a
reader assumes they do.

The three outcomes that look alike from outside a quiet child, and how they are kept apart:

| Case | Exit | What the model sees |
|---|---|---|
| Commits found | 0 | the lines |
| No commit touches the path (but there is a repository) | 0 | `no commits in this repository touch that path` |
| The path is not inside a repository | 3 | `did not run: <path> is not inside a repository…` |
| The request cannot be answered (bad line, unreadable object) | 4 | `failed: <diagnostic>` |

```bash
# The refusal, from a directory with no repository above it:
cd /tmp && mkdir -p not-a-repo && cargo run --features gitlog --manifest-path <repo>/Cargo.toml \
    -- gitlog-worker --path /tmp/not-a-repo --mode log; echo "exit=$?"
```

**Expected**: `exit=3` and nothing on stdout. An empty history and an unreadable one are different
answers, and the exit code — not the wording — is what the tool wrapper reads to keep them apart.

## Constitution verification

```bash
# V — no new crate reached bee-core / bee-common
cargo test --test core_deps_guard

# V — the core still builds without an async runtime and without the new families
cargo build -p bee-core

# VI (SC-006) — inspect a scanning episode's exec allowlist
# Expected: bee's own inode, plus exactly the scanners explicitly granted. Nothing else.
```

## US6 — the CodeQL bundle

Built, and testable without provisioning a bundle, because everything US6 turns on is a refusal:

```bash
cargo test --features scanners --test codeql_adapter
# Expected: 10 passed. Each refusal asserts both the outcome and that the stub CLI logged no
# invocation — for a version mismatch, exactly one (the version check itself, and nothing after).
```

Configuration is the Opengrep shape plus a pin. The grant names the bundle's own CLI:

```toml
[exec]
allow = ["!/opt/codeql-bundle/codeql/codeql"]

[security.scanners.codeql]
bundle_version = "codeql-bundle-v2.26.1"   # or "2.26.1"; both spell the same pin
```

Analysable: `actions`, `csharp`, `java`, `javascript`, `python`, `ruby` — plus CodeQL's own aliases,
so `typescript` reaches the `javascript` extractor. Everything else is declined **by name** with what
*is* analysable, because a traced language extracted without tracing yields a thin database, and a
thin database reads exactly like clean code.

> **Not yet walked against a real bundle.** A stub proves bee's half — the argv it authors, the order
> it runs, every path on which it refuses, and the report reaching the ledger. It cannot prove that
> CodeQL, handed these arguments, produces useful findings. That walkthrough is **T073**, and it is
> also where `rust` gets settled: if the bundle's Rust extractor ships no `tools/tracing-config.lua`,
> it joins the list and bee can analyse itself.

## Deferred to follow-on

- **VM case `scanner-escape-denied`** — landed on the matrix (38/38) once 017 made a scanner grant
  installable under enforcement; see `specs/017-enforceable-pins/`.
