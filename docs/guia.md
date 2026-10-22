# Guia de uso

Guia principal para quem usa o terraforme: instalação (Linux e Windows), extensões, agente e modos,
o terminal do `terraforme cli` e os spaces. Detalhes internos estão em [Arquitetura](arquitetura.md).

## Instalação

### Binários do release (sem Rust)

```bash
curl -fsSL https://raw.githubusercontent.com/Augustowtg/terraforme/main/install.sh | sh
```

```powershell
irm https://raw.githubusercontent.com/Augustowtg/terraforme/main/install.ps1 | iex
```

- Linux x86_64/aarch64 (musl) e Windows x86_64; instala `terraforme` e `terraforme-mcp` em
  `~/.local/bin` (Windows: `%LOCALAPPDATA%\Programs\terraforme\bin`).
- Variáveis: `TERRAFORME_VERSION=v0.19.0` fixa a versão; `PREFIX` muda a pasta (`$PREFIX/bin`).
- A [janela flutuante](janela-flutuante.md) não vem nos binários do release.

### Do código-fonte (Linux)

```bash
./install.linux.sh                       # compila em release e instala em ~/.local/bin
./install.linux.sh --update              # git pull (fast-forward) + compila + instala
./install.linux.sh --dist                # build otimizada (LTO), mais lenta para compilar
./install.linux.sh --restart             # reinicia o terraforme no fim sem perguntar
PREFIX=/usr/local ./install.linux.sh     # em /usr/local/bin (usa sudo)
./install.linux.sh --uninstall           # remove
```

Instala `terraforme` e `terraforme-mcp` (e, com GNOME, a extensão do GNOME Shell usada pela janela
flutuante, que é o plugin `terraforme plugin install pet`). Depois de uma atualização, o script mostra as novidades do
[`CHANGELOG.md`](../CHANGELOG.md).

### Do código-fonte (Windows)

Pré-requisitos: `git` e Rust ([rustup](https://rustup.rs), com o "Desktop development with C++"
do Visual Studio Build Tools). No PowerShell, na pasta do repositório:

```powershell
.\install.windows.ps1                    # compila e instala em %LOCALAPPDATA%\Programs\terraforme\bin
.\install.windows.ps1 -Update            # git pull (fast-forward) + compila + instala
.\install.windows.ps1 -Dist              # build otimizada (LTO)
.\install.windows.ps1 -Restart           # reinicia o terraforme no fim sem perguntar
$env:PREFIX='C:\tools'; .\install.windows.ps1   # em C:\tools\bin
.\install.windows.ps1 -Uninstall         # remove (e tira a pasta do PATH do usuário)
```

- Script bloqueado: `powershell -ExecutionPolicy Bypass -File .\install.windows.ps1`.
- A pasta de instalação entra no `PATH` do usuário: abra um terminal novo depois.
- No Windows não há AI Jail nem janela flutuante.

### Atualizar e reiniciar

```bash
terraforme upgrade                # no repositório: install.*.sh --update; por release: baixa a última
terraforme upgrade --no-pull      # só recompila o código local
terraforme upgrade --dist         # build otimizada (LTO)
terraforme upgrade --repo DIR     # repositório em outro lugar (ou TERRAFORME_REPO)
terraforme upgrade --restart      # reinicia no fim sem perguntar
terraforme restart                # reinicia o daemon MCP e a janela flutuante com os binários novos
```

Depois do `restart`, os `terraforme cli` abertos se reconectam sozinhos; os agentes já abertos só
voltam a usar o MCP depois de reiniciados (`Ctrl+Alt+R`).

### Comandos gerais

| Comando | O que faz |
| --- | --- |
| `terraforme` | mostra a ajuda (o mesmo que `--help`; `-h` resume) |
| `terraforme help <comando>` | ajuda de um comando |
| `terraforme --version` / `-V` / `version` | versão |
| `terraforme status` | extensões instaladas (e versão) e as pastas do terraforme |
| `terraforme --historico` (ou `--history`) | pastas em que o `terraforme cli` foi aberto: spaces e recentes |
| `terraforme completions <shell>` | script de autocompletar do shell |

## Extensões

[RTK](https://github.com/rtk-ai/rtk) (economia de tokens),
[AI Memory](https://github.com/akitaonrails/ai-memory) (memória entre sessões) e
[AI Jail](https://github.com/akitaonrails/ai-jail) (sandbox):

```bash
terraforme setup                            # configura tudo: agentes, extensões e skills
terraforme extension --status               # status e versão de todas
terraforme extension rtk --status           # de uma só
terraforme extension --install              # instala todas (pula as já instaladas)
terraforme extension ai-memory --install    # instala uma só
terraforme extension --setup                # escolhe agentes e extensões e configura
terraforme extension ai-memory --setup      # refaz a configuração de uma extensão
terraforme extension rtk --update           # atualiza (nome obrigatório)
terraforme extension rtk --delete           # remove (nome obrigatório; -y pula a confirmação)
terraforme extension ai-jail --project      # cria o .ai-jail na pasta atual
terraforme extension ai-jail --default true # liga o AI Jail por padrão (false desliga)
```

- `terraforme setup` é o caminho mais curto para deixar a máquina pronta; o passo a passo e o que
  cada configuração altera estão em [Setup](setup.md), e o fluxo de `extension --setup` em
  [Setup interativo](setup-interativo.md).
- A instalação usa os scripts oficiais de cada projeto (em `~/.local/bin`) e já configura a
  extensão (`rtk init -g` / `ai-memory setup`).
- A remoção desfaz essa configuração e apaga o binário; os dados do AI Memory (wiki, banco) são
  preservados.
- O RTK instala por padrão a edição BUSL-1.1. Para a Apache-2.0:
  `RTK_EDITION=oss terraforme extension rtk --install`.

### AI Jail

- Não tem script de instalação: o terraforme baixa o binário do último release (Linux x86_64 ou
  macOS arm64), confere o `.sha256` publicado e o instala em `~/.local/bin`.
- No Linux precisa do `bwrap` (pacote `bubblewrap`); o terraforme exige a versão 2.9+.
- Update/delete só mexem nessa cópia; uma instalada por Homebrew/AUR/cargo/Nix é atualizada pelo
  próprio gerenciador.
- A instalação cria uma configuração padrão em `~/.ai-jail` se ainda não houver uma (a remoção a
  preserva).
- Não existe no Windows: um AI Jail vindo do config, de um space ou de um `.ai-jail` é ignorado com
  um aviso; só `terraforme cli --jail` dá erro.

Detalhes em [Arquitetura](arquitetura.md#ai-jail).

## Agente e modos

### Configuração padrão

Fica em `~/.config/terraforme/config.toml` (`%APPDATA%\terraforme` no Windows):

```bash
terraforme config                         # mostra a configuração atual
terraforme config --agent                 # lista os agentes e pergunta o padrão
terraforme config --agent opencode        # define o agente (claude, opencode, antigravity)
terraforme config --cli                   # lista os presets de início e pergunta
terraforme config --cli security          # define o preset (eco, custom, security, high)
terraforme config --mode hybrid           # define o modo (none, eco, high, hybrid, custom)
terraforme config --jail true             # roda o agente no AI Jail por padrão (false desliga)
terraforme config --budget-5h <tokens> --budget-week <tokens>   # orçamento sem login OAuth (0 remove)
terraforme config --mcp-ui true           # liga a UI web do daemon MCP
terraforme config --pet true              # abre a janela flutuante com o terraforme cli
```

Modos (quais extensões o agente usa):

| Modo | Extensões |
| --- | --- |
| `none` | nenhuma |
| `eco` | RTK |
| `high` | AI Memory |
| `hybrid` | RTK + AI Memory |
| `custom` | pergunta quais usar |

O AI Jail não faz parte do modo: é ligado à parte (`--jail`). O **preset de início**
(`config --cli`) define modo e sandbox de uma vez:

| Preset | O que liga |
| --- | --- |
| `eco` | RTK |
| `high` | AI Memory |
| `security` | AI Jail, sem extensões |
| `custom` | você escolhe RTK, AI Memory e/ou AI Jail |

### Contas do Claude Code

```bash
terraforme config --account                         # lista as contas
terraforme config --account work=$HOME/.claude-work # registra (a pasta é o CLAUDE_CONFIG_DIR e precisa existir)
terraforme config --remove-account work             # remove
terraforme space api --account work                 # usa a conta num space
```

### Abrir o agente

```bash
terraforme cli                       # agente, modo e sandbox do space atual ou do config
terraforme cli --agent opencode      # outro agente só nesta sessão
terraforme cli --eco                 # só RTK
terraforme cli --high                # só AI Memory
terraforme cli --hybrid              # RTK + AI Memory
terraforme cli --jail                # dentro do AI Jail, em qualquer modo (--no-jail fora)
terraforme cli -n 3                  # três instâncias, cada uma numa sub-aba (até 9)
terraforme cli --plain               # abre o agente direto, sem o terminal do terraforme
terraforme cli --eco -- --continue   # depois de `--`, argumentos vão direto para o agente
```

Ordem de prioridade: flags da linha de comando → preferências do space → `.ai-jail` do projeto (só
a sandbox) → `config.toml`. O `terraforme cli` **não** cria o `.ai-jail` do projeto: use
`terraforme extension ai-jail --project`.

Como os modos são aplicados ao Claude Code está em
[Arquitetura](arquitetura.md#como-os-modos-funcionam-no-claude-code).

## Terminal do `terraforme cli`

O `terraforme cli` abre o agente dentro do terminal do terraforme, com um menu de abas no topo:

```text
 Terraforme │ Agente (claude) │ Consumo │ Extensões │ Configurações │ Sair          ⌃⌥K Atalhos
```

Cada aba e o layout detalhado estão em [TUI](tui/tui.feature.md). Resumo:

- **Agente** — cada instância é uma sub-aba (`claude 1`, `claude 2`…), todas rodando ao mesmo
  tempo. A sub-aba some quando a instância termina; o terraforme sai quando a última termina.
- **Consumo** — tokens e limites, atualizados a cada 5s, em quatro visões
  ([Consumo](tui/consumo/consumo.feature.md)):
  - **Limites** — quanto já foi usado e quanto falta. Claude Code com login OAuth
    (Pro/Max/Team): os limites do plano (5h e semana, os do `/usage`); sem login, o orçamento
    configurado. OpenCode: só o orçamento semanal. Antigravity: os limites que o `agy` informa
    (um grupo de modelos por janela semanal).
  - **Tokens** — entrada, saída, raciocínio e cache da sessão, do projeto e de todos os projetos.
  - **Subagentes** — a sessão dividida entre o agente principal e cada subagente.
  - **Spaces** — histórico por space e agente (24h, 7 dias e total), em `~/.terraforme/usage.db`
    (`%APPDATA%\.terraforme\usage.db` no Windows). O OpenCode não informa o total da sessão e não
    entra.
  - **RTK** (quando o modo usa o RTK) — os relatórios de `terraforme extension rtk --status`.
- **Extensões** — só com AI Memory ou AI Jail: os relatórios de cada um, um por sub-aba,
  carregados ao abrir.
- **Configurações** — o `config.toml` editável, gravado a cada alteração
  ([Config](tui/config/config.feature.md)):
  - **Agente**: Router (o agente; a troca vale ao sair do campo ou da aba e abre uma instância do
    novo sem fechar as outras), Modo, Conta e `Middleware (Experimental)`.
  - **Orçamento**: só o limite de 5 horas é digitado; o diário e o de 7 dias são calculados.
  - **Plugins · config global**: UI web (clique ou `o` abre no navegador), Mascote e MCP.
  - **Avançado**: capturar o mouse, copiar ao selecionar e os tempos/limites do daemon MCP.

  Dentro de um space, Router, Modo e Conta gravam nas preferências do space em vez do
  `config.toml`; o rodapé diz onde cada campo vale e `Del` volta ao valor global. Modo e conta valem
  nas próximas instâncias (`Ctrl+Alt+N`).

### Atalhos

`Ctrl+Alt+K` mostra a lista completa dentro do terminal.

| Tecla | Ação |
| --- | --- |
| `Ctrl+Shift+F1`…`F4` | abre Agente, Consumo, Extensões (só com extensões) ou Configurações |
| `Ctrl+Shift+←/→` | aba anterior / próxima (`←/→` sozinhas nas visões Consumo e Extensões) |
| clique no menu | abre a aba; `Sair` encerra |
| `Ctrl+Alt+Tab` / `Ctrl+Alt+Shift+Tab` | próxima / anterior sub-aba (em todas as abas) |
| `Ctrl+PgUp/PgDn` | sub-aba anterior / próxima |
| `Ctrl+Alt+1`…`9` (ou `Alt+1`…`9`) | vai para a sub-aba; na aba do agente, um número além da última abre uma instância |
| `Ctrl+Alt+N` / `F12` | nova instância (até 9) |
| `Ctrl+Alt+W` / `Shift+F12` | fecha a instância aberta |
| `Ctrl+Alt+R` | reinicia a instância aberta |
| arrastar com o mouse | seleciona e copia ao soltar (clique desfaz) |
| `Shift`+arrastar | seleção nativa do terminal |
| `Ctrl+Alt+C` | copia a seleção, a tela do agente ou o conteúdo da aba |
| `Ctrl+Alt+S` | devolve o mouse ao terminal e de volta |
| roda do mouse | rola o histórico do agente e o conteúdo das outras abas |
| `Shift+PgUp/PgDn` | rola o histórico do agente meia tela |
| `Shift+Home/End` | início do histórico / volta ao vivo |
| `F10` | sai (encerra todas as instâncias) |

Nas abas de conteúdo: `↑↓`/`j k`/`PgUp/PgDn`/`g` rolam, `r` recarrega e `Esc` ou `q` volta ao
agente. Em Configurações: `↑↓` (ou clique) escolhe o campo, `←→`/`Espaço`/`Enter` altera, dígitos
editam números, `Del` volta ao padrão e `o` abre a UI web.

Observações:

- `Tab`, `Shift+Tab`, `F1`–`F9`, `Ctrl+←→` e as demais teclas seguem para o agente.
- `Ctrl+Alt` + dígito só chega distinto em terminais com o protocolo de teclado do kitty (kitty,
  WezTerm, foot, Ghostty, Alacritty…); sem ele, use `Alt+1`…`9`.
- Os atalhos evitam os do ambiente Linux (`Ctrl+Alt+T/L/D/Del`, `Ctrl+Alt+F1`…`F12`,
  `Ctrl+Alt+setas`).
- A cópia vai pelo terminal (OSC 52) e também por `wl-copy`, `xclip` ou `xsel` no Linux e `clip`
  no Windows. O GNOME Terminal ignora o OSC 52: instale um deles.
- O agente continua rodando enquanto outra aba está aberta.

### Middleware (experimental)

Com `Middleware (Experimental)` ligado em Configurações, as próximas instâncias de agentes que
suportam (Claude Code, Antigravity e OpenCode) abrem num chat do terraforme em vez do terminal do
agente. Detalhes em [Agente](tui/agente/agente.feature.md).

### Comunicação entre instâncias (MCP)

O `terraforme cli` sobe o daemon `terraforme-mcp`, que permite às instâncias do Claude Code
listar umas às outras, trocar mensagens e delegar tarefas, além da UI web
(`terraforme config --mcp-ui true`). Detalhes em [MCP](MCP/mcp.feature.md).

### Janela flutuante (mascote)

`terraforme pet` abre uma janela pequena com o mascote, que avisa quando um agente de qualquer
terminal termina e, com um clique, leva à instância (só no Linux). Detalhes em
[Janela flutuante](janela-flutuante.md).

## Spaces

Pastas de trabalho salvas com um nome, em `~/.config/terraforme/spaces.toml`:

```bash
terraforme space --save              # salva a pasta atual; pergunta o nome (padrão: nome da pasta)
terraforme space api --save          # salva a pasta atual como "api"
terraforme space                     # lista e pergunta qual abrir (número ou nome)
terraforme space api                 # abre direto o space "api"
terraforme space --list              # só lista (com as preferências de cada um)
terraforme space api --rename web    # renomeia
terraforme space api --delete        # remove (a pasta não é apagada; -y pula a confirmação)
```

Abrir um space inicia um shell novo na pasta dele, com `TERRAFORME_SPACE` definido; `exit` volta
para onde você estava. Para trocar de pasta no próprio shell, use `--path`, que só imprime a pasta
(a lista e a pergunta vão para o stderr):

```bash
# em ~/.bashrc ou ~/.zshrc
ts() { local dir; dir=$(terraforme space --path "$@") && cd "$dir"; }
```

```powershell
cd (terraforme space api --path)
```

### Preferências do space

Cada space pode ter agente, modo, sandbox e conta próprios, gravados em
`~/.terraforme/<nome do space>/preferences.toml` (`%APPDATA%\.terraforme\...` no Windows), fora da
pasta do projeto. O `terraforme cli` aberto na pasta (ou numa subpasta) usa essas preferências no
lugar do `terraforme config`; flags da linha de comando continuam vencendo.

```bash
terraforme space api --agent opencode --mode eco   # só o que for passado muda
terraforme space api --jail true                   # roda no AI Jail neste space
terraforme space api --account work                # conta do Claude Code neste space
terraforme space api --save --mode hybrid          # salva o space já com preferências
terraforme space api --reset                       # apaga as preferências
```
