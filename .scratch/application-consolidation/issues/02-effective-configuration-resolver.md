# Effective configuration resolver

Status: ready-for-agent

Part of [Bee application and workspace consolidation](../PRD.md).

## Problem Statement

Bee has no single answer to "what is the configuration for this session". Each axis resolves on its
own, from its own sources, in its own order. The theme resolves from a flag, then `BEE_THEME`, then
the `[theme]` section of `~/.config/bee/config.toml`, and falls back to a built-in on anything
unrecognised. The visual level resolves from a flag, then `BEE_VISUAL_LEVEL`, and fails startup on
anything unrecognised. The provider, policy, ceiling policy, tool list, and MCP configuration resolve
from their flags and from nowhere else — there is no file that can supply them, so every invocation
carries them explicitly.

Two consequences follow. First, `CONTEXT.md` defines user configuration, project configuration, and
effective configuration as domain terms, and ADR-0001 commits to resolving them into a complete
result or failing — but none of that exists in code; the glossary describes an intention. Second,
because nothing assembles a complete configuration, nothing is positioned to refuse to start when
the configuration is incomplete. That refusal is the subject of issue 05, and it needs this resolver
underneath it.

There is also a live contradiction to settle. ADR-0001 and `CONTEXT.md` place user configuration
under `~/.bee/`, but the shipped code already reads `~/.config/bee/config.toml` and writes metrics
to `~/.local/state/bee/`. Two locations for user configuration is worse than either one.

## Solution

Add a configuration module to the application package that resolves one `EffectiveConfig` from four
sources, highest precedence first: explicit flags, an explicitly supplied `--config` file, project
configuration at `.bee/config.toml`, and user configuration at `~/.config/bee/config.toml`
(honouring `XDG_CONFIG_HOME`, exactly as the existing `viz::theme::config_path` does — reuse that
function rather than writing a second path resolver). No file is mandatory. Environment variables
keep their current position: below flags, above files, for the axes that already read them.

Amend ADR-0001 and `CONTEXT.md` to name the XDG path for user configuration. Project configuration
stays `.bee/config.toml`. The metrics log stays under `~/.local/state/bee/` because it is state, not
configuration, and XDG already distinguishes those. This amendment is a deliberate, recorded
override of what ADR-0001 says today, not a silent drift — the ADR gains a note saying so.

Classify every configuration field by trust, because project configuration is untrusted input when
the repository is untrusted. User-only fields — the capability ceiling and the name of the
environment variable holding the provider key — may appear only in user configuration or in explicit
flags; a project file that sets one is a startup error naming the offending field, not a value that
gets quietly dropped. Requestable fields — the session policy, the tool list, the system prompt, the
turn budget, the theme, the visual level, the MCP configuration — may be proposed by project
configuration and are resolved within the ceiling.

Resolve the policy request against the ceiling with `bee_core::Policy::derive`. This is the existing
attenuation validator, the same one behind subagent policies in `bee-cli`'s validate path and behind
skill capability grants in the REPL. A project policy that exceeds the ceiling produces the derive
error, unchanged, rather than a new and differently-worded refusal.

Return either a complete `EffectiveConfig` or a structured list of missing requirements. Each
missing requirement names the field and where it could be supplied — a flag, the project file, or
the user file — so the error text tells the operator what to do rather than only what went wrong.
Nothing consumes the resolver in this issue; `bee run` and `bee repl` are wired onto it in issues 03
and 04.

## Commits

1. **Amend the documentation.** Update `docs/adr/0001-configuration-files-are-optional.md` and the
   user-configuration entry in `CONTEXT.md` to name `~/.config/bee/config.toml`, with a sentence
   recording that this supersedes the `~/.bee/` path the ADR originally proposed and why (the code
   already shipped XDG paths, and the metrics state path follows the same convention).

2. **Define the configuration schema.** Extend the existing config file schema — currently a root
   table with an optional `[theme]` section — with the session sections: provider, policy, session
   parameters, visual, and MCP. Serde types with every field optional, since a file that supplies
   one thing must not be required to supply the rest. Round-trip tests, plus a test that today's
   theme-only config still parses unchanged.

3. **Add source discovery.** Locate the project file at `.bee/config.toml` relative to the working
   directory and the user file via the existing `viz::theme::config_path`. A missing file is
   absence, not an error; a malformed file is an error naming the file and the parse position.
   Tests for present, absent, and malformed at each layer.

4. **Implement precedence.** Merge the layers field by field, highest precedence winning per field
   rather than per file, so a project file that sets only the tool list does not discard the user
   file's theme. One test per precedence rule, and a test that four layers setting the same field
   resolve to the flag's value.

5. **Enforce the trust classification.** Reject user-only fields appearing in project configuration
   with an error naming the field and the file. Test each user-only field individually, and test
   that the same field in user configuration or on a flag is accepted.

6. **Attenuate the policy request.** Derive the requested policy against the ceiling with
   `Policy::derive`, propagating its error verbatim on an over-grant. Tests for a within-ceiling
   request, an exceeding request, a request with no ceiling configured, and a ceiling with no
   request.

7. **Report missing requirements.** Assemble the completeness check and the structured missing-
   requirement list. Test the message for each individually-missing requirement and for several
   missing at once, asserting each names a place it could be supplied.

## Verification

- `cargo test` and `cargo clippy --workspace --all-targets` green.
- Unit tests cover every precedence rule, every trust classification, and every missing-requirement
  message; the resolver is a pure function over discovered inputs, so it needs no session to test.
- An existing `~/.config/bee/config.toml` containing only a `[theme]` section still resolves, still
  yields that theme, and reports no error. This is the compatibility case that matters — it is the
  only bee config file anyone has today.
- No behaviour change is observable from any command yet. `bee check`, `bee validate`, and
  `bee exec` behave exactly as they did after issue 01.
