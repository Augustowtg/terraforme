# Desenvolvimento

## Pré-requisitos

- Rust (via [rustup](https://rustup.rs)) — a versão é instalada automaticamente pelo
  `rust-toolchain.toml`
- Node.js 20+ e pnpm (`corepack enable` ou `npm i -g pnpm`) — só para os hooks do git (Husky +
  commitlint); rode `pnpm install` na raiz para ativá-los
- **Windows** — [Microsoft C++ Build Tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/)
  ("Desenvolvimento para desktop com C++")

## Compilar e rodar

```bash
cargo run -p terraforme-cli -- --help
pnpm cli --help                                # atalho pela raiz
cargo run -p terraforme-cli -- completions bash > terraforme.bash

cargo build -p terraforme-cli --release        # target/release/terraforme(.exe)
cargo build -p terraforme-cli --profile dist   # target/dist/terraforme — fat LTO, para distribuir
```

O perfil `release` não usa LTO de propósito: com fat LTO e uma unidade de código, o último passo (o
binário) compilava num só núcleo por 35s+, parecendo travado. O perfil `dist` mantém isso para
builds de distribuição.

## Qualidade

```bash
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

## Versões

Toda alteração entregue (feature, correção, mudança de comportamento ou de build) sobe a versão em
`Cargo.toml` (`[workspace.package] version`, SemVer: minor para features, patch para correções) e
ganha uma seção `## X.Y.Z - AAAA-MM-DD` no topo do [`CHANGELOG.md`](../CHANGELOG.md). O
`install.linux.sh` mostra as seções mais novas que a versão instalada depois de uma atualização.

## Branches

- `main` — versões lançadas, cada uma com a tag `vX.Y.Z`.
- `develop` — integração do dia a dia.
- `feat/<nome>` e `fix/<nome>` — saem de `develop` e voltam para ela.

## Commits

Seguimos [Conventional Commits](https://www.conventionalcommits.org), **sempre em inglês**.

```
<type>(<scope>): <subject>

feat(cli): add init command
fix(cli): handle empty name on greet
chore(deps): bump clap to 4.6
```

- **types**: `feat`, `fix`, `docs`, `style`, `refactor`, `perf`, `test`, `build`, `ci`, `chore`, `revert`
- **scopes** (opcional): `core`, `cli`, `deps`, `ci`, `docs`, `config`, `release`
- subject em minúsculas, até 100 caracteres no header
- mensagens com acentos ou palavras comuns em português são rejeitadas

| Hook | O que roda |
|---|---|
| `pre-commit` | `cargo fmt --check` e `cargo clippy -D warnings` |
| `commit-msg` | `commitlint` (regras em `commitlint.config.mjs`) |
