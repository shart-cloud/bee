# Contract: keybindings & reserved keys

The full-screen front-end's key map. Follows the cross-app conventions from the `tui-design` skill.
Satisfies FR-005, FR-006, FR-016, FR-017.

## Always-visible footer hints (the 4–5 most useful)

```
 Enter send   ↑↓/PgUp/Dn scroll   Tab focus   y yank   ? help   q quit
```

Full reference behind `?`.

## Key map

| Key | Context | Action |
|---|---|---|
| `Enter` | input focus | submit the message (Shift+Enter / `\` continuation → newline) |
| `↑` / `↓`, `PgUp` / `PgDn` | chat focus | scroll history; `↑`/`↓` in input walks input history |
| `gg` / `G` | chat focus | jump to top / bottom (follow-tail) |
| `Tab` / `Shift+Tab` | any | cycle focus: Input → Chat → Panels |
| `p` | any | toggle the panel overlay (single-pane layouts) |
| `y` | chat focus | yank the current/selected message to the clipboard via **OSC 52** (FR-016) |
| `?` | any | open/close the help screen (full key list) |
| `q` / `Ctrl-D` | non-input focus | quit (restores the terminal — see modes contract) |
| `Esc` | overlay/help open | close it; else clear the input line |
| `Mouse wheel` / click | where natural | scroll a pane / focus a pane (augmentation only) |

Every action is reachable from the keyboard; the mouse is never required (FR-017 / tui-design).

## Reserved — never rebound (FR-017)

These keep their **terminal** meaning and are not application actions:

- `Ctrl-C` (SIGINT) → clean quit with terminal restore (not an app binding that swallows the signal).
- `Ctrl-Z` (SIGTSTP) → suspend (leave alt-screen, restore, re-raise; redraw on `SIGCONT`).
- `Ctrl-\` (SIGQUIT), `Ctrl-S`/`Ctrl-Q` (flow control) → left to the terminal.

## Discoverability ladder

1. Footer hints (above) — always visible.
2. `?` — full key reference.
3. (Future) a command palette — out of scope for v1, noted for M5.
