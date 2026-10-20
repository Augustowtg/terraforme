
# Funcionalidades do terraforme

> **Projeto:** terraforme — framework de desenvolvimento com agentes de IA  
> **Versão:** 0.9.0  
> **Mantenedor:** Augusto Wesley  
> **Idioma do código/usuário:** português (comments, strings, docs)  
> **Commits:** inglês (Conventional Commits)

## Resumo

O terraforme é uma CLI (binário `terraforme`) que prepara o seu agente de IA com:

1. **Economia de tokens** — via [RTK](https://github.com/rtk-ai/rtk) (modo `--eco`).
2. **Memória de longo prazo entre sessões** — via [AI Memory](https://github.com/akitaonrails/ai-memory) (modo `--high`).
3. **Sandbox de segurança** — via [AI Jail](https://github.com/akitaonrails/ai-jail) (flag `--jail`).
4. **Visibilidade do consumo** — abas de limites, tokens e subagentes no terminal.
5. **Produtividade** — até 9 instâncias do agente lado a lado e pastas de trabalho nomeadas (*spaces*).

Toda a lógica de negócio fica no crate `terraforme-core`; o crate `terraforme-cli` é apenas a casca (argumentos com clap + terminal TUI com ratatui).

---

## 1. Comandos da CLI

| Comando | O que faz |
| --- | --- |
| `terraforme` / `--help` | Mostra a ajuda |
| `terraforme --version` / `-V` / `version` | Exibe a versão do terraforme |
| `terraforme status` | Verifica o status e versão das extensões (RTK, AI Memory, AI Jail) |
| `terraforme setup` | Configura a máquina do zero: identifica os agentes, instala e configura as extensões neles e instala as skills embutidas |
| `terraforme extension …` | Instala, atualiza, configura, remove ou mostra o status das extensões |
| `terraforme config …` | Mostra ou altera a configuração (agente, modo de início, sandbox, orçamento de tokens) |
| `terraforme cli …` | Abre o agente de IA no terminal do terraforme (abas Consumo e Extensões) |
| `terraforme space …` | Gerencia pastas de trabalho salvas com um nome (*spaces*) |
| `terraforme upgrade …` | Atualiza o próprio terraforme (git pull + compila + reinstala) |
| `terraforme greet <nome>` | Exibe uma saudação |
| `terraforme completions <shell>` | Gera o script de autocompletar para bash, zsh, fish, powershell, elvish |

---

## 2. Extensões (`terraforme extension`)

São três extensões integradas, instaladas a partir dos projetos oficiais:

| Extensão | Projeto | Para quê |
| --- | --- | --- |
| **RTK** | rtk-ai/rtk | Reescreve saídas de comandos do shell, economizando até 90% dos tokens |
| **AI Memory** | akitaonrails/ai-memory | Memória de longo prazo do agente entre sessões (wiki, decisões, regras) |
| **AI Jail** | akitaonrails/ai-jail | Sandbox do sistema (bubblewrap/Landlock ou sandbox-exec); o agente só grava na pasta do projeto |

### Operações

```bash
terraforme extension --status              # status e versão de todas
terraforme extension rtk --status          # de uma só
terraforme extension --install             # instala todas (pula as já instaladas)
terraforme extension ai-memory --install   # instala uma só
terraforme extension --setup               # configuração interativa de agentes e extensões
terraforme extension ai-memory --setup     # refaz o setup individual da extensão
terraforme extension rtk --update          # atualiza (nome obrigatório)
terraforme extension rtk --delete          # remove (nome obrigatório; -y pula a confirmação)
```

### Setup completo (`terraforme setup`)

Configura a máquina do zero, em três passos, sem perguntas:

```bash
terraforme setup
```

1. **Agentes** — identifica no `PATH` os agentes suportados (Claude Code e OpenCode) e mostra quais
   estão instalados. Falha se nenhum for encontrado.
2. **Extensões** — instala as que faltarem (RTK, AI Memory, AI Jail) e aplica a configuração de
   cada uma em todos os agentes encontrados; o AI Jail é configurado uma vez para a máquina.
3. **Skills** — instala as skills embutidas no binário (ex.: `terraform-requisitos`) na pasta
   global de skills de cada agente (`~/.claude/skills`, `~/.config/opencode/skills`).

### Detalhes por extensão

**RTK**
- Instala via script oficial (`install.sh`) em `~/.local/bin`.
- Configura `rtk init -g` no Claude Code (hooks globais movidos para plugins do terraforme).
- No OpenCode 2, gera um plugin próprio (`RTK_PLUGIN`) que chama `rtk rewrite` em cada comando de shell.
- Relatórios disponíveis: Economia, Neste projeto, Por dia, Histórico (`rtk gain`).
- No Windows a instalação automática não é suportada; use a edição Apache-2.0 com `RTK_EDITION=oss`.

**AI Memory**
- Instala via script oficial em `~/.local/bin`.
- Configura `ai-memory setup` (Claude) ou `ai-memory install-hooks --agent opencode2 --apply` (OpenCode).
- Relatórios disponíveis: Status, Projetos, Handoffs.
- Os dados (wiki, banco) são preservados na remoção.
- Cria serviço de usuário systemd (`ai-memory.service`) no Linux; `uninstall` não o remove, mas o terraforme o para ao remover.

**AI Jail**
- Não tem script de instalação: o terraforme baixa o tarball do release mais recente (Linux x86_64 ou macOS arm64), confere o `.sha256` publicado e instala em `~/.local/bin`.
- No Linux exige `bwrap` (pacote `bubblewrap`) e ai-jail 2.9+ (usa `--forward-port`).
- Cria configuração padrão em `~/.ai-jail` se não existir (com seções de `claude` e `opencode`).
- Um `.ai-jail` na pasta do projeto liga a sandbox só nele.
- Update/delete só mexem na cópia instalada pelo terraforme (outros gerenciadores: Homebrew/AUR/cargo/Nix atualizam por conta própria).

---

## 3. Agentes (Providers)

| Provider | ID | Implementado? | Binário |
| --- | --- | --- | --- |
| Claude Code | `claude` | Sim | `claude` |
| OpenCode | `opencode` | Sim | `opencode` (v2+) |
| Cursor | `cursor` | Não | `cursor` |

### Adapter (`agent::Adapter`)

Tudo o que depende do agente fica atrás deste trait:

- `bin()` — executável no `PATH`.
- `setup(mode)` — prepara a configuração para as extensões do modo; devolve argumentos, pastas do AI Jail, variáveis de ambiente, avisos.
- `session_args(session)` — fixa um id de sessão por instância (para o consumo).
- `picks_session(args)` — detecta quando os argumentos do usuário já escolhem a sessão (ex.: `--continue`, `--session-id`).
- `usage(cwd, launched, session)` — consumo de tokens (aba Consumo).
- `quota()` — limites de uso no formato `Quota` (porcentagem já usada, tokens e quando a janela zera) — a API OAuth do Claude, o `agy -p /usage` do Antigravity; sem eles, vale o orçamento do `config.toml`.
- `forget(ext)` — desfaz o que `setup` criou ao remover uma extensão.

### Claude Code

- Hooks do RTK e AI Memory saem do `~/.claude/settings.json` (com backup) e viram plugins locais em `~/.config/terraforme/plugins/terraforme-<ext>/`.
- O servidor MCP global do AI Memory (`~/.claude.json`) não é alterado; é bloqueado na sessão (`deniedMcpServers`) e o plugin traz uma cópia no `.mcp.json`.
- Recebe `--plugin-dir <plugin>` por extensão do modo e `--settings '{"deniedMcpServers":[{"serverName":"ai-memory"}]}'`.
- Sessão: `--session-id`.

### OpenCode

- Exige OpenCode 2 ou mais novo.
- Plugins globais do RTK/AI Memory são movidos para plugins do terraforme (backup em `.terraforme-bak`); o MCP do AI Memory também sai do `opencode.json`.
- Configuração da sessão via `OPENCODE_CONFIG_CONTENT` (JSON, não mexe em arquivo).
- RTK: plugin próprio que chama `rtk rewrite` em cada comando de shell.
- AI Memory: plugin do AI Memory + ponte stdio `ai-memory mcp-bridge --no-session-id`.
- Sempre roda com `--standalone` (cada sessão sobe o próprio servidor).
- No AI Jail, lança um wrapper `bin/opencode` que desfaz `HTTP_PROXY` (a rede filtrada do ai-jail recusa a conexão do cliente com o próprio servidor).
- Consumo: `opencode stats --json` (tokens por modelo, custo, sessões/prompts/subagentes — sem janela de 5h nem sessão atual).

### Como adicionar um agente novo

1. Crie `crates/terraforme-core/src/agent/<agente>.rs` implementando `Adapter`.
2. Adicione a variante em `Provider` e o adapter em `Provider::adapter()`.
3. `terraforme config --provider <agente>` passa a usá-lo.

---

## 4. Modos de execução (`terraforme cli`)

Os modos selecionam quais extensões são ativadas na sessão:

| Modo | Extensões | Para quê |
| --- | --- | --- |
| `--eco` | RTK | Sessões longas gastando menos tokens |
| `--high` | AI Memory | Projetos em que o contexto acumulado importa |
| `--hybrid` (padrão) | RTK + AI Memory | O melhor dos dois |

O AI Jail não é um modo: liga/desliga à parte (`--jail` / `--no-jail`) e combina com qualquer modo e agente.

### Modos de início (`terraforme config --cli`)

Além do modo, o `terraforme config --cli` define o **modo de início** padrão — um atalho que já
escolhe o modo e a sandbox:

| Modo de início | O que liga |
| --- | --- |
| `eco` | RTK |
| `high` | AI Memory |
| `security` | AI Jail (sandbox), sem extensões |
| `custom` | você escolhe as extensões (RTK, AI Memory e/ou AI Jail) |

### Exemplos

```bash
terraforme cli                          # modo padrão (hybrid)
terraforme cli --eco                    # só RTK
terraforme cli --high                   # só AI Memory
terraforme cli --hybrid                 # RTK + AI Memory
terraforme cli --jail                   # dentro do AI Jail (sandbox)
terraforme cli --no-jail                # fora do sandbox
terraforme cli --plain                  # agente direto, sem terminal do terraform
terraforme cli -n 3                     # 3 instâncias lado a lado (máx. 9)
terraforme cli -- --continue            # argumentos depois de -- vão para o agente
terraforme cli --agent opencode --eco   # agente e modo desta sessão
```

---

## 5. Terminal do `terraforme cli` (TUI)

Interface em ratatui com barra de abas no topo:

| Aba | Conteúdo |
| --- | --- |
| **Agente** | O agente (claude/opencode), uma sub-aba por instância |
| **Consumo** | Limites, Tokens, Subagentes (atualiza a cada 5s) e, com o RTK no modo, os relatórios dele |
| **Extensões** | Relatórios do AI Memory e do AI Jail (se ligado), um por sub-aba; só aparece quando há algum |
| **Config** | Configuração geral (`config.toml`) editável: agente, modo, jail, orçamento, TUI e MCP |

### Atalhos

`Ctrl+Alt+K` lista todos dentro da TUI.

| Tecla | Ação |
| --- | --- |
| `Ctrl+Shift+F1` | abre a aba Agente |
| `Ctrl+Shift+F2` | abre a aba Consumo |
| `Ctrl+Shift+F3` | abre a aba Extensões (só quando o modo tem alguma além do RTK) |
| `Ctrl+Shift+F4` | abre a aba Config |
| `Ctrl+Alt+Tab` / `Ctrl+Alt+Shift+Tab` | próxima / anterior sub-aba |
| `Ctrl+Alt+1`…`9` / `Alt+1`…`9` | vai direto para a sub-aba; na aba do agente, além da última abre outra instância |
| `Ctrl+PgUp/PgDn` | sub-aba vizinha, em volta |
| `Ctrl+Alt+N` / `F12` | abre outra instância do agente |
| `Ctrl+Alt+W` / `Shift+F12` | fecha a instância aberta |
| `Ctrl+Alt+R` | reinicia a instância aberta |
| arrastar com o mouse | seleciona texto do agente e copia ao soltar |
| `Ctrl+Alt+C` | copia a seleção, a tela do agente ou o conteúdo da aba |
| `Ctrl+Alt+S` | liga/desliga a captura do mouse (seleção nativa do terminal) |
| `Shift+PgUp/PgDn` / `Shift+Home/End` | rola o histórico do agente / início e volta ao vivo |
| roda do mouse | rola o histórico do agente e as outras abas |
| `↑↓` / `PgUp/PgDn` · `r` · `Esc` | rola · recarrega · volta para o agente (abas de conteúdo) |
| `F10` | sai (encerra todas as instâncias) |

`Ctrl+Shift`/`Ctrl+Alt` + dígito precisam do protocolo de teclado do kitty, que a TUI liga quando o
terminal suporta; sem ele valem as alternativas (`Alt+1`…`9` e `Shift+Alt+1`…`9`) e `Shift+Tab`
segue para o agente.

### Aba Consumo

- **Limites:** quanto foi usado e quanto falta. Com login OAuth no Claude Code (Pro/Max/Team), usa os limites do plano (`api.anthropic.com/api/oauth/usage`, cache 60s); sem login, compara transcripts com o orçamento `[budget]` do `config.toml`. No OpenCode, não há limites de plano nem janela de 5h: valem o orçamento semanal e a contagem local. No Antigravity, os limites vêm do próprio agente (`agy -p /usage`): um grupo de modelos por janela semanal, com o quanto resta e quando a janela zera.
- **Tokens:** entrada, saída, raciocínio, cache (grava/le), total e respostas da sessão atual, projeto (24h) e todos os projetos (5h/24h/7 dias), por modelo.
- **Subagentes:** sessão atual dividida entre o agente principal e cada subagente (tipo e descrição do `.meta.json`).

---

## 6. Configuração (`terraforme config`)

Armazenada em `~/.config/terraforme/config.toml` (`%APPDATA%\terraforme\config.toml` no Windows; respeita `XDG_CONFIG_HOME`):

```toml
provider = "claude"     # claude | opencode | cursor
mode = "hybrid"         # none | eco | high | hybrid
jail = false            # AI Jail por padrão

[budget]
five_hours = 20000000   # orçamento tokens nas últimas 5h (sem login OAuth)
week = 300000000        # orçamento tokens nos últimos 7 dias
```

```bash
terraforme config                          # mostra provider, modo de início, sandbox, orçamento
terraforme config --provider claude        # define o agente padrão
terraforme config --cli                    # lista os modos de início e pergunta o padrão
terraforme config --cli custom             # escolhe as extensões do modo customizado
terraforme config --mode eco               # define o modo padrão
terraforme config --mode custom            # o mesmo que --cli custom: pergunta as extensões
terraforme config --jail true              # sandbox por padrão
terraforme config --budget-5h 20000000     # orçamento 5h (0 remove)
terraforme config --budget-week 300000000  # orçamento 7d (0 remove)
```

Um `.ai-jail` na pasta atual também pode ligar o AI Jail por projeto (vence o `config.toml` quando não há `--jail`/`--no-jail` explícito).

---

## 7. Spaces (pastas de trabalho nomeadas)

Salvas em `~/.config/terraforme/spaces.toml`:

```bash
terraforge space --save              # salva a pasta atual (pergunta o nome)
terraforge space --save api          # salva como "api"
terraforge space                     # lista e pergunta qual abrir
terraforge space api                 # abre direto o space "api"
terraforge space --list              # só lista
terraforge space api --delete        # remove
```

Abrir um space inicia um shell novo (`$SHELL` / `%COMSPEC%`) na pasta, com `TERRAFORME_SPACE` definido. `exit` volta para onde estava. Use `--path` para só imprimir a pasta (útil em aliases de shell, ex.: `ts() { dir=$(terraforge space --path "$@") && cd "$dir"; }`).

**Preferências por space** (`~/.terraforme/<nome do space>/preferences.toml`, fora do projeto): agente, modo, sandbox e conta usados pelo `terraforme cli` aberto na pasta ou numa subpasta. Ordem de prioridade: flags da linha de comando → preferências do space → `.ai-jail` da pasta (só a sandbox) → `config.toml`.

```bash
terraforme space api --agent opencode --mode eco --jail false
terraforme space api --reset          # apaga as preferências
```

Aba **Config** da TUI: dentro de um space, agente, modo e AI Jail gravam nas preferências do space (criando o `preferences.toml` se não existir) em vez do `config.toml` global — os valores exibidos são os efetivos da sessão e o rodapé diz onde cada campo vale; orçamento, TUI e MCP continuam no `config.toml`. `Delete` num campo tira o override do space e volta ao valor global.

### Conversa entre instâncias

Todas as instâncias abertas pelo `terraforme cli`, em qualquer space, se registram no daemon `terraforme-mcp` com um apelido `<space ou pasta>/<agente>-<n>` (ex.: `api/claude-2`). O Claude Code de cada instância recebe o MCP `terraforme` com as ferramentas:

- `list_instances` — instâncias abertas (apelido, agente, space, pasta; a própria vem com `you=true`);
- `send_message(to, text)` — a mensagem é digitada como prompt na instância de destino, com o apelido de quem mandou para a resposta (`to="web"` escreve só na UI).

Basta pedir a uma instância algo como "pergunte à `api/claude-1` qual é o formato do endpoint X". As duas ferramentas já vêm liberadas na sessão (não pedem confirmação).

**UI web** (desligada por padrão): instâncias agrupadas por space, histórico de mensagens e um formulário para mandar mensagem a qualquer instância.

```bash
terraforme config --mcp-ui true    # ou: aba Config → "UI web das instâncias"
terraforme config                  # mostra o endereço: http://127.0.0.1:<porta>/ui
```

A porta é a do daemon (muda quando ele reinicia). Ligar/desligar vale na hora, sem reiniciar o daemon.

---

## 8. Atualização do próprio terraforme

```bash
terraforge upgrade            # git pull + compila + reinstala (~/.local/bin)
terraforge upgrade --no-pull  # só recompila o código local
terraforge upgrade --dist     # build otimizada com LTO (mais lenta)
```

O perfil `release` (padrão) omite LTO para compilação mais rápida; o perfil `dist` usa LTO para distribuição otimizada.

---

## 9. Segurança

- **AI Jail:** o agente roda numa sandbox do sistema; só grava na pasta do projeto (e nas pastas de estado do agente, ex.: `~/.claude`).
- **Arquivos com segredos:** JSONs de configuração são gravados de forma atômica (arquivo `.tmp` + rename) com permissão `0o600` (só o usuário lê).
- **OAuth:** o terraforme nunca renova o token do Claude Code; se expirou, orienta o usuário a abrir o Claude para renová-lo.
- **Hooks globais:** ao remover uma extensão, o terraforme desfaz o que cada adapter criou nos agentes (plugins do Claude, etc.).

---

## 10. Estrutura do repositório

```
.
├── crates/
│   ├── terraforme-core/      # toda a lógica (compartilhada pela CLI)
│   └── terraforme-cli/       # binário terraforme: clap (main.rs) + TUI (term.rs)
├── docs/                     # esta documentação
├── install.linux.sh          # instala/atualiza/remove o binário em ~/.local/bin
├── install.windows.ps1       # o mesmo no Windows (%LOCALAPPDATA%\Programs\terraforme\bin)
├── rust-toolchain.toml       # Rust 1.98.0 + rustfmt + clippy
├── Cargo.toml                # workspace (version 0.9.0)
├── CHANGELOG.md              # changelog
├── README.md                 # README do projeto
├── CLAUDE.md                 # instruções para agentes
├── AGENTS.md                 # instruções do repositório
├── commitlint.config.mjs     # regras de commit (Conventional Commits, inglês)
├── package.json / pnpm-lock.yaml  # só para hooks do git (Husky)
└── .husky/                   # hooks de commit
```

### Módulos do `terraforme-core`

| Módulo | Responsabilidade |
| --- | --- |
| `status` | `Component`/`State` (instalado, conflito, não instalado), busca no `PATH`, leitura de `--version` |
| `extension` | Instala, atualiza e remove RTK e AI Memory pelos scripts oficiais; AI Jail pelo tarball do release (conferido pelo `.sha256`); define os relatórios de cada extensão |
| `agent` | `Provider`, `Mode` (none/eco/high/hybrid), `CliMode` (presets de início: eco/custom/security/high), trait `Adapter` e `agent::prepare()` (monta o comando do agente igual para todos) |
| `agent/claude.rs` | Adapter do Claude Code |
| `agent/opencode.rs` | Adapter do OpenCode (v2+) |
| `jail` | Integração com o AI Jail: configuração padrão em `~/.ai-jail` e `wrap()` (envolve o comando do agente) |
| `usage` | Consumo de tokens por agente (transcripts do Claude Code ou `opencode stats --json`) |
| `limits` | Quanto resta dos limites do plano/conta (API OAuth do Claude, `/usage` do `agy`) ou do orçamento configurado |
| `config` | `Config { provider, mode, jail, budget }` em `config.toml` |
| `space` | Pastas de trabalho nomeadas (*spaces*) em `spaces.toml` |

---

## 11. Como contribuir

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

- Qualquer feature, correção, mudança de comportamento ou de build: bumpa a versão em `Cargo.toml` (`[workspace.package] version`, SemVer) e adiciona uma seção `## X.Y.Z - AAAA-MM-DD` no topo do `CHANGELOG.md`.
- Commits seguem Conventional Commits em inglês, verificados por `commitlint` no hook `commit-msg`.
- Branches: `main` (lançamentos, tags `vX.Y.Z`), `develop` (integração), `feat/<nome>` e `fix/<nome>` (saem de `develop` e voltam para ela).
