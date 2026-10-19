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
`~/.config/opencode/skills`).

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

Como os modos são aplicados ao Claude Code está em [arquitetura](arquitetura.md#como-os-modos-funcionam-no-claude-code).

## Terminal do `terraforme cli`

O `terraforme cli` abre o agente dentro do terminal do terraforme, com abas no topo:

```
  terraforme   ⇧F1 claude   ⇧F2 Consumo   F10 sair   ⇧Tab alterna · …
```

- **claude** — o agente. Cada instância é uma sub-aba (`claude 1`, `claude 2`…), todas rodando ao
  mesmo tempo; `F12` abre outra, `Shift+F12` fecha a aberta e `Alt+1`…`Alt+9` ou `Ctrl+←→`
  trocam de instância, como as sub-abas de qualquer aba. A sub-aba some quando a instância termina,
  e o terraforme sai quando a última termina.
- **Consumo** — três sub-abas, atualizadas a cada 5s:
  - **Limites** — quanto já foi usado e quanto falta. Com login OAuth no Claude Code
    (Pro/Max/Team), usa os limites do plano (sessão de 5h e semana, os mesmos do `/usage`); sem
    login, compara os transcripts com o orçamento configurado. Com o OpenCode, não há limites de
    plano nem janela de 5h (o `opencode stats` não os expõe): valem o orçamento semanal e a
    contagem de tokens/5h da sessão local.
  - **Tokens** — entrada, saída, raciocínio e cache da sessão atual (a da instância aberta), do
    projeto nas últimas 24h e de todos os projetos em 5h/24h/7 dias; com o OpenCode, o total do
    projeto hoje e de todos hoje/7 dias, com custo e tokens por modelo.
  - **Subagentes** — a sessão atual dividida entre o agente principal e cada subagente; com o
    OpenCode, a contagem de sessões, prompts e subagentes.

| Tecla | Ação |
| --- | --- |
| `Shift+Tab` | próxima aba |
| `Shift+F1`…`Shift+F2` | vai direto para a aba |
| `Alt+1`…`Alt+9` | vai direto para a sub-aba da aba aberta (instância do agente ou visão do consumo) |
| `Ctrl+←→` / `Ctrl+PgUp/PgDn` | vai para a sub-aba vizinha, em volta (o mesmo em todas as abas) |
| `F12` / `Shift+F12` | abre outra instância do agente / fecha a aberta |
| `F10` | sai (encerra todas as instâncias) |
| `Shift+PgUp/PgDn` | rola o histórico do agente |

Na aba Consumo: `↑↓`/`PgUp/PgDn` rolam, `r` recarrega e `Esc` volta para o agente. Na aba do
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

### Preferências do space

Cada space pode ter preferências próprias — agente, modo e sandbox — gravadas em
`<pasta do space>/.terraforme/preferences.toml`. O `terraforme cli` aberto na pasta (ou numa
subpasta) usa essas preferências no lugar do `terraforme config`; flags da linha de comando
(`--agent`, `--eco`, `--jail`…) continuam vencendo:

```bash
terraforme space api --agent opencode --mode eco   # define (só o que for passado muda)
terraforme space api --jail true                   # roda no AI Jail neste space
terraforme space --save api --mode hybrid          # salva o space já com preferências
terraforme space api --reset                       # apaga as preferências
```

`terraforme space --list` mostra as preferências de cada space.
