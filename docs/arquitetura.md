# Arquitetura

Como o código do terraforme está organizado: os 4 crates, os módulos do core e as partes não
óbvias (modos no Claude Code, AI Jail, consumo, daemon MCP, TUI). Para quem vai ler ou mudar o
código; o uso está no [Guia](guia.md) e o catálogo em [Funcionalidades](funcionalidades.md).

## Estrutura do repositório

```
.
├── crates/
│   ├── terraforme-core/      # toda a lógica (biblioteca)
│   ├── terraforme-cli/       # binário `terraforme`: clap e a TUI
│   └── terraforme-mcp/       # binário `terraforme-mcp`: daemon compartilhado entre os terminais
├── skills/                   # skills de agente embutidas no binário (build.rs do core)
├── assets/                   # sprites do mascote e extensão do GNOME Shell
├── docs/                     # esta documentação
├── install.linux.sh / install.windows.ps1   # instalação a partir do código
├── install.sh / install.ps1                 # instalação a partir das releases
└── rust-toolchain.toml       # Rust 1.98.0
```

## Crates

| Crate | Binário | Papel |
| --- | --- | --- |
| `terraforme-core` | — | Toda a lógica. Com a feature `clap`, deriva `clap::ValueEnum` em `Extension`, `Provider`, `Mode` e `CliMode`, usados direto como argumentos. |
| `terraforme-cli` | `terraforme` | Casca em volta do core: `main.rs` (clap e saída), `term.rs` (TUI), `chat.rs` + `markdown.rs` (chat do Middleware), `select.rs` (seleção múltipla no terminal). |
| `terraforme-mcp` | `terraforme-mcp` | Daemon axum em `127.0.0.1` (JSON-RPC MCP sobre HTTP): `hub.rs` (instâncias e mensagens), `lifecycle.rs` (clientes, heartbeat, encerramento), `tools/` (ferramentas MCP), `ui.rs` + `ui.html` (UI web), `pet.rs` (endpoints da janela). |

A janela flutuante (`terraforme-pet`) é um plugin com repositório próprio,
[Augustowtg/terraforme-pet](https://github.com/Augustowtg/terraforme-pet), que depende do
`terraforme-core` por git; ver o módulo `plugin`.

## Módulos do `terraforme-core`

| Módulo | Responsabilidade |
| --- | --- |
| `status` | `Component`/`State` (instalado, conflito, não instalado), busca no `PATH` (no Windows segue o `PATHEXT`) e leitura de `--version`. Use sempre `status::command(bin)` com o caminho completo. |
| `platform` | O que muda entre sistemas e o único uso de `windows-sys`: `is_pid_alive`, `ignore_ctrl_c`, `kill_all`, `detach`, `open_url`, `raise_terminal`. |
| `extension` | Instala/atualiza/remove RTK e AI Memory pelos scripts oficiais e o AI Jail pelo tarball do release (`.sha256` conferido). Define os `Report`s (`rtk gain`, `ai-memory status`…), rodados com `NO_COLOR` e devolvidos sem ANSI. |
| `agent` | `Provider`, `Mode`, `CliMode`, o trait `Adapter` e `prepare()`, que monta o comando do agente igual para todos. Um submódulo por agente: `claude`, `opencode`, `antigravity`. |
| `jail` | AI Jail: `default_config()` para o `~/.ai-jail`, `wrap()` (envolve o comando do agente) e `ensure_project()`. |
| `usage` | Consumo de tokens do Claude Code pelos transcripts (o OpenCode e o Antigravity têm o seu no adapter). |
| `limits` | Converte o `Quota` de cada adapter em `Limit`s (porcentagem, tokens, quando zera); sem `Quota`, usa o `[budget]`. Cache de 60s. |
| `history` | SQLite em `<root>/usage.db`: consumo por space × agente (`History::totals`) e aberturas do `terraforme cli` (`record_launch`). |
| `config` | `Config { provider, mode, jail, budget, mcp, tui }` em `config.toml`. |
| `accounts` | Contas do Claude Code (nome → `CLAUDE_CONFIG_DIR`) em `<config>/claude.toml`. |
| `space` | `Spaces` em `<config>/spaces.toml`, `Preferences` por space e `space::root` (`~/.terraforme`). |
| `skill` | Skills embutidas (pasta `skills/`, via `build.rs`) e instalação em `Adapter::skills_dir`. |
| `mcp` | Lado cliente do daemon: `McpConfig` (`[mcp]`), lock file e `ensure_running`. |
| `plugin` | Plugins de outros repositórios: `Manifest` (`terraforme-plugin.toml`), registro `KNOWN` dos oficiais, `plugins.toml` dos instalados e a fábrica `sources()` das formas de instalar (`Release`: `<bin>-<target>.tar.gz` + `.sha256` da última GitHub Release; `Cargo`: `cargo install --git`), tentadas em ordem por `install`. |
| `pet` | `Activity` e `Tracker` (estado de cada instância), cliente HTTP do `terraforme-pet`, `notify` e lock/start/stop da janela. |
| `chat` | Middleware: `ChatEvent` (saída estruturada comum a todos os agentes) e os protocolos de cada um — o `stream-json` do Claude Code com os pedidos de controle do SDK (`initialize`, `can_use_tool`), o `stream-json` do Antigravity (`step_update`, `result`) e o Agent Client Protocol do OpenCode (`opencode acp`). |

## Agentes (adapters)

Tudo o que depende do agente fica atrás de `agent::Adapter`; o resto (o `prepare`, a TUI, os
limites, a remoção de extensões) só fala com ele.

| Método | Para quê |
| --- | --- |
| `bin` | executável do agente |
| `setup(mode)` | prepara as extensões do modo; devolve argumentos, pastas para o AI Jail, variáveis de ambiente e avisos |
| `session_args` / `picks_session` | fixa um id de sessão por instância / detecta quando os argumentos já escolhem a sessão |
| `usage` | consumo de tokens (aba Consumo) |
| `quota` | limites do plano no formato `Quota` |
| `forget(ext)` | desfaz o que o `setup` criou quando a extensão é removida |
| `skills_dir` | pasta global de skills (usada pelo `terraforme setup`) |
| `mcp_args` | liga o daemon MCP na instância (só Claude) |
| `chat_args`, `chat_drop_args`, `chat_waits_session`, `chat_input`, `chat_events`, `chat_start`, `chat_permission` | Middleware (Claude, Antigravity e OpenCode) |

Só `bin` e `setup` são obrigatórios; os outros têm um padrão "não suportado". `Provider::adapter()`
devolve `None` para agentes não implementados (Cursor).

| Agente | Módulo | Observações |
| --- | --- | --- |
| Claude Code | `agent/claude.rs` | plugins locais por extensão, transcripts, limites OAuth, MCP, Middleware |
| OpenCode | `agent/opencode.rs` | v2+; sessão por `OPENCODE_CONFIG_CONTENT`, sempre `--standalone` (menos no Middleware, em que o `acp` abre a própria sessão); consumo por `opencode stats --json` |
| Antigravity | `agent/antigravity.rs` | binário `agy`; sem extensões; transcripts em `~/.gemini/antigravity-cli/brain`, limites por `agy -p "/usage" --output-format json` |

Para adicionar um agente:

1. crie `crates/terraforme-core/src/agent/<agente>.rs` com um tipo que implemente `Adapter`;
2. adicione a variante em `Provider` (`id`, `name`, `url`) e o adapter em `Provider::adapter`;
3. `terraforme config --agent <agente>` passa a usá-lo.

## Como os modos funcionam no Claude Code

`Claude::setup` mexe na configuração global do Claude a cada `terraforme cli`:

1. Os hooks do RTK e do AI Memory saem do `~/.claude/settings.json` (backup em
   `settings.json.terraforme-bak`) e viram plugins locais em
   `~/.config/terraforme/plugins/terraforme-<ext>/hooks/hooks.json`. O global só é regravado
   depois que os plugins foram escritos; se `rtk init -g`/`ai-memory setup` recolocar os hooks,
   eles são movidos de novo.
2. Cada extensão do modo vira um `--plugin-dir <plugin>`.
3. O MCP global do AI Memory (`~/.claude.json`) não é alterado: é bloqueado na sessão com
   `--settings '{"deniedMcpServers":[{"serverName":"ai-memory"}]}'`, e o plugin do AI Memory traz
   uma cópia dele no `.mcp.json` (servidor `memory`) quando o modo o usa.
4. Os argumentos depois de `--` vão direto para o `claude`.
5. Com AI Jail, o comando todo é envolvido por `jail::wrap`.

Consequência: `claude` aberto direto, sem o terraforme, roda sem os hooks do RTK e do AI Memory.

## AI Jail

O comando vira `ai-jail --no-save-config <mapeamentos> -- <agente> <args>`. Os mapeamentos são
passados a cada execução, nunca salvos:

- plugins e binários das extensões do modo (e a pasta real dos symlinks), somente leitura;
- `~/.local/share/rtk` e `~/.local/share/ai-memory`, graváveis;
- `--forward-port` para cada porta `127.0.0.1:`/`localhost:` no `hooks.json`/`.mcp.json` do plugin
  e para a porta do daemon MCP (por isso `run_term` sobe o daemon antes de montar o comando).

O resto fica no `~/.ai-jail`, criado só se não existir:

```toml
no_save_config = true     # não grava .ai-jail em cada projeto
no_status_bar = true      # o terminal do terraforme já tem as abas

[commands.claude]
agent_state = true        # ~/.claude e ~/.claude.json montados: sessão já logada

[commands.opencode]
agent_state = true
```

- Exige ai-jail 2.9+ (`--forward-port`) e `bwrap` no Linux.
- `--no-status-bar` nunca vai na linha de comando: o ai-jail o grava no `~/.ai-jail`.
- Não existe AI Jail no Windows (`jail::is_supported`): o jail implícito (config, space,
  `.ai-jail`) é ignorado com aviso; só `--jail` explícito dá erro.
- `ensure_project()` (cria o `.ai-jail` do projeto) só roda por `terraforme extension ai-jail --project`.

## Consumo, limites e histórico

- **`usage`** (Claude Code) — `~/.claude/projects/<cwd com não alfanuméricos → '-'>/<sessão>.jsonl`
  (respeita `CLAUDE_CONFIG_DIR`). Uma resposta ocupa várias linhas: deduplica por `message.id` +
  `requestId`. Subagentes em `<sessão>/subagents/agent-<id>.jsonl`, com tipo/descrição no
  `.meta.json`.
- **OpenCode** — `opencode stats --json`: projeto/dia e global/dia/semana; sem janela de 5h, total
  da sessão nem detalhe por subagente.
- **Antigravity** — transcripts em
  `~/.gemini/antigravity-cli/brain/<conversa>/.system_generated/logs/transcript.jsonl`, conversa
  atual pelo `cache/last_conversations.json`.
- **`limits`** — Claude: `api.anthropic.com/api/oauth/usage` com o token de
  `<claude>/.credentials.json` (nunca renovado). Sem `Quota`, compara os totais com o `[budget]`.
- **`history`** — uma linha por sessão (`key` = `<agente>:<sessão>`), totais substituídos a cada
  gravação. A TUI grava a cada 60s (`RECORD_EVERY`), ao fechar/reiniciar e ao sair
  (`App::record_usage`). A tabela `launches` guarda as aberturas para o `--historico`.

## Daemon MCP (`terraforme-mcp`)

- Sobe sob demanda (`mcp::ensure_running`, lock em `<data>/terraforme/mcp.lock`) e encerra depois
  que o último cliente sai. `LockFile::claim` impede dois daemons; `LockFile::release` só apaga o
  próprio lock.
- Cada TUI se registra (`/register` + `/heartbeat`, com o endereço do seu servidor de controle) e
  registra cada instância (`/instances/register`). Se o heartbeat recebe 404 ou o daemon sumiu, a
  TUI chama `ensure_running` e registra tudo de novo.
- As ferramentas chegam à TUI por `POST /message`, `/open`, `/close` e `/spawn` no servidor de
  controle dela; os prompts são entregues por `Agent::deliver_pending` (colados quando a saída
  fica quieta, Enter 300ms depois).
- Só o Claude recebe o MCP (`Adapter::mcp_args`: `--mcp-config` com os cabeçalhos
  `x-client-id`/`x-instance-id`).

Detalhes das ferramentas: [MCP/mcp.feature.md](MCP/mcp.feature.md); UI web:
[tui/webserver/webserver.feature.md](tui/webserver/webserver.feature.md).

## Terminal do `terraforme cli` (`term.rs`)

- **Abas pelo padrão Registro** — cada aba é uma entrada `TabSpec` em `TABS` (ordem, nome, atalho
  `Ctrl+Shift+F<n>`, sub-abas e `TabKind`: `Agent`/`Content`/`Form`); menu, atalhos e ajuda leem
  essa lista. Adicionar uma aba = adicionar uma entrada.
- **Instâncias** — cada uma num `portable-pty` (ConPTY no Windows), redimensionado antes de cada
  desenho (`App::fit`), renderizado com `vt100` + `tui-term` (5000 linhas de histórico). Cada
  instância recebe um id de sessão próprio, a não ser que os argumentos já escolham a sessão.
- **Teclado** — `key_bytes` traduz as teclas para sequências xterm; o protocolo do kitty é ligado
  quando o terminal suporta, para `Ctrl+Shift`/`Ctrl+Alt` chegarem distintos.
- **Mouse** — captura em `[tui] mouse`; arrastar faz uma `Selection` copiada por OSC 52 e
  `wl-copy`/`xclip`/`xsel`.
- **Estado para a janela** — cada `Agent` tem um `pet::Tracker` alimentado pela saída e pelas
  teclas, e reporta mudanças ao daemon (`/instances/state`).
- **Middleware** — com `[tui] middleware`, agentes com `chat_args` rodam por pipes em vez de PTY e a
  instância ganha um `Chat` (`chat.rs`).
- **Abas de conteúdo** carregam numa thread em segundo plano; avisos do `prepare()` aparecem num
  popup inicial (a tela alternativa esconderia o que foi impresso antes).

Detalhes de comportamento: [tui/tui.feature.md](tui/tui.feature.md) e
[tui/engine/engine.feature.md](tui/engine/engine.feature.md).

## Janela flutuante (`terraforme-pet`)

Consulta `pet::instances()` (`GET /pet/state` do daemon) a cada segundo, desenha o mascote dos
sprites em `assets/*_strip.png` (embutidos com `include_bytes!`) e, num clique, chama `pet::focus`
(`POST /pet/focus` → `POST /focus` na TUI dona, que chama `platform::raise_terminal`). Detalhes:
[janela-flutuante.md](janela-flutuante.md).

## Spaces

- Um programa não muda a pasta do shell que o chamou: abrir um space faz `exec` de um shell novo
  (`$SHELL`; `%COMSPEC%` no Windows) com `TERRAFORME_SPACE`. `--path` só imprime a pasta.
- Os spaces ficam fora do `config.toml` porque `Config` é `Copy`.
- `Preferences` (`agent`, `mode`, `jail`, `account`) ficam em `<root>/<space>/preferences.toml`
  (`space::root`: `~/.terraforme`, `%APPDATA%\.terraforme` no Windows, `TERRAFORME_HOME`), e
  `Spaces::load` migra o `<pasta>/.terraforme/preferences.toml` antigo.
- `Choice::resolve` (na CLI) acha o space mais específico que contém a pasta
  (`Preferences::find`) e resolve: linha de comando → preferências do space → `.ai-jail` da pasta
  (só `jail`) → `config.toml`. Também grava a abertura no `history`.
