# Arquitetura

## Estrutura do repositório

```
.
├── crates/
│   ├── terraforme-core/      # toda a lógica (compartilhada)
│   └── terraforme-cli/       # binário `terraforme`: clap (main.rs) e terminal com abas (term.rs)
├── docs/                     # esta documentação
├── install.linux.sh          # instala/atualiza/remove o binário em ~/.local/bin
└── rust-toolchain.toml       # Rust 1.98.0 + alvos Linux/Windows
```

Toda a lógica vai no `terraforme-core`; a CLI é apenas uma "casca" que o chama. Com a feature
opcional `clap`, o core deriva `clap::ValueEnum` nos enums públicos (`Extension`, `Provider`,
`Mode`), que a CLI usa direto como argumentos.

## Módulos do `terraforme-core`

| Módulo | Responsabilidade |
| --- | --- |
| `status` | `Component`/`State` (instalado, conflito, não instalado), busca no `PATH` e leitura de `--version`. |
| `extension` | Instala, atualiza e remove RTK e AI Memory pelos scripts oficiais de cada projeto; o AI Jail vem do tarball do release, conferido pelo `.sha256`. Define os relatórios (`rtk gain`, `ai-memory status`…), rodados com `NO_COLOR` e devolvidos sem ANSI. |
| `agent` | `Provider` e `Mode` (`eco` = RTK, `high` = AI Memory, `hybrid` = os dois), o trait `Adapter` e `agent::prepare()`, que monta o comando do agente igual para todos. Cada agente é um adapter (`agent/claude.rs` para o Claude Code). |
| `jail` | Integração com o AI Jail: configuração padrão em `~/.ai-jail` e `wrap()`, que envolve o comando do agente. |
| `usage` | Consumo de tokens lido dos transcripts do Claude Code. |
| `limits` | Quanto resta dos limites do plano (API OAuth) ou do orçamento configurado. |
| `config` | `Config { provider, mode, jail, budget }` em `~/.config/terraforme/config.toml` (`%APPDATA%\terraforme` no Windows; respeita `XDG_CONFIG_HOME`). |
| `space` | Pastas de trabalho com nome, em `<config>/spaces.toml`. |

## Agentes (adapters)

Tudo o que depende do agente de IA fica atrás do trait `agent::Adapter`, e o resto do terraforme
(o `prepare`, o terminal com abas, os limites, a remoção de extensões) só fala com ele:

| Método | Para quê |
| --- | --- |
| `bin` | executável do agente no `PATH` |
| `setup(mode)` | prepara a configuração do agente para as extensões do modo e devolve os argumentos que as ligam, as pastas que o AI Jail deve montar e avisos |
| `session_args` / `picks_session` | fixar um id de sessão por instância (para o consumo) e saber quando os argumentos do usuário já escolhem a sessão |
| `usage` | consumo de tokens (aba Consumo) |
| `plan_limits` | limites do plano do usuário; sem eles, vale o orçamento do config.toml |
| `forget(ext)` | desfaz o que o `setup` criou quando a extensão é removida |

Só `bin` e `setup` são obrigatórios; os outros têm um padrão "não suportado". `Provider::adapter()`
devolve o adapter de cada provider, ou `None` se ele ainda não foi implementado (é o caso do Cursor).

Para trocar ou adicionar um agente:

1. crie `crates/terraforme-core/src/agent/<agente>.rs` com um tipo que implemente `Adapter`;
2. adicione a variante em `Provider` (com `id` e `name`) e o adapter em `Provider::adapter`;
3. `terraforme config --provider <agente>` passa a usá-lo.

## Como os modos funcionam no Claude Code

`Claude::setup` (em `agent/claude.rs`) mexe na configuração global do Claude a cada `terraforme cli`:

1. Os hooks do RTK e do AI Memory saem do `~/.claude/settings.json` (backup em
   `settings.json.terraforme-bak`) e viram plugins locais em
   `~/.config/terraforme/plugins/terraforme-<ext>/hooks/hooks.json`. O arquivo global só é
   regravado depois que os plugins foram escritos. Como isso se repete a cada execução, se um
   `rtk init -g`, `ai-memory setup` ou `ai-memory upgrade` recolocar os hooks no global, eles são
   movidos de novo.
2. Para cada extensão do modo, o `claude` recebe `--plugin-dir <plugin>`.
3. O servidor MCP global do AI Memory (`~/.claude.json`) não é alterado: é bloqueado na sessão com
   `--settings '{"deniedMcpServers":[{"serverName":"ai-memory"}]}'`, e o plugin do AI Memory traz
   uma cópia dele no `.mcp.json` (servidor `memory`) quando o modo o usa.
4. Os argumentos depois de `--` vão direto para o `claude`.
5. Com o AI Jail ligado (`config.jail`, ou `--jail`/`--no-jail`), o comando todo é envolvido por
   `jail::wrap`.

Consequência: `claude` aberto direto, sem o terraforme, roda sem os hooks do RTK e do AI Memory.

## AI Jail

Com o AI Jail ligado, o comando vira `ai-jail --no-save-config <mapeamentos> -- claude <args>` e o
agente só grava na pasta do projeto (e no `~/.claude`). Os mapeamentos são passados a cada
execução, nunca salvos:

- os plugins e binários das extensões do modo (e a pasta real dos symlinks), somente leitura;
- `~/.local/share/rtk` e `~/.local/share/ai-memory`, graváveis;
- `--forward-port` para cada porta `127.0.0.1:`/`localhost:` encontrada no `hooks.json`/`.mcp.json`
  do plugin (o servidor local do AI Memory).

O resto (rede, SSH, Docker…) fica no `~/.ai-jail`, criado com uma configuração padrão só quando não
existe:

```toml
no_save_config = true     # não grava .ai-jail em cada projeto
no_status_bar = true      # o terminal do terraforme já tem as abas

[commands.claude]
agent_state = true        # ~/.claude e ~/.claude.json montados: sessão já logada
```

Exige ai-jail 2.9+ (`--forward-port`) e, no Linux, o `bwrap` (pacote bubblewrap). O
`--no-status-bar` nunca é passado na linha de comando, porque o ai-jail o grava no `~/.ai-jail`.

## Consumo e limites

- **`usage`** lê `~/.claude/projects/<cwd com não alfanuméricos → '-'>/<sessão>.jsonl` (respeita
  `CLAUDE_CONFIG_DIR`). Uma resposta ocupa várias linhas, então as entradas são deduplicadas por
  `message.id` + `requestId`. Os transcripts dos subagentes ficam em
  `<sessão>/subagents/agent-<id>.jsonl`, com tipo e descrição no `.meta.json` ao lado.
- **`limits`** consulta `api.anthropic.com/api/oauth/usage` com o token OAuth de
  `<claude>/.credentials.json` (nunca renovado pelo terraforme; cache de 60s). Sem login, compara os
  totais dos transcripts com o `[budget]` da configuração.

## Terminal do `terraforme cli` (`term.rs`)

TUI em ratatui com barra de abas: a aba do agente (`claude`), cujas sub-abas são as instâncias,
`Consumo` (Limites / Tokens / Subagentes, atualizada a cada 5s) e, quando o modo usa extensões, uma
aba por relatório de cada extensão.

- Cada instância roda no seu próprio pseudoterminal (`portable-pty`), redimensionado para a tela
  real antes de cada desenho (`App::fit`), e é renderizada com `vt100` + `tui-term` (5000 linhas de
  histórico). Continua rodando enquanto outra aba está aberta.
- Cada instância recebe um `--session-id` próprio, a não ser que os argumentos já escolham a sessão
  (`--continue`/`--resume`/`--session-id`, ver `agent::picks_session`), para o consumo saber qual
  transcript é dela.
- As abas que não são do agente carregam o conteúdo numa thread em segundo plano.
- As teclas viram sequências xterm em `key_bytes` (respeitando o modo de cursor de aplicação).
  `Shift+Tab` sempre troca de aba e nunca chega ao agente.
- Avisos do `prepare()` aparecem num popup inicial, porque a tela alternativa esconderia o que foi
  impresso antes dela.

## Spaces

Um programa não consegue mudar a pasta do shell que o chamou. Por isso abrir um space faz `exec` de
um shell novo (`$SHELL`; `%COMSPEC%` no Windows) na pasta, com `TERRAFORME_SPACE` definido. Os
spaces ficam fora do `config.toml` porque `Config` é `Copy`.
