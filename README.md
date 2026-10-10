# cashell

> CA ecosystem CLI — universal shell for CATerm & tools

## Quick Start

```bash
# Install
curl -fsSL https://cashell.fath.my.id/install.sh | sh

# Login
cashell auth login

# Browse products
cashell product list
cashell product info caterm

# Version
cashell version

# Config
cashell config show
cashell config set api_base https://api.fathforce.com
```

## Subcommands

| Command | Description |
|---------|-------------|
| `auth login|status|logout` | Token management |
| `product list|info <slug>|browse` | CA product catalog |
| `version` | Build & version info |
| `config show|set|reset` | Config file management |

### TUI Browser (`cashell product browse`)

Interactive full-screen terminal UI built with **ratatui + crossterm** — no web, pure terminal cells.

- `↑` / `↓` or `j` / `k` — move selection
- `Enter` / `i` — toggle detail pane
- `Esc` / `q` — back / quit

Refuses to run when stdout is not a TTY (safe in scripts/CI).

## Global Flags

- `-c, --config <file>` — custom config path
- `-o, --output <fmt>` — `text` | `json` | `auto` (default: auto)

## Output Modes

- **TTY mode:** colored output, markdown-style tables, progress bars
- **Pipe mode:** clean plain text or JSON (for CI/scripting)

## Development

```bash
# Build
cargo build --release

# Run
cargo run -- auth status

# Test
cargo test
```

## Architecture

- Pure Rust CLI — no Node.js, no webview, no external runtime
- Static musl binary (`x86_64-unknown-linux-musl`) — runs on any Linux distro
- Config stored at `~/.config/cashell/config.toml`
- Error handling via `thiserror` + `anyhow`
- TTY detection via `stdout.is_terminal()`

## Roadmap

- [x] TUI catalog browser (`ratatui`) — `product browse`
- [ ] `sync` — sync data with CATerm cloud
- [ ] `deploy` — deploy CATerm/FathStore builds
- [ ] `doctor` — health check for CA ecosystem
- [ ] GitHub Actions auto-release on tag push

## License

MIT — © Cecep Saeful Azhar Hidayat, ST
