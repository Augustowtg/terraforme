# terraforme

CLI em Rust, para **Linux** e **Windows**.

## Estrutura

```
.
├── crates/
│   ├── terraforme-core/      # regras de negócio (compartilhadas)
│   └── terraforme-cli/       # CLI (clap) → binário `terraforme`
├── docs/                     # documentação local (fora do git)
└── rust-toolchain.toml       # Rust 1.98.0 + alvos Linux/Windows
```

Toda a lógica vai no `terraforme-core`; a CLI é apenas uma "casca" que o chama.

## Pré-requisitos

- Rust (via [rustup](https://rustup.rs)) — a versão é instalada automaticamente pelo `rust-toolchain.toml`
- Node.js 20+ e pnpm (`corepack enable` ou `npm i -g pnpm`) — só para os hooks do git (Husky + commitlint)

**Windows** — [Microsoft C++ Build Tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/)
("Desenvolvimento para desktop com C++").

## CLI

Instalar ou atualizar (Linux) — compila em release e copia para `~/.local/bin`:

```bash
./install.linux.sh                       # instala/recompila o código local
./install.linux.sh --update              # git pull (fast-forward) + compila + instala
./install.linux.sh --dist                # build otimizada (LTO), mais lenta para compilar
PREFIX=/usr/local ./install.linux.sh     # em /usr/local/bin (usa sudo)
./install.linux.sh --uninstall           # remove

terraforme upgrade                       # o mesmo que --update, a partir do próprio terraforme
terraforme upgrade --no-pull             # só recompila (--repo DIR / TERRAFORME_REPO se o repositório mudou de lugar)

terraforme                               # mostra a ajuda (mesmo que --help)
terraforme --version                     # ou -V, ou `terraforme version`
terraforme status                        # extensões (RTK, AI Memory, AI Jail)
```

Extensões ([RTK](https://github.com/rtk-ai/rtk), [AI Memory](https://github.com/akitaonrails/ai-memory) e
[AI Jail](https://github.com/akitaonrails/ai-jail)):

```bash
terraforme extension --status              # status e versão de todas
terraforme extension rtk --status          # de uma só
terraforme extension --install             # instala todas (pula as já instaladas)
terraforme extension ai-memory --install   # instala uma só
terraforme extension rtk --update          # atualiza (nome obrigatório)
terraforme extension rtk --delete          # remove (nome obrigatório; -y pula a confirmação)
```

A instalação usa os scripts oficiais de cada projeto (em `~/.local/bin`) e já configura
os agentes (`rtk init -g` / `ai-memory setup`). A remoção desfaz essa configuração e apaga o
binário; os dados do AI Memory (wiki, banco) são preservados. O RTK instala por padrão a
edição BUSL-1.1 — para a Apache-2.0: `RTK_EDITION=oss terraforme extension rtk --install`.

O AI Jail não tem script de instalação: o terraforme baixa o binário do último release (Linux
x86_64 ou macOS arm64), confere o `.sha256` publicado e o instala em `~/.local/bin`; no Linux ele
precisa do `bwrap` (pacote bubblewrap). Update/delete só mexem nessa cópia — uma instalada por
Homebrew/AUR/cargo/Nix é atualizada pelo próprio gerenciador (o terraforme exige a 2.9+). A
instalação também cria uma configuração padrão em `~/.ai-jail` se ainda não houver uma (a
remoção a preserva):

```toml
no_save_config = true     # não grava .ai-jail em cada projeto
no_status_bar = true      # o terminal do terraforme já tem as abas

[commands.claude]
agent_state = true        # ~/.claude e ~/.claude.json montados: sessão já logada
```

Agente de IA (provider) e modos — config em `~/.config/terraforme/config.toml`:

```bash
terraforme config                    # mostra provider e modo padrão
terraforme config --provider         # lista os providers (claude; cursor ainda não implementado)
terraforme config --provider claude  # define o provider
terraforme config --mode eco         # define o modo padrão
terraforme config --jail true        # roda o agente dentro do AI Jail por padrão (false desliga)

terraforme cli                       # abre o agente no modo padrão
terraforme cli --eco                 # só RTK (economia de tokens)
terraforme cli --high                # só AI Memory (memória entre sessões)
terraforme cli --hybrid              # RTK + AI Memory
terraforme cli --eco -- --continue   # depois de `--`, argumentos vão direto para o claude
terraforme cli --plain               # abre o agente direto, sem o terminal do terraforme
terraforme cli -n 3                  # três instâncias do agente, sub-abas da aba claude (até 9)
terraforme cli --jail                # dentro do AI Jail (sandbox), em qualquer modo; --no-jail fora
```

O `terraforme cli` abre o agente dentro do terminal do terraforme, com abas no topo:

```
 terraforme   ⇧F1 claude   ⇧F2 Consumo   ⇧F3 RTK · gain   ⇧F4 AI Memory   F10 sair   ⇧Tab alterna · …
```

- **claude** — o agente. Cada instância é uma sub-aba (`claude 1`, `claude 2`…), todas rodando ao
  mesmo tempo; `F12` abre outra, `Shift+F12` fecha a aberta e `Alt+1`…`Alt+9` ou
  `Ctrl+PgUp/PgDn` trocam de instância. A sub-aba some quando a instância termina, e o terraforme
  sai quando a última termina.
- **Consumo** — três sub-abas, atualizadas a cada 5s:
  - **Limites** — barra de progresso de quanto já foi usado e quanto falta. Com login OAuth no
    Claude Code (Pro/Max/Team), usa os limites do plano (sessão de 5h e semana, os mesmos do
    `/usage`, consultados a cada 60s); sem login, compara os transcripts com o orçamento definido
    em `terraforme config --budget-5h <tokens> --budget-week <tokens>`.
  - **Tokens** — entrada, saída e cache da sessão atual (a da instância aberta na aba claude), do projeto nas últimas 24h e de todos os
    projetos em 5h/24h/7 dias, lidos dos transcripts do Claude Code (`~/.claude/projects`).
  - **Subagentes** — a sessão atual dividida entre o agente principal e cada subagente (tipo e
    descrição).
- **RTK · gain** — só nos modos com RTK: `rtk gain` (geral, neste projeto, por dia, histórico).
- **AI Memory** — só nos modos com AI Memory: status, projetos e handoffs.
- **AI Jail** — só com o AI Jail ligado: `ai-jail status` e a sandbox montada (`--dry-run`).

| Tecla | Ação |
| --- | --- |
| `Shift+Tab` | próxima aba |
| `Shift+F1`…`Shift+F4` | vai direto para a aba |
| `Shift+1`…`Shift+4` | vai direto para a sub-aba (também `←→`) |
| `Alt+1`…`Alt+9` / `Ctrl+PgUp/PgDn` | vai para a instância do agente (sub-aba da aba claude) |
| `F12` / `Shift+F12` | abre outra instância do agente / fecha a aberta |
| `F10` | sai (encerra todas as instâncias) |
| `Shift+PgUp/PgDn` | rola o histórico do agente |

Nas abas de relatório: `↑↓`/`PgUp/PgDn` rolam, `r` recarrega e `Esc` volta para o agente. Na aba
do agente, `Tab` e `F1`–`F9` chegam ao Claude, mas `Shift+Tab` não (troca de aba). O agente
continua rodando enquanto outra aba está aberta. Cada instância recebe um `--session-id` próprio
(a não ser que os argumentos já escolham a sessão, como `--continue`/`--resume`), para o consumo
saber qual transcript é dela.

Como os modos funcionam no Claude Code: os hooks do RTK e do AI Memory saem do
`~/.claude/settings.json` (backup em `settings.json.terraforme-bak`) e viram dois plugins em
`~/.config/terraforme/plugins/`, carregados por sessão com `--plugin-dir` conforme o modo. Isso é
refeito a cada `terraforme cli`, então se um `rtk init -g`/`ai-memory setup`/`ai-memory upgrade`
recolocar os hooks no global, eles são movidos de novo. O servidor MCP global do AI Memory
(`~/.claude.json`) não é alterado: é bloqueado na sessão e o plugin traz uma cópia quando o modo o usa.
Consequência: `claude` aberto direto, sem o terraforme, roda sem os hooks do RTK e do AI Memory.

Com o AI Jail ligado, o comando vira `ai-jail --no-save-config … -- claude …`: o agente só grava na
pasta do projeto (e no `~/.claude`). O terraforme passa a cada execução o que as extensões do modo
precisam enxergar lá dentro — os plugins e os binários somente leitura, `~/.local/share/rtk` e
`~/.local/share/ai-memory` graváveis, e a porta do servidor local do AI Memory (`--forward-port`,
lida dos hooks e do `.mcp.json` do plugin). O resto da configuração (rede, SSH, Docker…) fica no
`~/.ai-jail`.

### Spaces

Pastas de trabalho salvas com um nome (em `~/.config/terraforme/spaces.toml`):

```bash
terraforme space --save              # salva a pasta atual; pergunta o nome (padrão: nome da pasta)
terraforme space --save api          # salva a pasta atual como "api"
terraforme space                     # lista os spaces e pergunta qual abrir (número ou nome)
terraforme space api                 # abre direto o space "api"
terraforme space --list              # só lista
terraforme space api --delete        # remove
```

Abrir um space inicia um shell novo (`$SHELL`; `%COMSPEC%` no Windows) na pasta dele, com
`TERRAFORME_SPACE` definido — `exit` volta para onde você estava. Um programa não consegue mudar a
pasta do shell que o chamou; para ir com `cd` no próprio shell, use `--path`, que só imprime a pasta
(a lista e a pergunta vão para o stderr):

```bash
# ~/.bashrc ou ~/.zshrc
ts() { local dir; dir=$(terraforme space --path "$@") && cd "$dir"; }
```

Desenvolvimento:

```bash
cargo run -p terraforme-cli -- --help
cargo run -p terraforme-cli -- greet Augusto
pnpm cli greet Augusto                     # atalho pela raiz
cargo run -p terraforme-cli -- completions bash > terraforme.bash

cargo build -p terraforme-cli --release   # target/release/terraforme(.exe)
cargo build -p terraforme-cli --profile dist   # target/dist/terraforme — fat LTO, para distribuir
```

## Qualidade

```bash
cargo fmt --all
cargo clippy --workspace -- -D warnings
cargo test --workspace
```

## Versões

Toda alteração entregue sobe a versão em `Cargo.toml` (`[workspace.package] version`, SemVer) e
ganha uma seção no [`CHANGELOG.md`](CHANGELOG.md). Depois de um `terraforme upgrade` /
`./install.linux.sh --update`, o script mostra as novidades das versões instaladas.

## Commits

Seguimos [Conventional Commits](https://www.conventionalcommits.org), **sempre em inglês**.
Os hooks do [Husky](https://typicode.github.io/husky) são ativados pelo `pnpm install` na raiz.

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

Hooks:

| Hook | O que roda |
|---|---|
| `pre-commit` | `cargo fmt --check` e `cargo clippy -D warnings` |
| `commit-msg` | `commitlint` (regras em `commitlint.config.mjs`) |
