# Configuration files are optional inputs

Bee may resolve configuration from explicit arguments, an explicitly supplied file, project configuration under `.bee/`, and user configuration under `~/.bee/`; no individual configuration file is mandatory. Before contacting a provider or executing any tool, Bee must produce a complete effective configuration or fail with the missing requirements, which preserves one-off CLI use without introducing implicit unsafe defaults.
