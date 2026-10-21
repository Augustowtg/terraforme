# Guia de uso

## Instalação e atualização (Linux)

```bash
./install.linux.sh                       # compila em release e instala em ~/.local/bin
./install.linux.sh --update              # git pull (fast-forward) + compila + instala
./install.linux.sh --dist                # build otimizada (LTO), mais lenta para compilar
PREFIX=/usr/local ./install.linux.sh     # em /usr/local/bin (usa sudo)
./install.linux.sh --uninstall           # remove

terraforme upgrade                       # o mesmo que --update, a partir do próprio terraforme
terraforme upgrade --no-pull             # só recompila (--repo DIR / TERRAFORME_REPO se o repositório mudou de lugar)

terraforme                               # mostra a ajuda (mesmo que --help)
terraforme --version                     # ou -V, ou `terraforme version`
terraforme status                        # extensões (RTK, AI Memory, AI Jail) e pastas do terraforme
```

Depois de uma atualização, o script mostra as novidades do [`CHANGELOG.md`](../CHANGELOG.md).

## Instalação e atualização (Windows)

Pré-requisitos: `git` e Rust ([rustup](https://rustup.rs), com o "Desktop development with C++"
do Visual Studio Build Tools). No PowerShell, na pasta do repositório:

```powershell
.\install.windows.ps1                    # compila e instala em %LOCALAPPDATA%\Programs\terraforme\bin
.\install.windows.ps1 -Update            # git pull (fast-forward) + compila + instala
.\install.windows.ps1 -Dist              # build otimizada (LTO)
$env:PREFIX='C:\tools'; .\install.windows.ps1   # em C:\tools\bin
.\install.windows.ps1 -Uninstall         # remove (e tira a pasta do PATH do usuário)
```

Se a execução de scripts estiver bloqueada, use
`powershell -ExecutionPolicy Bypass -File .\install.windows.ps1`. A pasta de instalação é
adicionada ao `PATH` do usuário (abra um novo terminal depois). `terraforme upgrade` também
funciona no Windows.

## Extensões

[RTK](https://github.com/rtk-ai/rtk), [AI Memory](https://github.com/akitaonrails/ai-memory) e
[AI Jail](https://github.com/akitaonrails/ai-jail):

```bash
terraforme setup                           # configura tudo: agentes, extensões e skills
terraforme extension --status              # status e versão de todas
terraforme extension rtk --status          # de uma só
terraforme extension --install             # instala todas (pula as já instaladas)
terraforme extension ai-memory --install   # instala uma só
terraforme extension --setup                # detecta agentes e configura as extensões escolhidas
terraforme extension ai-memory --setup      # refaz o setup individual da extensão
terraforme extension rtk --update          # atualiza (nome obrigatório)
terraforme extension rtk --delete          # remove (nome obrigatório; -y pula a confirmação)
```

`terraforme setup` é o caminho mais curto para deixar a máquina pronta: identifica os agentes no
`PATH`, instala as extensões que faltarem e as configura em cada agente e, por fim, instala as
skills embutidas no binário na pasta global de skills de cada agente (`~/.claude/skills`,
`~/.config/opencode/skills`). Se não houver nenhum agente, ele mostra a lista para marcar quais
instalar (`↑`/`↓` movem, `Espaço` marca, `Enter` confirma) e roda o instalador oficial de cada um
(Claude Code: `claude.ai/install.sh` / `install.ps1`; OpenCode: `opencode.ai/install`, ou
`npm install -g opencode-ai` no Windows).

A instalação usa os scripts oficiais de cada projeto (em `~/.local/bin`) e já configura os agentes
(`rtk init -g` / `ai-memory setup`). A remoção desfaz essa configuração e apaga o binário; os dados
do AI Memory (wiki, banco) são preservados. O RTK instala por padrão a edição BUSL-1.1 — para a
Apache-2.0: `RTK_EDITION=oss terraforme extension rtk --install`.

`extension --setup` não instala binários: detecta os agentes no `PATH`, pergunta quais agentes e
extensões configurar e executa as etapas específicas de cada integração. Consulte a
[documentação do setup interativo](setup-interativo.md) para os detalhes.

O AI Jail não tem script de instalação: o terraforme baixa o binário do último release (Linux x86_64
ou macOS arm64), confere o `.sha256` publicado e o instala em `~/.local/bin`; no Linux ele precisa do
`bwrap` (pacote bubblewrap). Update/delete só mexem nessa cópia — uma instalada por
Homebrew/AUR/cargo/Nix é atualizada pelo próprio gerenciador (o terraforme exige a 2.9+). A
instalação também cria uma configuração padrão em `~/.ai-jail` se ainda não houver uma (a remoção a
preserva). Detalhes em [arquitetura](arquitetura.md#ai-jail).

## Agente e modos

Configuração em `~/.config/terraforme/config.toml`:

```bash
terraforme config                    # mostra provider e modo padrão
terraforme config --provider         # lista os providers (claude; cursor ainda não implementado)
terraforme config --provider claude  # define o provider
terraforme config --cli              # lista os modos de início e pergunta o padrão
terraforme config --cli eco          # define o modo de início (eco, custom, security, high)
terraforme config --mode eco         # define o modo padrão
terraforme config --mode custom      # escolhe as extensões (RTK, AI Memory e/ou AI Jail)
terraforme config --jail true        # roda o agente dentro do AI Jail por padrão (false desliga)
terraforme config --budget-5h <tokens> --budget-week <tokens>   # orçamento sem login OAuth

terraforme cli                       # abre o agente no modo padrão
terraforme cli --eco                 # só RTK (economia de tokens)
terraforme cli --high                # só AI Memory (memória entre sessões)
terraforme cli --hybrid              # RTK + AI Memory
terraforme cli --eco -- --continue   # depois de `--`, argumentos vão direto para o claude
terraforme cli --plain               # abre o agente direto, sem o terminal do terraforme
terraforme cli -n 3                  # três instâncias do agente, sub-abas da aba claude (até 9)
terraforme cli --jail                # dentro do AI Jail (sandbox), em qualquer modo; --no-jail fora
```

`terraforme config --cli` define o **modo de início**, um atalho que já escolhe o modo e a sandbox:

| Modo | O que liga |
| --- | --- |
| `eco` | RTK (economia de tokens) |
| `high` | AI Memory (memória entre sessões) |
| `security` | AI Jail (sandbox), sem extensões |
| `custom` | você escolhe as extensões (RTK, AI Memory e/ou AI Jail) |

Se uma extensão do modo precisa de algo no projeto, o `terraforme cli` confere e configura ao iniciar:
com o AI Jail ligado numa pasta sem `.ai-jail`, ele cria o arquivo (`ai-jail --init <agente>`) — exceto
na sua pasta pessoal, onde o `~/.ai-jail` é a configuração global.

Como os modos são aplicados ao Claude Code está em [arquitetura](arquitetura.md#como-os-modos-funcionam-no-claude-code).

## Terminal do `terraforme cli`

O `terraforme cli` abre o agente dentro do terminal do terraforme, com abas no topo:

```
  terraforme   ⌃⇧1 claude   ⌃⇧2 Consumo   ⌃⇧3 Extensões   ⌃⇧4 Config   F10 sair   ⌃⇧Tab alterna · ⌃⌥K atalhos · …
```

- **claude** — o agente. Cada instância é uma sub-aba (`claude 1`, `claude 2`…), todas rodando ao
  mesmo tempo; `Ctrl+Alt+N` (ou `F12`) abre outra, `Ctrl+Alt+W` (ou `Shift+F12`) fecha a aberta,
  `Ctrl+Alt+R` reinicia e `Ctrl+Alt+1`…`9` troca de instância — um número além da última abre uma
  nova (com uma instância, `Ctrl+Alt+2` abre a segunda). A sub-aba some quando a instância termina,
  e o terraforme sai quando a última termina.
- **Consumo** — o consumo de tokens, atualizado a cada 5s, e a economia do RTK:
  - **Limites** — quanto já foi usado e quanto falta. Com login OAuth no Claude Code
    (Pro/Max/Team), usa os limites do plano (sessão de 5h e semana, os mesmos do `/usage`); sem
    login, compara os transcripts com o orçamento configurado. Com o OpenCode, não há limites de
    plano nem janela de 5h (o `opencode stats` não os expõe): valem o orçamento semanal e a
    contagem de tokens/5h da sessão local. Com o Antigravity, os limites são os que o `agy` informa
    (`agy -p /usage`): um grupo de modelos por janela semanal, com o quanto resta e quando ela
    zera.
  - **Tokens** — entrada, saída, raciocínio e cache da sessão atual (a da instância aberta), do
    projeto nas últimas 24h e de todos os projetos em 5h/24h/7 dias; com o OpenCode, o total do
    projeto hoje e de todos hoje/7 dias, com custo e tokens por modelo.
  - **Subagentes** — a sessão atual dividida entre o agente principal e cada subagente; com o
    OpenCode, a contagem de sessões, prompts e subagentes.
  - **RTK** (quando o modo usa o RTK) — Economia, Neste projeto, Por dia e Histórico, os
    relatórios de `terraforme extension rtk --status`.
- **Extensões** — só aparece quando o modo usa o AI Memory ou o AI Jail está ligado: os relatórios
  de cada um, um por sub-aba — AI Memory (Status, Projetos, Handoffs) e AI Jail (Status, Sandbox).
  São os mesmos de `terraforme extension <nome> --status`, carregados quando a sub-aba é aberta.
- **Config** — a configuração geral (`config.toml`), editável: agente, modo e AI Jail
  (valem no próximo `terraforme cli`), orçamento de tokens, preferências da TUI (capturar o mouse,
  copiar ao selecionar — aplicadas na hora) e o daemon MCP. Cada alteração é gravada na hora.
  Dentro de um space, agente, modo e AI Jail gravam nas preferências do space
  (`~/.terraforme/<nome do space>/preferences.toml`; no Windows, `%APPDATA%\.terraforme\...`) em vez do `config.toml` global — é o arquivo
  que o `terraforme cli` passa a usar ali — e o rodapé diz onde cada campo vale; os outros campos
  continuam no `config.toml`. Apagar um valor com `Delete` tira o override e volta ao `config.toml`.

| Tecla | Ação |
| --- | --- |
| `Ctrl+Shift+F1` | abre a aba Agente |
| `Ctrl+Shift+F2` | abre a aba Consumo |
| `Ctrl+Shift+F3` | abre a aba Extensões (só quando o modo tem alguma além do RTK) |
| `Ctrl+Shift+F4` | abre a aba Config |
| `Ctrl+Alt+Tab` / `Ctrl+Alt+Shift+Tab` | próxima / anterior sub-aba (o mesmo em todas as abas) |
| `Ctrl+Alt+1`…`9` (ou `Alt+1`…`9`) | vai direto para a sub-aba (instância do agente, visão do consumo ou relatório) |
| `Ctrl+PgUp/PgDn` | sub-aba vizinha, em volta |
| `Ctrl+Alt+N` / `F12` | abre outra instância do agente |
| `Ctrl+Alt+W` / `Shift+F12` | fecha a instância aberta |
| `Ctrl+Alt+R` | reinicia a instância aberta |
| arrastar com o mouse | seleciona texto do agente e copia ao soltar (clique desfaz) |
| `Ctrl+Alt+C` | copia a seleção, a tela do agente ou o conteúdo da aba aberta |
| `Ctrl+Alt+S` | devolve o mouse ao terminal (seleção nativa) e de volta |
| `Shift+PgUp/PgDn` · `Shift+Home/End` | rola o histórico do agente · início / volta ao vivo |
| roda do mouse | rola o histórico do agente e o conteúdo das outras abas |
| `Ctrl+Alt+K` | lista todos os atalhos |
| `F10` | sai (encerra todas as instâncias) |

A cópia vai para a área de transferência pelo terminal (OSC 52) e, no Linux, também por
`wl-copy`, `xclip` ou `xsel`, se algum estiver instalado — o GNOME Terminal ignora o OSC 52, então
nele instale um deles. `Ctrl+Alt` + dígito só chega distinto em terminais com o
protocolo de teclado do kitty (kitty, WezTerm, foot, Ghostty, Alacritty…), que a TUI liga quando
existe; sem ele, use `Alt+1`…`9` ou `Shift+Alt+1`…`9`. `Shift+Tab` segue para o agente (Claude Code
o usa para trocar de modo). Os atalhos evitam os do ambiente Linux (`Ctrl+Alt+T/L/D/Del`,
`Ctrl+Alt+F1`…`F12`, `Ctrl+Alt+setas`).

Nas abas de conteúdo: `↑↓`/`PgUp/PgDn` rolam, `r` recarrega e `Esc` volta para o agente. Na aba
Config: `↑↓` escolhe o campo, `←→`/`Enter` altera, dígitos editam números (vazio tira o orçamento),
`Del` volta ao padrão. Na aba do agente, `Tab`, `F1`–`F9` e `Ctrl+←→` chegam ao Claude. O agente
continua rodando enquanto outra aba está aberta.

## Spaces

Pastas de trabalho salvas com um nome (em `~/.config/terraforme/spaces.toml`):

```bash
terraforme space --save              # salva a pasta atual; pergunta o nome (padrão: nome da pasta)
terraforme space --save api          # salva a pasta atual como "api"
terraforme space                     # lista os spaces e pergunta qual abrir (número ou nome)
terraforme space api                 # abre direto o space "api"
terraforme space --list              # só lista
terraforme space api --delete        # remove
```

Abrir um space inicia um shell novo na pasta dele, com `TERRAFORME_SPACE` definido — `exit` volta
para onde você estava. Para ir com `cd` no próprio shell, use `--path`, que só imprime a pasta (a
lista e a pergunta vão para o stderr):

```bash
# ~/.bashrc ou ~/.zshrc
ts() { local dir; dir=$(terraforme space --path "$@") && cd "$dir"; }
```

### Preferências do space

Cada space pode ter preferências próprias — agente, modo, sandbox e conta — gravadas em
`~/.terraforme/<nome do space>/preferences.toml`, fora da pasta do projeto. O `terraforme cli` aberto na pasta (ou numa
subpasta) usa essas preferências no lugar do `terraforme config`; flags da linha de comando
(`--agent`, `--eco`, `--jail`…) continuam vencendo:

```bash
terraforme space api --agent opencode --mode eco   # define (só o que for passado muda)
terraforme space api --jail true                   # roda no AI Jail neste space
terraforme space --save api --mode hybrid          # salva o space já com preferências
terraforme space api --reset                       # apaga as preferências
```

`terraforme space --list` mostra as preferências de cada space.
