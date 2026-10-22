# Funcionalidades do terraforme

Catálogo de tudo o que o terraforme faz hoje, com um resumo por assunto e links para o documento
que detalha cada um. Para o passo a passo de uso, veja o [Guia](guia.md); para o código, a
[Arquitetura](arquitetura.md).

## Sumário

1. [Visão geral](#1-visão-geral)
2. [Comandos da CLI](#2-comandos-da-cli)
3. [Extensões](#3-extensões-terraforme-extension)
4. [Agentes](#4-agentes)
5. [Modos e presets de início](#5-modos-e-presets-de-início)
6. [Terminal do `terraforme cli`](#6-terminal-do-terraforme-cli-tui)
7. [Configuração e contas](#7-configuração-e-contas-terraforme-config)
8. [Spaces](#8-spaces-terraforme-space)
9. [Histórico](#9-histórico)
10. [Conversa entre instâncias (MCP) e UI web](#10-conversa-entre-instâncias-mcp-e-ui-web)
11. [Janela flutuante](#11-janela-flutuante-terraforme-pet)
12. [Atualizar e reiniciar](#12-atualizar-e-reiniciar)
13. [Segurança](#13-segurança)

## 1. Visão geral

O `terraforme` é uma CLI (Linux e Windows) que prepara o agente de IA com:

- **Economia de tokens** — [RTK](https://github.com/rtk-ai/rtk) (modo `eco`).
- **Memória entre sessões** — [AI Memory](https://github.com/akitaonrails/ai-memory) (modo `high`).
- **Sandbox** — [AI Jail](https://github.com/akitaonrails/ai-jail) (`--jail`), ortogonal ao modo; só no Linux.
- **Consumo visível** — limites do plano, tokens e subagentes na aba Consumo, mais o histórico por space.
- **Várias instâncias** — até 9 agentes por terminal, que conversam entre si pelo daemon MCP.
- **Spaces** — pastas de trabalho com nome e preferências próprias.
- **Janela flutuante** — mascote que avisa quando um agente termina ou pede atenção (Linux).

## 2. Comandos da CLI

| Comando | O que faz |
| --- | --- |
| `terraforme` / `-h` / `--help` | Ajuda (`-h` resume, `--help` detalha) |
| `terraforme help <comando>` | Ajuda de um comando (ex.: `terraforme help space`) |
| `terraforme -V` / `--version` / `version` | Versão |
| `terraforme --historico` / `--history` | Pastas em que o `terraforme cli` foi aberto ([§9](#9-histórico)) |
| `terraforme status` | Extensões instaladas e versões, mais as pastas do terraforme (local, configuração e dados) |
| `terraforme setup` | Configura a máquina do zero: agentes, extensões e skills ([§3](#3-extensões-terraforme-extension)) |
| `terraforme extension …` | Instala, atualiza, reconfigura, remove ou mostra as extensões |
| `terraforme config …` | Mostra ou altera a configuração padrão ([§7](#7-configuração-e-contas-terraforme-config)) |
| `terraforme cli …` | Abre o agente no terminal do terraforme ([§6](#6-terminal-do-terraforme-cli-tui)) |
| `terraforme space …` | Pastas de trabalho com nome ([§8](#8-spaces-terraforme-space)) |
| `terraforme pet` | Abre a janela flutuante ([§11](#11-janela-flutuante-terraforme-pet)) |
| `terraforme upgrade …` | Atualiza o próprio terraforme ([§12](#12-atualizar-e-reiniciar)) |
| `terraforme restart` | Reinicia o daemon MCP (e órfãos) e a janela flutuante com os binários instalados |
| `terraforme completions <shell>` | Autocompletar para `bash`, `zsh`, `fish`, `powershell`, `elvish` |

`-v`/`--verbose` vale em qualquer comando.

## 3. Extensões (`terraforme extension`)

| Extensão | Para quê | Instalação |
| --- | --- | --- |
| **RTK** | Reescreve a saída de comandos do shell para gastar menos tokens | Script oficial, em `~/.local/bin` |
| **AI Memory** | Memória de longo prazo entre sessões (wiki, decisões, regras, handoffs) | Script oficial, em `~/.local/bin` |
| **AI Jail** | Sandbox do sistema: o agente só grava na pasta do projeto | Tarball do release (Linux x86_64 / macOS arm64), conferido pelo `.sha256`, em `~/.local/bin` |

```bash
terraforme extension --status                 # status e versão de todas
terraforme extension --install                # instala todas (pula as instaladas)
terraforme extension ai-memory --install      # só uma
terraforme extension --setup                  # refaz a configuração nos agentes (pergunta quais)
terraforme extension rtk --update             # atualiza (nome obrigatório)
terraforme extension rtk --delete [-y]        # remove a extensão e a configuração nos agentes
terraforme extension ai-jail --project        # cria o .ai-jail na pasta atual
terraforme extension ai-jail --default true   # AI Jail ligado por padrão no terraforme cli
```

`terraforme setup` faz tudo de uma vez, em três passos:

1. **Agentes** — lista os agentes suportados no `PATH`; sem nenhum, pergunta quais instalar.
2. **Extensões** — instala as que faltam e aplica a configuração em cada agente encontrado (o AI
   Jail é configurado uma vez para a máquina).
3. **Skills** — instala as skills embutidas no binário (pasta `skills/` do repositório, ex.:
   `terraform-requisitos`) na pasta global de skills de cada agente.

Detalhes por extensão:

- **RTK** — no Claude Code, `rtk init -g` (os hooks são movidos para plugins do terraforme); no
  OpenCode, um plugin próprio que chama `rtk rewrite`. Relatórios de `rtk gain` na aba Consumo. No
  Windows, a instalação automática não é suportada.
- **AI Memory** — `ai-memory setup` (Claude) ou `ai-memory install-hooks --agent opencode2 --apply`
  (OpenCode). Relatórios: Status, Projetos, Handoffs. Os dados são preservados na remoção.
- **AI Jail** — exige ai-jail 2.9+ e `bwrap` no Linux; não existe no Windows. Cria `~/.ai-jail`
  só se não existir. Update/delete só mexem na cópia instalada pelo terraforme.

Setup passo a passo: [setup.md](setup.md) e [setup-interativo.md](setup-interativo.md).

## 4. Agentes

| Agente | ID | Binário | Extensões | Consumo e limites | MCP / Middleware |
| --- | --- | --- | --- | --- | --- |
| Claude Code | `claude` | `claude` | RTK, AI Memory, AI Jail | transcripts + limites do plano (OAuth) | sim / sim |
| OpenCode | `opencode` | `opencode` (v2+) | RTK, AI Memory, AI Jail | `opencode stats --json`, só orçamento semanal | não / sim (`opencode acp`) |
| Antigravity | `antigravity` | `agy` | nenhuma | transcripts do `agy` + `agy -p "/usage"` | não / sim (`stream-json`) |
| Cursor | `cursor` | — | não implementado (oculto na ajuda) | — | — |

Cada agente é um `Adapter`; como ele liga as extensões e como adicionar um novo está na
[Arquitetura](arquitetura.md#agentes-adapters). O que falta para os outros agentes está no
[ROADMAP](../ROADMAP.md).

## 5. Modos e presets de início

| Modo | Extensões |
| --- | --- |
| `none` | nenhuma |
| `eco` | RTK |
| `high` | AI Memory |
| `hybrid` (padrão) | RTK + AI Memory |

O AI Jail não faz parte do modo: liga à parte (`--jail`/`--no-jail`, `config --jail`, `.ai-jail`
na pasta ou preferência do space).

Os **presets de início** (`terraforme config --cli`) escolhem modo e sandbox de uma vez:

| Preset | Liga |
| --- | --- |
| `eco` | RTK |
| `high` | AI Memory |
| `security` | AI Jail, sem extensões |
| `custom` | o que você escolher (RTK, AI Memory e/ou AI Jail) |

```bash
terraforme cli                         # padrão do config (ou do space)
terraforme cli --eco | --high | --hybrid
terraforme cli --jail | --no-jail
terraforme cli --agent opencode        # agente desta sessão (alias --agente)
terraforme cli -n 3                    # 3 instâncias (máx. 9)
terraforme cli --plain                 # agente direto, sem o terminal do terraforme
terraforme cli -- --continue           # depois de --, vai para o agente
```

Ordem de resolução: flags → preferências do space → `.ai-jail` da pasta (só a sandbox) →
`config.toml`.

## 6. Terminal do `terraforme cli` (TUI)

| Aba | Conteúdo |
| --- | --- |
| **Agente** (`claude`, `opencode`…) | uma sub-aba por instância; com o Middleware, se chama `Middleware` |
| **Consumo** | Limites, Tokens, Subagentes (atualizados a cada 5s) e Spaces (histórico por space × agente); com RTK no modo, os relatórios dele |
| **Extensões** | relatórios do AI Memory e do AI Jail; só aparece quando há algum |
| **Configurações** | formulário que grava no `config.toml` (ou nas preferências do space) a cada mudança |

Principais atalhos (`Ctrl+Alt+K` lista todos dentro da TUI):

| Tecla | Ação |
| --- | --- |
| `Ctrl+Shift+F1`…`F4` | abre Agente / Consumo / Extensões / Configurações |
| `Ctrl+Shift+←/→` | aba vizinha |
| `Ctrl+Alt+Tab`, `Ctrl+PgUp/PgDn` | sub-aba vizinha |
| `Ctrl+Alt+1`…`9` / `Alt+1`…`9` | vai para a sub-aba (no agente, além da última abre outra) |
| `Ctrl+Alt+N` / `F12` | nova instância |
| `Ctrl+Alt+W` / `Shift+F12` | fecha a instância |
| `Ctrl+Alt+R` | reinicia a instância |
| `Ctrl+Alt+C` | copia seleção, tela ou conteúdo da aba |
| `Ctrl+Alt+S` | liga/desliga a captura do mouse |
| `F10` | sai |

Também:

- **Seleção com o mouse** — arrastar no agente seleciona e copia ao soltar (`[tui] copy_on_select`).
- **Middleware (experimental)** — `[tui] middleware`: o agente roda sem a TUI dele e o terraforme
  desenha a conversa como chat (Markdown e código coloridos, `/` comandos, `@` arquivos, pedidos de
  permissão). Claude Code, Antigravity e OpenCode.

Detalhes: [tui/tui.feature.md](tui/tui.feature.md) (menu e atalhos),
[tui/agente/agente.feature.md](tui/agente/agente.feature.md) (Middleware),
[tui/consumo/consumo.feature.md](tui/consumo/consumo.feature.md),
[tui/config/config.feature.md](tui/config/config.feature.md) e
[tui/engine/engine.feature.md](tui/engine/engine.feature.md).

## 7. Configuração e contas (`terraforme config`)

Fica em `~/.config/terraforme/config.toml` (`%APPDATA%\terraforme\config.toml` no Windows;
respeita `XDG_CONFIG_HOME`):

```toml
provider = "claude"      # claude | opencode | antigravity
mode = "hybrid"          # none | eco | high | hybrid
jail = false

[budget]                 # usado quando o agente não informa os limites do plano
five_hours = 20000000
week = 300000000

[mcp]
ui = false                        # UI web do daemon
grace_seconds = 5                 # espera antes de encerrar quando o último terminal sai
subagent_timeout_seconds = 300    # spawn_subagent
max_simultaneous_subagents = 3

[tui]
mouse = true
copy_on_select = true
pet = false              # abre a janela flutuante com o terraforme cli
middleware = false       # experimental
```

```bash
terraforme config                              # mostra a configuração
terraforme config --agent [opencode]           # agente padrão (sem valor, pergunta)
terraforme config --cli [security]             # preset de início (sem valor, pergunta)
terraforme config --mode custom                # pergunta as extensões
terraforme config --jail true
terraforme config --budget-5h 20000000         # 0 remove
terraforme config --budget-week 300000000      # 0 remove
terraforme config --mcp-ui true
terraforme config --pet true
terraforme config --account                    # lista as contas do Claude Code
terraforme config --account work=$HOME/.claude-work
terraforme config --remove-account work
```

**Contas do Claude Code** — cada conta é um nome → pasta `CLAUDE_CONFIG_DIR`, registrada em
`~/.config/terraforme/claude.toml`; o space escolhe a conta com `--account`.

## 8. Spaces (`terraforme space`)

Pastas de trabalho com nome, em `<config>/spaces.toml`:

```bash
terraforme space --save [api]           # salva a pasta atual
terraforme space                        # lista e pergunta qual abrir
terraforme space api                    # abre um shell na pasta (TERRAFORME_SPACE definido)
terraforme space --list
terraforme space api --rename backend
terraforme space api --delete [-y]      # a pasta não é apagada
terraforme space api --path             # só imprime a pasta (para cd "$(…)")
```

**Preferências por space** (`--agent`, `--mode`, `--jail`, `--account`, `--reset`) ficam fora do
projeto, em `~/.terraforme/<space>/preferences.toml` (`%APPDATA%\.terraforme` no Windows;
`TERRAFORME_HOME` troca a raiz), e valem para o `terraforme cli` aberto na pasta ou em subpastas.
Na aba Configurações, dentro de um space, agente/modo/conta gravam nessas preferências.

## 9. Histórico

Banco SQLite em `~/.terraforme/usage.db`:

- **Consumo por space × agente** — cada instância grava o total da sessão a cada 60s, ao
  fechar/reiniciar e ao sair (OpenCode não informa o total da sessão e não entra). Somas de 24h,
  7 dias e total na sub-aba Consumo → Spaces e na UI web.
- **Aberturas** — `terraforme --historico` lista a abertura mais recente de cada space salvo e as
  pastas recentes fora de space, com quando, o agente e quantas vezes.

## 10. Conversa entre instâncias (MCP) e UI web

O `terraforme cli` sobe sob demanda o daemon `terraforme-mcp` (local, `127.0.0.1`), que se encerra
depois que o último terminal sai. Cada instância é registrada com um apelido
`<space ou pasta>/<agente>-<n>` (ex.: `api/claude-2`) e o Claude Code recebe o MCP `terraforme`:

| Ferramenta | O que faz |
| --- | --- |
| `list_instances` | instâncias abertas em todos os spaces |
| `send_message` | digita a mensagem como prompt na instância de destino |
| `open_instance` | abre uma instância nova no mesmo terminal (sem tirar o foco) |
| `delegate_task` | entrega uma tarefa a uma instância (nova ou aberta) e espera o resultado (até 30 min) |
| `close_instance` | encerra outra instância (pede confirmação; nunca a própria) |
| `spawn_subagent` | roda a tarefa com `claude --print` e devolve a saída na chamada |

A **UI web** (`terraforme config --mcp-ui true`, endereço em `terraforme config`) mostra os spaces
e agentes abertos com o estado de cada um, o histórico de mensagens, um formulário para mandar
mensagem e o consumo por space. Só aceita pedidos locais.

Detalhes: [MCP/mcp.feature.md](MCP/mcp.feature.md) e
[tui/webserver/webserver.feature.md](tui/webserver/webserver.feature.md).

## 11. Janela flutuante (`terraforme pet`)

Plugin `terraforme-pet` (só Linux, repositório
[Augustowtg/terraforme-pet](https://github.com/Augustowtg/terraforme-pet); instale com
`terraforme plugin install pet`):
janela sempre por cima com o mascote, cujo humor segue a instância mais urgente. Avisa (balão +
`notify-send`) quando um agente termina ou pede atenção, e um clique abre o menu de instâncias e
traz o terminal certo para a frente. Abre com `terraforme pet` ou junto com o `terraforme cli`
(`terraforme config --pet true`).

Detalhes: [janela-flutuante.md](janela-flutuante.md).

## 12. Atualizar e reiniciar

```bash
terraforme upgrade              # no repositório: git pull + compila + reinstala
terraforme upgrade --no-pull    # só recompila
terraforme upgrade --dist       # build com LTO (mais lenta)
terraforme upgrade --repo DIR   # outro repositório (ou TERRAFORME_REPO)
terraforme upgrade --restart    # reinicia o terraforme no fim, sem perguntar
terraforme restart
```

Sem o repositório (binário de release), o `upgrade` baixa a última release com o `install.sh` /
`install.ps1`. O `restart` encerra todos os `terraforme-mcp`, sobe o novo e reabre a janela
flutuante; os terminais abertos se registram sozinhos no daemon novo, mas os agentes só voltam a
usar o MCP depois de reiniciados (`Ctrl+Alt+R`). Terminais anteriores à 0.22.2 não se reconectam.

## 13. Segurança

- **AI Jail** — o agente só grava na pasta do projeto (e no estado do agente, ex.: `~/.claude`);
  os mapeamentos são passados a cada execução, nunca salvos.
- **Arquivos sensíveis** — JSONs de configuração são gravados de forma atômica com permissão
  `0600`.
- **OAuth** — o terraforme lê o token do Claude Code, mas nunca o renova.
- **Configuração global** — os hooks do RTK/AI Memory saem dos arquivos globais (com backup) para
  plugins do terraforme; remover uma extensão desfaz o que cada adapter criou.
- **Daemon MCP** — escuta só em `127.0.0.1`; a UI web e os endpoints da janela flutuante só
  aceitam pedidos locais (a UI confere `Host`/`Origin`).
