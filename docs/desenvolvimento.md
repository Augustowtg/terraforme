# Desenvolvimento

Como compilar, testar, versionar e lançar o terraforme. Para quem vai contribuir com código; a
organização do código está na [Arquitetura](arquitetura.md).

## Pré-requisitos

- **Rust** via [rustup](https://rustup.rs) — o `rust-toolchain.toml` fixa a 1.98.0 (edition 2024)
  e instala `rustfmt` e `clippy`.
- **Node.js 20+ e pnpm** (`corepack enable` ou `npm i -g pnpm`) — só para os hooks do git; rode
  `pnpm install` na raiz para ativar o Husky.
- **Windows** — [Microsoft C++ Build Tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/)
  ("Desenvolvimento para desktop com C++").

## Compilar e rodar

```bash
cargo run -p terraforme-cli -- --help
pnpm cli --help                                  # atalho pela raiz

cargo build -p terraforme-cli --release          # target/release/terraforme(.exe)
cargo build --profile dist -p terraforme-cli     # fat LTO, para distribuir
```

| Perfil | Uso |
| --- | --- |
| `release` | padrão, sem LTO de propósito: com fat LTO + 1 unidade de código, o último binário compilava num só núcleo por 35s+ e parecia travado |
| `dist` | herda o `release` com `lto = true` e `codegen-units = 1`; usado pelas releases e por `--dist` |

Para instalar a partir do código (constrói `terraforme`, `terraforme-mcp` e, no Linux,
`terraforme-pet`):

```bash
./install.linux.sh               # --update (git pull antes), --dist, --uninstall
.\install.windows.ps1            # -Update, -Dist, -Uninstall
```

- O `install.windows.ps1` precisa do BOM UTF-8 (PowerShell 5.1); o `install.ps1` precisa ficar em
  ASCII e dentro de `& { }` (roda na sessão do usuário: nada de `exit`).
- Depois de instalar, se um daemon MCP ou a janela flutuante antigos estiverem no ar, os scripts
  perguntam se rodam `terraforme restart`.

## Qualidade e testes

```bash
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo test -p terraforme-core toml_round_trip    # um teste (filtro por nome)
```

- O código em `#[cfg(windows)]` só é compilado na CI; checar `x86_64-pc-windows-msvc` localmente
  falha no `ring` (precisa do `lib.exe`).
- Rode `fmt`, `clippy` e `test` antes de abrir um PR: a CI não roda em push nem em PR.

## Versões e CHANGELOG

- Toda mudança entregue (feature, correção, mudança de comportamento ou de build — não refactor
  puro nem docs) sobe `[workspace.package] version` no `Cargo.toml` da raiz. SemVer: minor para
  features, patch para correções. Os crates herdam a versão; atualize o `Cargo.lock` com um build
  ou `cargo metadata`.
- Cada versão ganha uma seção `## X.Y.Z - AAAA-MM-DD` no topo do [`CHANGELOG.md`](../CHANGELOG.md),
  em português, com os títulos `Adicionado` / `Alterado` / `Corrigido` / `Removido`.
- Várias mudanças da mesma sessão não lançada entram na mesma versão nova.
- Mantenha o formato `## <versão>`: o `install.linux.sh` mostra as seções mais novas que a versão
  instalada depois de uma atualização, e a release usa a seção da versão como notas.

## Branches

- `main` — versões lançadas, cada uma com a tag `vX.Y.Z`.
- `develop` — integração do dia a dia.
- `feat/<nome>`, `fix/<nome>` — saem de `develop` e voltam para ela.

## Commits

[Conventional Commits](https://www.conventionalcommits.org), **sempre em inglês**:

```
<type>(<scope>): <subject>

feat(cli): add init command
fix(core): handle empty space name
chore(deps): bump clap to 4.6
```

Regras do `commitlint.config.mjs` (sobre `@commitlint/config-conventional`):

| Regra | Valor |
| --- | --- |
| types | `feat`, `fix`, `docs`, `style`, `refactor`, `perf`, `test`, `build`, `ci`, `chore`, `revert` |
| scopes (opcional) | `core`, `cli`, `deps`, `ci`, `docs`, `config`, `release` |
| subject | minúsculas |
| header | até 100 caracteres |
| idioma | rejeita caracteres fora do ASCII e palavras comuns em português (`para`, `uma`, `cria`…) |

| Hook (Husky) | O que roda |
| --- | --- |
| `pre-commit` | `cargo fmt --check` e `cargo clippy -D warnings` |
| `commit-msg` | `commitlint` |

## CI

`.github/workflows/ci.yml` roda `cargo fmt --all --check`, `clippy` e `test` (`--locked`) em
`ubuntu-latest` e `windows-latest`, **só** em tags `v*` e no disparo manual (`workflow_dispatch`).

## Releases

`.github/workflows/release.yml` roda numa tag `v*`, que precisa ser `v` + a versão do `Cargo.toml`:

1. compila `terraforme` e `terraforme-mcp` com o perfil `dist` para:

   | Alvo | Pacote |
   | --- | --- |
   | `x86_64-unknown-linux-musl` | `terraforme-x86_64-unknown-linux-musl.tar.gz` |
   | `aarch64-unknown-linux-musl` | `terraforme-aarch64-unknown-linux-musl.tar.gz` |
   | `x86_64-pc-windows-msvc` | `terraforme-x86_64-pc-windows-msvc.zip` |

   Cada pacote vem com um `.sha256`; os alvos musl usam `musl-gcc` por causa do `ring`.
2. cria a GitHub Release com a seção da versão no `CHANGELOG.md`.

- O `install.sh`/`install.ps1` baixam esses pacotes: mantenha os nomes em sincronia.
- O `terraforme-pet` não vai na release: os binários musl não conseguem carregar X11/Wayland/GL
  (veja o [ROADMAP](../ROADMAP.md)).
