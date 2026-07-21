// A pass/fail dot grid (003-visual-render, US6) — the agent can render test/episode outcomes the same
// way bee's own batch chrome does (the honeycomb status grid).
let d = dots("Scenario Outcomes");
d.pass("read-denied-ssh");
d.pass("write-allowed");
d.fail("ctf-suid-escape");
d.skip("net-egress-block");
render(d);
