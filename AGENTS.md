# Repository instructions

- This is a Rust 1.98.0 workspace (`crates/*`): `terraforme-core` owns application logic; `terraforme-cli` is the `terraforme` binary, argument handling, and TUI. Keep new business logic in core.
- Agent integrations are adapters under `crates/terraforme-core/src/agent/`. `Provider::adapter()` is the source of truth: Claude and OpenCode are implemented; Cursor is not. The prose in some docs still describes Claude as the only integration. Read `CLAUDE.md` before changing agent setup, jail behavior, usage accounting, or TUI details.

## Build and verification

- `rust-toolchain.toml` pins Rust 1.98.0 with `rustfmt` and `clippy`.
- Run `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, then `cargo test --workspace`. To run a focused test: `cargo test -p terraforme-core <test-name>` (or choose the relevant crate).
- Run the CLI with `cargo run -p terraforme-cli -- <args>`; `pnpm cli -- <args>` is the root shortcut. Node/pnpm are otherwise only needed to install the Husky commit hooks (`pnpm install`).
- Release builds intentionally omit LTO for faster compilation; use `cargo build -p terraforme-cli --release` for normal release builds and `cargo build -p terraforme-cli --profile dist` only for optimized distribution builds.

## Project conventions

- User-facing strings, docs, and code comments are in Portuguese; commit messages are in English. Commit format and allowed scopes are enforced by `commitlint.config.mjs`.
- For shipped features, fixes, behavior changes, or build changes (not pure refactors/docs), bump `[workspace.package].version` in the root `Cargo.toml` and add a dated `## X.Y.Z - YYYY-MM-DD` section at the top of `CHANGELOG.md`; both crates inherit that version.
