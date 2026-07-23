# Bee

Bee runs coding-agent sessions whose tools receive capability-scoped access to a host. The same session model supports headless sandbox testing and an interactive REPL.

## Language

**User configuration**:
Protected, operator-owned configuration under `~/.bee/` that defines trusted defaults and the maximum authority available to Bee sessions. Project content and agents cannot weaken or widen its authority.
_Avoid_: Global config, personal config

**Project configuration**:
Repository-local configuration under `.bee/` that describes project defaults and may request authority within the user configuration. It is treated as untrusted input when the repository is untrusted.
_Avoid_: Repo config, workspace config

**Effective configuration**:
The validated, complete result of resolving explicit operator intent, user configuration, and project configuration. Individual configuration files are optional, but Bee fails before execution when the available sources are incomplete; the result never grants authority beyond the user configuration.
_Avoid_: Merged config, resolved config
