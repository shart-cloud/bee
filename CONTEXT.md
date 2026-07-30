# Bee

Bee runs coding-agent sessions whose tools receive capability-scoped access to a host. The same session model supports headless sandbox testing and an interactive REPL.

## Language

**User configuration**:
Protected, operator-owned configuration at `~/.config/bee/config.toml` (honouring `XDG_CONFIG_HOME`) that defines trusted defaults and the maximum authority available to Bee sessions. Project content and agents cannot weaken or widen its authority.
_Avoid_: Global config, personal config

**Project configuration**:
Repository-local configuration at `.bee/config.toml` that describes project defaults and may request authority within the user configuration. It is treated as untrusted input when the repository is untrusted.
_Avoid_: Repo config, workspace config

**Effective configuration**:
The validated, complete result of resolving explicit operator intent, user configuration, and project configuration. Individual configuration files are optional, but Bee fails before execution when the available sources are incomplete; the result never grants authority beyond the user configuration.
_Avoid_: Merged config, resolved config

**Native tool**:
A security-analysis tool whose engine is a maintained Rust crate compiled into Bee, run as a scope-joined `bee <name>-worker` child — structural search, repository history, severity scoring. The rule is which side of the buy/build line the *value* sits on: when the value is the engine, Bee links the crate.
_Avoid_: Built-in tool, internal scanner

**External scanner**:
A third-party analysis binary whose value is its curated **rule corpus** rather than its engine, run only under an inode-pinned `exec.allow` grant with argv Bee constructs from typed inputs. Bee never reimplements a corpus and never hands the model a command line. One scan may be several children — a version pin verified before use, then the analysis itself — but every one of them is a child, because asking a binary what it is, running it, and reading what it wrote are all things the harness does from outside the scope if it does them itself.
_Avoid_: Plugin, integration, shell-out

**Finding ledger**:
The project's durable, append-only record of security findings at `.bee/findings/ledger.jsonl`, folded into a derived `view.json`. Re-runs merge into it; a human verdict recorded in it outranks every later automated rediscovery.
_Avoid_: Findings database, results file, report

**Finding**:
One reviewable observation about the code, identified by its path, issue class, and normalised title — deliberately **not** its line number, so code movement alone never mints a duplicate. Each occurrence is a *sighting*.
_Avoid_: Issue, alert, hit

**Verdict**:
A person's adjudication of a finding — confirmed, false positive, fixed, duplicate, deferred — recorded only through `bee findings adjudicate`. It is never writable by an agent: a verdict is worth something precisely because a human formed it.
_Avoid_: Status, triage state, disposition
