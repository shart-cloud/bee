# Configuration files are optional inputs

Bee may resolve configuration from explicit arguments, an explicitly supplied file, project configuration under `.bee/`, and user configuration under `~/.config/bee/`; no individual configuration file is mandatory. Before contacting a provider or executing any tool, Bee must produce a complete effective configuration or fail with the missing requirements, which preserves one-off CLI use without introducing implicit unsafe defaults.

## Amendment: user configuration is an XDG path

This decision originally placed user configuration under `~/.bee/`. It is amended to `~/.config/bee/config.toml`, honouring `XDG_CONFIG_HOME`.

Bee had already shipped that path — the `[theme]` section has been read from it since 005-themes — and the metrics log had shipped at `~/.local/state/bee/` under the same convention. Introducing `~/.bee/` would have meant two locations for user configuration and a migration for the only bee config file anyone has. XDG also draws the configuration/state distinction this decision needs: the metrics log stays where it is because it is state, not configuration.

Project configuration is unaffected and remains `.bee/config.toml`, relative to the working directory.
