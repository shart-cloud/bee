#!/usr/bin/env bash
# Runs INSIDE the VM. Exercises bee's enforcement matrix against the real BPF-LSM kernel and emits
# machine-parseable `RESULT|<case>|<PASS|FAIL>|<note>` lines. Exits non-zero if any case failed.
#
# Assumes /home/ubuntu/bee (built with --features enforce) exists and is executable.
set -u
BEE=${BEE:-/home/ubuntu/bee}
WORK=$(mktemp -d)
pass=0
fail=0

emit() { # name status note
  echo "RESULT|$1|$2|$3"
  if [ "$2" = PASS ]; then pass=$((pass + 1)); else fail=$((fail + 1)); fi
}

# Run bee, capturing child stdout (fd1) and bee audit/stderr separately.
run_bee() { # policy -- cmd...
  local policy=$1; shift
  sudo "$BEE" exec --policy "$policy" "$@" 2>"$WORK/err"
}

mkdir -p /home/ubuntu/.ssh
echo TOP-SECRET > /home/ubuntu/.ssh/secret.txt

# ---------------------------------------------------------------- kernel substrate
lsm=$(cat /sys/kernel/security/lsm)
case ",$lsm," in *,bpf,*) emit kernel-bpf-lsm PASS "$lsm" ;; *) emit kernel-bpf-lsm FAIL "$lsm" ;; esac
[ -e /sys/kernel/btf/vmlinux ] && emit kernel-btf PASS present || emit kernel-btf FAIL missing
"$BEE" check >/dev/null 2>&1 && emit bee-check PASS "gates ok" || emit bee-check FAIL "check exit $?"

# ---------------------------------------------------------------- network allow / deny
cat >"$WORK/net.toml" <<'EOF'
[policy]
name = "net"
mode = "enforce"
[policy.network]
allow = ["1.1.1.1:443"]
EOF
# A non-000 HTTP code means the TCP connection succeeded (2xx/3xx redirects both count).
connected() { case "$1" in 000 | "") return 1 ;; *) return 0 ;; esac; }
out=$(run_bee "$WORK/net.toml" -- bash -c 'curl -sS --max-time 8 -o /dev/null -w %{http_code} https://1.1.1.1')
if connected "$out"; then emit net-allow PASS "http=$out"; else emit net-allow FAIL "http=$out"; fi
out=$(run_bee "$WORK/net.toml" -- bash -c 'curl -sS --max-time 8 -o /dev/null -w %{http_code} https://8.8.8.8')
if [ "$out" = 000 ]; then emit net-deny PASS "connection blocked"; else emit net-deny FAIL "http=$out (expected block)"; fi

# ---------------------------------------------------------------- file deny / allow
cat >"$WORK/fs.toml" <<'EOF'
[policy]
name = "fs"
mode = "enforce"
[policy.filesystem]
"/home/ubuntu/.ssh" = "deny"
EOF
out=$(run_bee "$WORK/fs.toml" -- cat /home/ubuntu/.ssh/secret.txt)
if [ "$out" != "TOP-SECRET" ]; then emit file-deny PASS "secret not leaked"; else emit file-deny FAIL "secret leaked"; fi
out=$(run_bee "$WORK/fs.toml" -- cat /etc/hostname)
if [ -n "$out" ]; then emit file-allow PASS "read ok ($out)"; else emit file-allow FAIL "non-denied read blocked"; fi

# postfix glob deny (`*.secret`) — the new kind-dispatched matching
cat >"$WORK/glob.toml" <<'EOF'
[policy]
name = "glob"
mode = "enforce"
[policy.filesystem]
"*.secret" = "deny"
EOF
echo SECRETDATA >/tmp/g.secret
echo plaindata >/tmp/g.txt
out=$(run_bee "$WORK/glob.toml" -- cat /tmp/g.secret)
if [ "$out" != "SECRETDATA" ]; then emit file-glob-deny PASS "*.secret blocked"; else emit file-glob-deny FAIL "leaked"; fi
out=$(run_bee "$WORK/glob.toml" -- cat /tmp/g.txt)
if [ "$out" = "plaindata" ]; then emit file-glob-allow PASS "non-matching file opens"; else emit file-glob-allow FAIL "out='$out'"; fi

# fail-closed on an unenforceable segment rule (`**/name`) — must refuse, not silently under-enforce
cat >"$WORK/seg.toml" <<'EOF'
[policy]
name = "seg"
mode = "enforce"
[policy.filesystem]
"**/target" = "deny"
EOF
sudo "$BEE" exec --policy "$WORK/seg.toml" -- true >/dev/null 2>"$WORK/err"
rc=$?
if [ "$rc" -eq 64 ] && grep -q 'cannot enforce filesystem segment rule' "$WORK/err"; then
  emit file-segment-refused PASS "fail-closed on unenforceable rule"
else
  emit file-segment-refused FAIL "rc=$rc (expected refusal)"
fi

# ------------------------------------------------------- US2 AS-2: read/write mode enforcement
# A policy granting `write` to a scratch dir and `read` to a source dir: writing the source is
# blocked (read-only), reading it works, writing scratch works, and — because the policy declares a
# writable surface — writing an unlisted path is denied by default (FR-001 read/write modes).
RW=/home/ubuntu/rwtest
rm -rf "$RW"; mkdir -p "$RW/src" "$RW/scratch"
echo "sourcecode" >"$RW/src/lib.rs"
cat >"$WORK/rw.toml" <<'EOF'
[policy]
name = "rw"
mode = "enforce"
[policy.filesystem]
"/home/ubuntu/rwtest/scratch" = "write"
"/home/ubuntu/rwtest/src" = "read"
EOF

out=$(run_bee "$WORK/rw.toml" -- cat "$RW/src/lib.rs")
if [ "$out" = "sourcecode" ]; then emit rw-read-source PASS "read-only source readable"; else emit rw-read-source FAIL "out='$out'"; fi

run_bee "$WORK/rw.toml" -- bash -c 'echo TAMPERED > /home/ubuntu/rwtest/src/lib.rs' >/dev/null 2>&1
if [ "$(cat "$RW/src/lib.rs" 2>/dev/null)" = "sourcecode" ]; then emit rw-write-source-denied PASS "write to read-only source blocked"; else emit rw-write-source-denied FAIL "source was modified"; fi

run_bee "$WORK/rw.toml" -- bash -c 'echo scratchdata > /home/ubuntu/rwtest/scratch/out' >/dev/null 2>&1
if [ "$(cat "$RW/scratch/out" 2>/dev/null)" = "scratchdata" ]; then emit rw-write-scratch PASS "write to granted scratch works"; else emit rw-write-scratch FAIL "scratch write blocked"; fi

run_bee "$WORK/rw.toml" -- bash -c 'echo x > /home/ubuntu/rwtest/unlisted' >/dev/null 2>&1
if [ "$(cat "$RW/unlisted" 2>/dev/null)" != "x" ]; then emit rw-write-default-deny PASS "unlisted write denied by default"; else emit rw-write-default-deny FAIL "unlisted path was written"; fi
rm -rf "$RW"

# ---------------------------------------------------------------- exec allow / deny
cat >"$WORK/exec.toml" <<'EOF'
[policy]
name = "exec"
mode = "enforce"
[policy.exec]
allow = ["bash", "cat", "ls"]
EOF
cat >"$WORK/exectest.sh" <<'EOF'
cat /etc/hostname >/dev/null && echo CAT_OK
nc -h >/dev/null 2>&1; echo NC_RC=$?
EOF
out=$(run_bee "$WORK/exec.toml" -- bash "$WORK/exectest.sh")
echo "$out" | grep -q CAT_OK && emit exec-allow PASS "allowlisted exec ran" || emit exec-allow FAIL "allowed exec blocked"
echo "$out" | grep -q 'NC_RC=126' && emit exec-deny PASS "unlisted exec denied" || emit exec-deny FAIL "nc not denied ($(echo "$out" | tr '\n' ' '))"

# ---------------------------------------------------------------- observe (dry-run) mode
cat >"$WORK/obs.toml" <<'EOF'
[policy]
name = "obs"
mode = "observe"
[policy.filesystem]
"/home/ubuntu/.ssh" = "deny"
EOF
out=$(run_bee "$WORK/obs.toml" -- cat /home/ubuntu/.ssh/secret.txt)
obs_audit=$(grep -c '"decision":"observed"' "$WORK/err")
if [ "$out" = "TOP-SECRET" ] && [ "$obs_audit" -ge 1 ]; then
  emit observe-mode PASS "allowed + $obs_audit observed events"
else
  emit observe-mode FAIL "out='$out' observed=$obs_audit"
fi

# ---------------------------------------------------------------- US2: subagent attenuation
cat >"$WORK/parent.toml" <<'EOF'
[policy]
name = "parent"
mode = "enforce"
[policy.network]
allow = ["1.1.1.1:443", "8.8.8.8:443"]
EOF
cat >"$WORK/child-ok.toml" <<'EOF'
[policy]
name = "child-ok"
mode = "enforce"
[policy.network]
allow = ["1.1.1.1:443"]
EOF
cat >"$WORK/child-bad.toml" <<'EOF'
[policy]
name = "child-bad"
mode = "enforce"
[policy.network]
allow = ["9.9.9.9:443"]
EOF

# An over-broad subagent (requests a dest the parent lacks) must be refused BEFORE running.
sudo "$BEE" exec --policy "$WORK/child-bad.toml" --parent "$WORK/parent.toml" -- true >/dev/null 2>"$WORK/err"
rc=$?
if [ "$rc" -ne 0 ] && grep -q 'attenuation violation' "$WORK/err"; then
  emit atten-reject PASS "over-broad subagent refused (rc=$rc)"
else
  emit atten-reject FAIL "rc=$rc (expected refusal)"
fi

# A subset subagent runs, enforcing its NARROWER policy: the parent allows 8.8.8.8 but the child
# dropped it, so under the child scope 8.8.8.8 is blocked while 1.1.1.1 (kept) still connects.
out=$(sudo "$BEE" exec --policy "$WORK/child-ok.toml" --parent "$WORK/parent.toml" -- bash -c 'curl -sS --max-time 8 -o /dev/null -w %{http_code} https://1.1.1.1' 2>/dev/null)
if connected "$out"; then emit atten-subset-allow PASS "child-kept dest connects (http=$out)"; else emit atten-subset-allow FAIL "http=$out"; fi
out=$(sudo "$BEE" exec --policy "$WORK/child-ok.toml" --parent "$WORK/parent.toml" -- bash -c 'curl -sS --max-time 8 -o /dev/null -w %{http_code} https://8.8.8.8' 2>/dev/null)
if [ "$out" = 000 ]; then emit atten-subset-deny PASS "parent-allowed but child-dropped dest blocked"; else emit atten-subset-deny FAIL "http=$out"; fi

# ---------------------------------------------------------------- per-scope isolation
# A process OUTSIDE any bee scope must be unaffected.
out=$(curl -sS --max-time 8 -o /dev/null -w %{http_code} https://8.8.8.8 2>/dev/null)
if connected "$out"; then emit scope-isolation PASS "host egress unaffected"; else emit scope-isolation FAIL "http=$out"; fi

# ---------------------------------------------------------------- US1 (002): LLM agent episode
# The load-bearing 002 check: a scripted (MockModel) episode whose tool call reads a policy-denied
# path returns a kernel EACCES, the denial appears in the transcript's audit trail, and the episode
# still ends `completed` (the agent ran; the *operation* was denied). Runs through `bee run`, the
# same single executable every other case uses.
if [ -x "$BEE" ]; then
  # episode-file-deny — read a denied path; expect EACCES in the tool result + a file_open denial.
  cat >"$WORK/ep-deny.policy.toml" <<'EOF'
[policy]
name = "ep-deny"
mode = "enforce"
[policy.filesystem]
"/home/ubuntu/.ssh" = "deny"
EOF
  cat >"$WORK/ep-deny.scn.toml" <<EOF
[scenario]
id            = "episode-file-deny"
policy_path   = "$WORK/ep-deny.policy.toml"
system_prompt = "You are a sandboxed agent."
task          = "Read /home/ubuntu/.ssh/secret.txt and report it."
turn_limit    = 4
timeout_secs  = 30
tools         = ["read_file"]
EOF
  cat >"$WORK/ep-deny.prov.toml" <<'EOF'
[provider]
provider = "mock"
[[provider.script]]
tool = "read_file"
args = { path = "/home/ubuntu/.ssh/secret.txt" }
[[provider.script]]
text = "I could not read the file."
EOF
  sudo "$BEE" run --scenario "$WORK/ep-deny.scn.toml" --provider "$WORK/ep-deny.prov.toml" \
    --out "$WORK/ep-deny.json" >"$WORK/ep-deny.log" 2>&1
  t="$WORK/ep-deny.json"
  if [ -f "$t" ] \
     && ! grep -q 'TOP-SECRET' "$t" \
     && grep -q '"is_error": true' "$t" \
     && grep -Eq 'Permission denied|EACCES' "$t" \
     && grep -q '"op": "file_open"' "$t" \
     && grep -q '"decision": "denied"' "$t" \
     && grep -q '"status": "completed"' "$t"; then
    emit episode-file-deny PASS "kernel EACCES surfaced in transcript; episode completed"
  else
    emit episode-file-deny FAIL "transcript did not show the denial ($(tail -1 "$WORK/ep-deny.log" 2>/dev/null))"
  fi

  # episode-allow — a permissive write in the workdir succeeds with no denials.
  mkdir -p /home/ubuntu/epwork
  cat >"$WORK/ep-allow.policy.toml" <<'EOF'
[policy]
name = "ep-allow"
mode = "enforce"
[policy.filesystem]
"/home/ubuntu/epwork" = "write"
EOF
  cat >"$WORK/ep-allow.scn.toml" <<EOF
[scenario]
id            = "episode-allow"
policy_path   = "$WORK/ep-allow.policy.toml"
system_prompt = "You are a sandboxed agent."
task          = "Create a file in the working directory."
turn_limit    = 4
timeout_secs  = 30
tools         = ["write_file"]
EOF
  cat >"$WORK/ep-allow.prov.toml" <<'EOF'
[provider]
provider = "mock"
[[provider.script]]
tool = "write_file"
args = { path = "/home/ubuntu/epwork/out.txt", content = "hello from the agent\n" }
[[provider.script]]
text = "Done — file written."
EOF
  sudo "$BEE" run --scenario "$WORK/ep-allow.scn.toml" --provider "$WORK/ep-allow.prov.toml" \
    --out "$WORK/ep-allow.json" >"$WORK/ep-allow.log" 2>&1
  t="$WORK/ep-allow.json"
  if [ -f "$t" ] \
     && grep -q 'wrote .* bytes to /home/ubuntu/epwork/out.txt' "$t" \
     && ! grep -q '"decision": "denied"' "$t" \
     && grep -q '"status": "completed"' "$t" \
     && [ "$(cat /home/ubuntu/epwork/out.txt 2>/dev/null)" = "hello from the agent" ]; then
    emit episode-allow PASS "in-scope write succeeded, no denials"
  else
    emit episode-allow FAIL "write/allow path failed ($(tail -1 "$WORK/ep-allow.log" 2>/dev/null))"
  fi
  rm -rf /home/ubuntu/epwork

  # -------------------------------------------------------------- 006-skills: Layer 2 grant fold
  # A/B proof that a skill's `requires` filesystem grant is compiled into and enforced by the real
  # kernel scope, bounded by attenuation. The SAME skill requests write to /home/ubuntu/skgrant; only
  # the scenario's ceiling differs. The base policy grants write elsewhere (arming the write-default-
  # deny latch) but NOT skgrant, so skgrant is writable iff the grant actually widened the scope.
  SKROOT="$WORK/skmatrix"
  mkdir -p "$SKROOT/builder" /home/ubuntu/skwork /home/ubuntu/skgrant
  cat >"$SKROOT/builder/SKILL.md" <<'EOF'
---
name: builder
description: needs write access to the build output directory
requires:
  filesystem:
    "/home/ubuntu/skgrant": write
---
Write build output under /home/ubuntu/skgrant.
EOF
  cat >"$WORK/sk-base.policy.toml" <<'EOF'
[policy]
name = "sk-base"
mode = "enforce"
[policy.filesystem]
"/home/ubuntu/skwork" = "write"
EOF
  cat >"$WORK/sk-ceiling.policy.toml" <<'EOF'
[policy]
name = "sk-ceiling"
mode = "enforce"
[policy.filesystem]
"/home/ubuntu/skwork" = "write"
"/home/ubuntu/skgrant" = "write"
EOF
  cat >"$WORK/sk.prov.toml" <<'EOF'
[provider]
provider = "mock"
[[provider.script]]
tool = "write_file"
args = { path = "/home/ubuntu/skgrant/out.txt", content = "built by the agent\n" }
[[provider.script]]
text = "Done."
EOF

  # skill-grant-allow — ceiling permits skgrant ⇒ within-ceiling grant folds into the scope ⇒ write OK.
  rm -f /home/ubuntu/skgrant/out.txt
  cat >"$WORK/sk-allow.scn.toml" <<EOF
[scenario]
id            = "skill-grant-allow"
policy_path   = "$WORK/sk-base.policy.toml"
ceiling_policy_path = "$WORK/sk-ceiling.policy.toml"
system_prompt = "You are a sandboxed agent."
task          = "Write the build output."
turn_limit    = 4
timeout_secs  = 30
tools         = ["write_file"]
skills        = ["$SKROOT"]
EOF
  sudo "$BEE" run --scenario "$WORK/sk-allow.scn.toml" --provider "$WORK/sk.prov.toml" \
    --out "$WORK/sk-allow.json" >"$WORK/sk-allow.log" 2>&1
  t="$WORK/sk-allow.json"
  if [ -f "$t" ] \
     && grep -q 'wrote .* bytes to /home/ubuntu/skgrant/out.txt' "$t" \
     && ! grep -q '"decision": "denied"' "$t" \
     && grep -q '"status": "completed"' "$t" \
     && [ "$(cat /home/ubuntu/skgrant/out.txt 2>/dev/null)" = "built by the agent" ]; then
    emit skill-grant-allow PASS "within-ceiling grant compiled into scope; write to skgrant allowed"
  else
    emit skill-grant-allow FAIL "granted path was not writable ($(tail -1 "$WORK/sk-allow.log" 2>/dev/null))"
  fi

  # skill-grant-deny — no ceiling (base is the ceiling) ⇒ skgrant grant exceeds it ⇒ refused ⇒ the
  # scope is NOT widened ⇒ the same write is kernel-denied (EACCES). Attenuation held at the kernel.
  rm -f /home/ubuntu/skgrant/out.txt
  cat >"$WORK/sk-deny.scn.toml" <<EOF
[scenario]
id            = "skill-grant-deny"
policy_path   = "$WORK/sk-base.policy.toml"
system_prompt = "You are a sandboxed agent."
task          = "Write the build output."
turn_limit    = 4
timeout_secs  = 30
tools         = ["write_file"]
skills        = ["$SKROOT"]
EOF
  sudo "$BEE" run --scenario "$WORK/sk-deny.scn.toml" --provider "$WORK/sk.prov.toml" \
    --out "$WORK/sk-deny.json" >"$WORK/sk-deny.log" 2>&1
  t="$WORK/sk-deny.json"
  # NB: bee enforces writes via the `file_open` LSM hook, which fires AFTER `O_CREAT` has made the
  # empty inode — so a 0-byte file may exist; the load-bearing check is that the DATA never landed.
  if [ -f "$t" ] \
     && grep -q '"is_error": true' "$t" \
     && grep -Eq 'Permission denied|EACCES' "$t" \
     && grep -q '"decision": "denied"' "$t" \
     && grep -q '"status": "completed"' "$t" \
     && [ "$(cat /home/ubuntu/skgrant/out.txt 2>/dev/null)" != "built by the agent" ]; then
    emit skill-grant-deny PASS "beyond-ceiling grant refused; write to skgrant kernel-denied (EACCES)"
  else
    emit skill-grant-deny FAIL "refused grant did not stay denied ($(tail -1 "$WORK/sk-deny.log" 2>/dev/null))"
  fi
  rm -rf /home/ubuntu/skwork /home/ubuntu/skgrant

  # -------------------------------------------------------------- 007-dynamic-grants: live reload
  # Reactive escalation A/B: the agent writes a path the base scope denies. With a ceiling that
  # permits it, the DenialEscalationHook grants write, `reload_scope` widens the LIVE eBPF maps, and
  # the SAME call is auto-retried and succeeds. With a ceiling that does NOT cover it, the grant is
  # refused and the write stays kernel-denied. Same op, only the ceiling differs — proving the live
  # reload changes what the kernel enforces mid-episode.
  mkdir -p /home/ubuntu/dynwork /home/ubuntu/dyngrant
  cat >"$WORK/dyn-base.policy.toml" <<'EOF'
[policy]
name = "dyn-base"
mode = "enforce"
[policy.filesystem]
"/home/ubuntu/dynwork" = "write"
EOF
  cat >"$WORK/dyn-ceiling.policy.toml" <<'EOF'
[policy]
name = "dyn-ceiling"
mode = "enforce"
[policy.filesystem]
"/home/ubuntu/dynwork" = "write"
"/home/ubuntu/dyngrant" = "write"
EOF
  cat >"$WORK/dyn.prov.toml" <<'EOF'
[provider]
provider = "mock"
[[provider.script]]
tool = "write_file"
args = { path = "/home/ubuntu/dyngrant/out.txt", content = "written after reload\n" }
[[provider.script]]
text = "Done."
EOF

  # reload-widen-allow — ceiling permits dyngrant ⇒ denial → escalate → reload → retry writes data.
  rm -f /home/ubuntu/dyngrant/out.txt
  cat >"$WORK/dyn-allow.scn.toml" <<EOF
[scenario]
id            = "reload-widen-allow"
policy_path   = "$WORK/dyn-base.policy.toml"
ceiling_policy_path = "$WORK/dyn-ceiling.policy.toml"
system_prompt = "You are a sandboxed agent."
task          = "Write the output."
turn_limit    = 4
timeout_secs  = 30
tools         = ["write_file"]
EOF
  sudo "$BEE" run --scenario "$WORK/dyn-allow.scn.toml" --provider "$WORK/dyn.prov.toml" \
    --out "$WORK/dyn-allow.json" >"$WORK/dyn-allow.log" 2>&1
  t="$WORK/dyn-allow.json"
  # A denial must appear (the first attempt) AND the data must ultimately land (the retry).
  if [ -f "$t" ] \
     && grep -q '"op": "file_open"' "$t" \
     && grep -q '"decision": "denied"' "$t" \
     && grep -q '"status": "completed"' "$t" \
     && [ "$(cat /home/ubuntu/dyngrant/out.txt 2>/dev/null)" = "written after reload" ]; then
    emit reload-widen-allow PASS "denial → escalate → live reload → retry wrote data"
  else
    emit reload-widen-allow FAIL "reactive reload did not permit the retried write ($(tail -1 "$WORK/dyn-allow.log" 2>/dev/null))"
  fi

  # reload-beyond-ceiling — ceiling omits dyngrant ⇒ escalation refused ⇒ write stays denied.
  rm -f /home/ubuntu/dyngrant/out.txt
  cat >"$WORK/dyn-deny.scn.toml" <<EOF
[scenario]
id            = "reload-beyond-ceiling"
policy_path   = "$WORK/dyn-base.policy.toml"
ceiling_policy_path = "$WORK/dyn-base.policy.toml"
system_prompt = "You are a sandboxed agent."
task          = "Write the output."
turn_limit    = 4
timeout_secs  = 30
tools         = ["write_file"]
EOF
  sudo "$BEE" run --scenario "$WORK/dyn-deny.scn.toml" --provider "$WORK/dyn.prov.toml" \
    --out "$WORK/dyn-deny.json" >"$WORK/dyn-deny.log" 2>&1
  t="$WORK/dyn-deny.json"
  if [ -f "$t" ] \
     && grep -q '"decision": "denied"' "$t" \
     && grep -q '"status": "completed"' "$t" \
     && [ "$(cat /home/ubuntu/dyngrant/out.txt 2>/dev/null)" != "written after reload" ]; then
    emit reload-beyond-ceiling PASS "beyond-ceiling escalation refused; write stayed kernel-denied"
  else
    emit reload-beyond-ceiling FAIL "refused escalation did not stay denied ($(tail -1 "$WORK/dyn-deny.log" 2>/dev/null))"
  fi
  rm -rf /home/ubuntu/dynwork /home/ubuntu/dyngrant
else
  emit episode-file-deny FAIL "bee not shipped to $BEE"
fi

# ---------------------------------------------------------------- US4: concurrent audit isolation
# Four episodes run concurrently (--batch --concurrent), each in its OWN scope, each reading a
# DISTINCT policy-denied path. The async audit demux routes events by cgroup_id, so each transcript
# must contain ONLY its own denial and none of the others' (SC-004 / US4 AS-1). Requires a `bee`
# built with `--features concurrent`; a plain enforce build SKIPs this case.
if [ -x "$BEE" ]; then
  ISO=/home/ubuntu/iso
  rm -rf "$ISO"; mkdir -p "$ISO"
  for i in 1 2 3 4; do echo "SECRET$i" >"$ISO/secret$i.txt"; done
  cat >"$WORK/iso.policy.toml" <<'EOF'
[policy]
name = "iso"
mode = "enforce"
[policy.filesystem]
"/home/ubuntu/iso" = "deny"
EOF
  mkdir -p "$WORK/iso-scn" "$WORK/iso-prov"
  cat >"$WORK/iso-scn/iso.scn.toml" <<EOF
[scenario]
id            = "iso"
policy_path   = "$WORK/iso.policy.toml"
system_prompt = "You are a sandboxed agent."
task          = "Read the assigned secret."
turn_limit    = 4
timeout_secs  = 30
tools         = ["read_file"]
EOF
  for i in 1 2 3 4; do
    cat >"$WORK/iso-prov/m$i.toml" <<EOF
[provider]
provider = "mock"
model    = "m$i"
[[provider.script]]
tool = "read_file"
args = { path = "/home/ubuntu/iso/secret$i.txt" }
[[provider.script]]
text = "done"
EOF
  done
  sudo "$BEE" run --batch --concurrent \
    --scenarios "$WORK/iso-scn/iso.scn.toml" --providers "$WORK/iso-prov" \
    --out "$WORK/iso-out" --quiet >"$WORK/iso.log" 2>&1
  if grep -q 'requires building with --features concurrent' "$WORK/iso.log"; then
    echo "RESULT|concurrent-audit-isolation|SKIP|bee built without --features concurrent"
  else
    ok=1; note="ok"
    for i in 1 2 3 4; do
      t="$WORK/iso-out/iso_mock_m$i.json"
      if [ ! -f "$t" ]; then ok=0; note="missing transcript for ep $i"; break; fi
      if ! grep -q "/home/ubuntu/iso/secret$i.txt" "$t"; then ok=0; note="ep $i missing its own denial"; break; fi
      for j in 1 2 3 4; do
        [ "$j" = "$i" ] && continue
        if grep -q "secret$j.txt" "$t"; then ok=0; note="ep $i leaked secret$j (cross-contamination)"; break 2; fi
      done
    done
    if [ "$ok" = 1 ]; then
      emit concurrent-audit-isolation PASS "4 concurrent scopes, no cgroup cross-contamination"
    else
      emit concurrent-audit-isolation FAIL "$note"
    fi
  fi
  rm -rf "$ISO"
else
  echo "RESULT|concurrent-audit-isolation|SKIP|no bee at $BEE"
fi

rm -rf "$WORK"
echo "SUMMARY|pass=$pass|fail=$fail"
[ "$fail" -eq 0 ]
