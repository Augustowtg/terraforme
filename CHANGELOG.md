# Changelog

Todas as mudanças relevantes do terraforme ficam neste arquivo. O formato segue o
[Keep a Changelog](https://keepachangelog.com/pt-BR/1.1.0/) e as versões seguem o
[Versionamento Semântico](https://semver.org/lang/pt-BR/): toda alteração entregue sobe a versão
em `Cargo.toml` (`[workspace.package] version`) e ganha uma seção aqui.

## 0.6.0 - 2026-10-17

### Alterado
- Atalhos de sub-abas padronizados no terminal do `terraforme cli`, iguais em todas as abas:
  `Alt+1`…`Alt+9` vai direto para a sub-aba e `Ctrl+←→` / `Ctrl+PgUp/PgDn` passam para a vizinha
  (em volta). Valem para as instâncias do agente, as visões do consumo e os relatórios das
  extensões.
- Nas abas de relatório, `Shift+1`…`Shift+9`, `←→` e `h`/`l` deixam de trocar a sub-aba.

### Corrigido
- Troca de instância do agente: `Shift+1`…`Shift+9` não servem para isso, porque na aba do agente
  chegam ao Claude (digitam `!@#$`…). Use `Alt+1`…`Alt+9` ou `Ctrl+←→`.

## 0.5.0 - 2026-10-17

### Alterado

- Sub-abas padronizadas no terminal do `terraforme cli`, no estilo do Consumo: `Alt+1`…`Alt+9` e
  `Ctrl+PgUp/PgDn` trocam a sub-aba da aba aberta em todas as abas (instâncias do agente, visões do
  consumo e relatórios das extensões), todas com rótulo `⌥N` e o mesmo destaque. Nas abas de
  relatório, `Shift+1`…`Shift+9` e `←→` continuam funcionando. Antes, `Alt+1`…`Alt+9` sempre
  levavam a uma instância do agente, de qualquer aba.
- Por dentro, cada agente de IA fica atrás de um adapter (`agent::Adapter`), para facilitar a
  troca ou a inclusão de outros agentes além do Claude Code.

## 0.4.0 - 2026-10-17

### Adicionado

- Extensão AI Jail (`terraforme extension ai-jail`): sandbox do sistema para o agente. Instala o
  binário do último release em `~/.local/bin` conferindo o checksum, e cria uma configuração
  padrão em `~/.ai-jail` (sem `.ai-jail` nos projetos, sem barra de status, Claude já logado)
  quando ainda não existe uma.
- `terraforme cli --jail`/`--no-jail` e `terraforme config --jail true|false`: roda o agente dentro
  do AI Jail em qualquer modo, com os plugins, binários, dados e a porta do servidor do AI Memory
  das extensões do modo liberados na sandbox. Aba AI Jail no terminal (status e `--dry-run`).

## 0.3.0 - 2026-10-17

### Alterado

- Terminal do terraforme: o agente fica numa só aba (`claude`) e cada instância virou uma sub-aba
  dela (`claude 1`, `claude 2`…), numa segunda linha que também lembra `F12`/`Shift+F12`. Troca de
  instância com `Alt+1`–`9` ou `Ctrl+PgUp/PgDn` (em qualquer aba); `Shift+F1`–`F9` passam a contar
  só as abas principais.

## 0.2.0 - 2026-10-17

### Adicionado

- `terraforme upgrade`: atualiza o próprio terraforme rodando o `install.linux.sh --update` do
  repositório (`--no-pull` só recompila, `--dist` usa a build otimizada, `--repo DIR` /
  `TERRAFORME_REPO` se o repositório mudou de lugar). Reinstala no mesmo prefixo do binário atual.
- `install.linux.sh --update` (`git pull --ff-only` antes de compilar), `--dist` e `--uninstall`;
  ao terminar mostra a versão anterior → a nova e o changelog das versões instaladas.
- Este `CHANGELOG.md`.

### Alterado

- Perfil `release` sem LTO: a última etapa da build (o binário `terraforme`, 181/182) não fica mais
  ~35s compilando num único núcleo. A build com fat LTO + 1 codegen unit passou para o perfil
  `dist` (`cargo build --profile dist` ou `./install.linux.sh --dist`).
- O binário instalado é trocado com `mv` a partir de um arquivo temporário, então a instalação não
  falha com "Text file busy" se houver um `terraforme` rodando.

## 0.1.0 - 2026-10-17

### Adicionado

- `terraforme extension`: status, instalação, atualização e remoção do
  [RTK](https://github.com/rtk-ai/rtk) e do [AI Memory](https://github.com/akitaonrails/ai-memory)
  com os scripts oficiais de cada projeto.
- `terraforme config`: provider (Claude Code), modo padrão e orçamento de tokens (`[budget]`).
- `terraforme cli` com os modos `eco` (RTK), `high` (AI Memory) e `hybrid` (os dois): hooks movidos
  do `~/.claude/settings.json` para plugins locais carregados por sessão com `--plugin-dir`, e o MCP
  global do AI Memory bloqueado na sessão.
- Terminal do terraforme (TUI): uma aba por instância do agente (até 9), aba **Consumo** (Limites,
  Tokens, Subagentes) e aba **Extensões** com os relatórios do RTK e do AI Memory; `--plain` abre o
  agente direto.
- `terraforme space`: diretórios de trabalho nomeados.
- `terraforme status` e `terraforme version`.

