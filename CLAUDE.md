# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project

`terraforme` is a Rust CLI (Linux and Windows) that installs and manages three extensions for AI agents — [RTK](https://github.com/rtk-ai/rtk) (token savings), [AI Memory](https://github.com/akitaonrails/ai-memory) (cross-session memory) and [AI Jail](https://github.com/akitaonrails/ai-jail) (OS sandbox) — and launches the agent (currently only Claude Code) in modes that toggle the first two, optionally inside the jail. User-facing text, doc comments and code comments are in **Portuguese**; commit messages must be in **English**.

## Commands

```bash
cargo run -p terraforme-cli -- --help        # or: pnpm cli --help
cargo build -p terraforme-cli --release      # target/release/terraforme
./install.linux.sh                           # release build + install to ~/.local/bin (--update: git pull first; --dist; --uninstall)
terraforme upgrade                           # execs <repo>/install.linux.sh --update (repo = CARGO_MANIFEST_DIR/../.. at build time, or --repo / TERRAFORME_REPO)

cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo test -p terraforme-core toml_round_trip   # single test (name filter)
```

`release` has no LTO on purpose (fat LTO + 1 codegen unit left the last step, the `terraforme` binary, compiling on a single core for ~35s+, looking stuck at 181/182); the `dist` profile keeps that for distribution builds.

Toolchain is pinned to Rust 1.98.0 (edition 2024) via `rust-toolchain.toml`. Node/pnpm exist only for git hooks: `pnpm install` at the root enables Husky. `pre-commit` runs `cargo fmt --check` and `cargo clippy -D warnings`; `commit-msg` runs commitlint.

## Versioning and changelog

Every change that ships (feature, fix, behavior or build change — not pure refactors/docs) bumps `[workspace.package] version` in the root `Cargo.toml` (SemVer: minor for features, patch for fixes; both crates inherit it, refresh `Cargo.lock` with `cargo metadata` or a build) and adds a `## X.Y.Z - YYYY-MM-DD` section at the top of `CHANGELOG.md` (Portuguese, Keep a Changelog headings `Adicionado` / `Alterado` / `Corrigido` / `Removido`). Several changes in the same unreleased session go into the same new version. `install.linux.sh` prints the sections newer than the previously installed version after an update, so keep the `## <version>` heading format.

## Commits

Conventional Commits, `<type>(<scope>): <subject>`. Rules in `commitlint.config.mjs`: allowed scopes are `core`, `cli`, `deps`, `ci`, `docs`, `config`, `release`; subject lower-case; header ≤ 100 chars; any non-ASCII character or common Portuguese word (e.g. `para`, `uma`, `cria`) is rejected.

## Architecture

Cargo workspace with two crates:

- **`crates/terraforme-core`** — all logic. The CLI is just a shell around it. The optional `clap` feature derives `clap::ValueEnum` on public enums (`Extension`, `Provider`, `Mode`) so the CLI can use them directly as arguments.
- **`crates/terraforme-cli`** — the `terraforme` binary: clap definitions and output formatting in `main.rs`, plus the TUI in `term.rs`.

Core modules:

- `status` — `Component`/`State` (Installed / Conflict / NotInstalled) and PATH lookup + `--version` parsing helpers shared by the other modules.
- `extension` — install/update/delete for RTK and AI Memory using each project's official install scripts and their own subcommands (AI Jail has no install script: the release tarball is downloaded, checked against its `.sha256` and installed to `~/.local/bin`; update/delete refuse a copy outside it, e.g. Homebrew); external commands inherit the terminal. Also defines `Report`s (e.g. `rtk gain`, `ai-memory status`), which run the extension binary with `NO_COLOR` and return ANSI-stripped text.
- `usage` — Claude Code token consumption parsed from the transcripts in `~/.claude/projects/<cwd with non-alphanumerics → '-'>/<session>.jsonl` (respects `CLAUDE_CONFIG_DIR`). One response spans several lines, so entries are deduplicated by `message.id` + `requestId`; the current session is the newest transcript modified since launch. Subagent transcripts live in `<session>/subagents/agent-<id>.jsonl` (same `sessionId`, plus `agentId`; type/description in the sibling `.meta.json`).
- `limits` — how much of the usage limits is left: the plan's limits from `api.anthropic.com/api/oauth/usage` (OAuth token read from `<claude dir>/.credentials.json`, never refreshed by us; cached 60s), falling back to the config `[budget]` compared with transcript totals.
- `config` — `Config { provider, mode, jail, budget }` in `~/.config/terraforme/config.toml` (`%APPDATA%\terraforme` on Windows; respects `XDG_CONFIG_HOME`).
- `space` — named working directories (`Spaces`, a name → path `BTreeMap`) in `<config dir>/spaces.toml`, kept out of `config.toml` because `Config` is `Copy`. `terraforme space` picks one (list number or name) and `exec`s the user's shell there (`space::shell`, sets `TERRAFORME_SPACE`), since a process can't change its parent shell's cwd; `--path` prints the dir instead (prompts go to stderr) for a `cd "$(…)"` shell function.
- `jail` — AI Jail integration: `DEFAULT_CONFIG` written to `~/.ai-jail` only when absent (on install and on a jailed launch), and `wrap()`, which turns the agent command into `ai-jail --no-save-config <maps> -- claude <args>`. Maps are passed per launch, never saved: each mode extension's plugin dir and binary (plus its symlink target dir) read-only, `~/.local/share/{rtk,ai-memory}` writable, and `--forward-port` for every `127.0.0.1:`/`localhost:` port found in the plugin's `hooks.json`/`.mcp.json` (AI Memory's local server). Requires ai-jail ≥ 2.9 (`--forward-port`) and `bwrap` on Linux. Never pass `--no-status-bar` on the CLI: ai-jail persists status-bar flags into `~/.ai-jail`.
- `agent` — `Provider` and `Mode` (`eco` = RTK, `high` = AI Memory, `hybrid` = both; AI Jail is never part of a mode — `Mode::uses` is false for it). `agent::prepare()` builds the `std::process::Command` that `terraforme cli` runs in the TUI (or directly, with `exec`, under `--plain`).

### How modes work with Claude Code (`agent::prepare`)

This is the non-obvious part and it touches the user's global Claude config:

1. RTK/AI Memory hooks are **moved out** of `~/.claude/settings.json` (backup saved as `settings.json.terraforme-bak`) into per-extension local plugins at `~/.config/terraforme/plugins/terraforme-<ext>/hooks/hooks.json`. This happens on every launch, so if `rtk init -g` / `ai-memory setup` puts the hooks back in the global file, they are moved again. The global file is only rewritten after the plugins have been written.
2. For each extension used by the mode, `--plugin-dir <plugin>` is passed to `claude`.
3. The global AI Memory MCP server in `~/.claude.json` is **not modified**: it is blocked for the session via `--settings '{"deniedMcpServers":[{"serverName":"ai-memory"}]}'`, and the AI Memory plugin gets a copy of it in its `.mcp.json` (as server `memory`) when the mode uses it.
4. Extra args after `--` are passed through to `claude`.
5. With `jail` (`config.jail`, overridden by `--jail`/`--no-jail`), the whole command is wrapped by `jail::wrap`, and the TUI gets an AI Jail reports tab.

As a result, running `claude` directly (without terraforme) gives a session without the RTK and AI Memory hooks.

### `terraforme cli` terminal (`term.rs`)

ratatui TUI with a tab bar on top: one agent tab (`claude`) whose sub-tabs, on a second line, are the agent instances (`claude 1`, `claude 2`…; `--instances/-n N` at launch, `F12` opens another, `Shift+F12`/`F24` kills the active one, `Alt+1`–`9` / `Ctrl+PgUp/PgDn` switch, max 9; a sub-tab disappears when its process exits and the TUI quits when the last one does), `Consumo` (sub-tabs Limites / Tokens / Subagentes from `usage::collect` + `limits::collect`, auto-refreshed every 5s, for the focused instance — each instance gets its own `--session-id` unless `agent::picks_session` sees `--continue`/`--resume`/`--session-id` in the passthrough args, and `usage::collect` takes that id) and `Extensões` when the mode uses any (one sub-tab per `Report` of every extension, grouped by extension). Each instance runs in its own `portable-pty` pseudoterminal, resized to the real screen before every draw (`App::fit`, not only on `Event::Resize`), rendered with `vt100` + `tui-term` (5000-line scrollback) and keeps running while another tab is open; non-agent tabs load their content on a background thread when selected. `Shift+Tab` always switches tabs (so it never reaches the agent; `Tab` and plain `F1`–`F9` do), `Shift+F1`–`F9` jump to a tab (some terminals send them as `F13`–`F21`), `Shift+1`–`9` (the shifted characters `!@#$`…) jump to a sub-tab, `F10` quits. Key events are translated to xterm byte sequences in `key_bytes` (which respects application-cursor mode). Notices/warnings from `prepare()` are shown in an initial popup, because the alternate screen would hide anything printed before it.
