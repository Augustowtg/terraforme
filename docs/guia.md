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
terraforme status                        # extensões (RTK, AI Memory, AI Jail)
```

Depois de uma atualização, o script mostra as novidades do [`CHANGELOG.md`](../CHANGELOG.md).

## Extensões

[RTK](https://github.com/rtk-ai/rtk), [AI Memory](https://github.com/akitaonrails/ai-memory) e
[AI Jail](https://github.com/akitaonrails/ai-jail):

```bash
terraforme extension --status              # status e versão de todas
terraforme extension rtk --status          # de uma só
terraforme extension --install             # instala todas (pula as já instaladas)
terraforme extension ai-memory --install   # instala uma só
terraforme extension rtk --update          # atualiza (nome obrigatório)
terraforme extension rtk --delete          # remove (nome obrigatório; -y pula a confirmação)
```

A instalação usa os scripts oficiais de cada projeto (em `~/.local/bin`) e já configura os agentes
(`rtk init -g` / `ai-memory setup`). A remoção desfaz essa configuração e apaga o binário; os dados
do AI Memory (wiki, banco) são preservados. O RTK instala por padrão a edição BUSL-1.1 — para a
Apache-2.0: `RTK_EDITION=oss terraforme extension rtk --install`.

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
terraforme config --mode eco         # define o modo padrão
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

Como os modos são aplicados ao Claude Code está em [arquitetura](arquitetura.md#como-os-modos-funcionam-no-claude-code).

## Terminal do `terraforme cli`

O `terraforme cli` abre o agente dentro do terminal do terraforme, com abas no topo:

```
 terraforme   ⇧F1 claude   ⇧F2 Consumo   ⇧F3 RTK · gain   ⇧F4 AI Memory   F10 sair   ⇧Tab alterna · …
```

- **claude** — o agente. Cada instância é uma sub-aba (`claude 1`, `claude 2`…), todas rodando ao
  mesmo tempo; `F12` abre outra, `Shift+F12` fecha a aberta e `Alt+1`…`Alt+9` ou `Ctrl+←→`
  trocam de instância, como as sub-abas de qualquer aba. A sub-aba some quando a instância termina,
  e o terraforme sai quando a última termina.
- **Consumo** — três sub-abas, atualizadas a cada 5s:
  - **Limites** — quanto já foi usado e quanto falta. Com login OAuth no Claude Code
    (Pro/Max/Team), usa os limites do plano (sessão de 5h e semana, os mesmos do `/usage`); sem
    login, compara os transcripts com o orçamento configurado.
  - **Tokens** — entrada, saída e cache da sessão atual (a da instância aberta), do projeto nas
    últimas 24h e de todos os projetos em 5h/24h/7 dias.
  - **Subagentes** — a sessão atual dividida entre o agente principal e cada subagente.
- **RTK · gain** — só nos modos com RTK: `rtk gain` (geral, neste projeto, por dia, histórico).
- **AI Memory** — só nos modos com AI Memory: status, projetos e handoffs.
- **AI Jail** — só com o AI Jail ligado: `ai-jail status` e a sandbox montada (`--dry-run`).

| Tecla | Ação |
| --- | --- |
| `Shift+Tab` | próxima aba |
| `Shift+F1`…`Shift+F4` | vai direto para a aba |
| `Alt+1`…`Alt+9` | vai direto para a sub-aba da aba aberta (instância do agente, visão do consumo ou relatório) |
| `Ctrl+←→` / `Ctrl+PgUp/PgDn` | vai para a sub-aba vizinha, em volta (o mesmo em todas as abas) |
| `F12` / `Shift+F12` | abre outra instância do agente / fecha a aberta |
| `F10` | sai (encerra todas as instâncias) |
| `Shift+PgUp/PgDn` | rola o histórico do agente |

Nas abas de relatório: `↑↓`/`PgUp/PgDn` rolam, `r` recarrega e `Esc` volta para o agente. Na aba do
agente, `Tab` e `F1`–`F9` chegam ao Claude, mas `Shift+Tab` não (troca de aba). O agente continua
rodando enquanto outra aba está aberta.

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
