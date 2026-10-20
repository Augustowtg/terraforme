# Changelog

Todas as mudanças relevantes do terraforme ficam neste arquivo. O formato segue o
[Keep a Changelog](https://keepachangelog.com/pt-BR/1.1.0/) e as versões seguem o
[Versionamento Semântico](https://semver.org/lang/pt-BR/): toda alteração entregue sobe a versão
em `Cargo.toml` (`[workspace.package] version`) e ganha uma seção aqui.

## 0.17.0 - 2026-10-20

### Alterado

- As preferências de cada space (agente, modo, AI Jail e conta) saíram da pasta do projeto e agora
  ficam em `~/.terraforme/<nome do space>/preferences.toml` (`TERRAFORME_HOME` troca a raiz). O
  arquivo antigo, `<pasta do space>/.terraforme/preferences.toml`, é movido para lá
  automaticamente. Renomear um space leva as preferências junto, e remover apaga.
- Valem as preferências do space salvo que contém a pasta atual (o mais específico); um
  `.terraforme/preferences.toml` solto numa pasta que não é space deixa de ser lido.
- O nome do space não pode ter `/` nem `\`, nem ser `.` ou `..` (vira o nome da pasta).

### Corrigido

- Uma conta registrada com a pasta padrão `~/.claude` (ex.: `pessoal=~/.claude`) abre o Claude
  Code sem `CLAUDE_CONFIG_DIR`; antes a variável era definida e o Claude Code passava a ler
  `~/.claude/.claude.json` em vez de `~/.claude.json`, perdendo login e servidores MCP.

## 0.16.0 - 2026-10-20

### Adicionado

- Suporte a múltiplas contas do Claude Code por space: `terraforme config --account NOME=PASTA`
  registra uma conta (cada conta é um `CLAUDE_CONFIG_DIR` distinto — credenciais, sessões e
  configurações separadas); `terraforme space <nome> --account <nome-da-conta>` atribui a conta ao
  space, e o `terraforme cli` aberto nele define `CLAUDE_CONFIG_DIR` automaticamente antes de
  iniciar o Claude Code (inclusive dentro do AI Jail). A lista de contas fica em
  `~/.config/terraforme/claude.toml`.

## 0.15.1 - 2026-10-20

### Adicionado

- Limites de consumo do Antigravity (`agent/antigravity.rs`): o adapter pergunta ao próprio agente
  com `agy -p "/usage" --output-format json` e cada grupo de modelos da resposta vira um limite na
  aba Consumo — a porcentagem já usada sai do `remaining_fraction` e o instante em que a janela
  semanal zera vem do `reset_time`, que aparece no título do limite (`· zera em 5d 22h`). Sem login
  no `agy`, vale o orçamento `[budget]` do `config.toml`, como nos outros agentes.

### Alterado

- Rodapé e dica da visão Limites deixam de citar só o Claude Code: com o Antigravity a fonte é o
  `agy -p /usage` e a dica passa a sugerir entrar na conta pelo `agy` antes de cair no orçamento.

## 0.15.0 - 2026-10-20

### Adicionado

- Adapter padronizado de limites de consumo (`Adapter::quota`): cada CLI traduz o que o dele
  informa para o formato `Quota` (porcentagem usada, tokens usados e o limite total, quando a
  janela zera) e o terraforme é quem calcula o quanto falta e desenha a barra. A consulta da API
  OAuth do Claude Code virou parte do adapter dele (`agent/claude.rs`); os agentes sem limites
  usam o default e caem no orçamento do `config.toml`.

### Alterado

- Barra de limites da aba Consumo no estilo do Claude Code: o verde nasce cheio em 100% e vai
  encurtando a cada consumo (`████████░░░░  75% restante`), esquentando para amarelo e vermelho
  quando o que falta aperta; o título de cada limite continua trazendo o total de tokens e quando
  a janela zera.

## 0.14.3 - 2026-10-20

### Adicionado

- Registro de abas na TUI (padrão de projeto **Registro**): cada aba vira uma entrada única
  (`TabSpec` em `TABS`) com o nome, o atalho, as sub-abas próprias e o comportamento
  (`TabKind`), e o menu, os atalhos e a janela de ajuda saem todos dessa lista.

### Alterado

- Atalhos de abas na TUI, um por aba e na ordem do menu: `Ctrl+Shift+F1` abre só o agente,
  `Ctrl+Shift+F2` o Consumo, `Ctrl+Shift+F3` as Extensões (quando o modo abre a aba) e
  `Ctrl+Shift+F4` a Config; o menu e a janela de atalhos (`Ctrl+Alt+K`) mostram a mesma tecla e
  uma aba fora do menu não tem atalho.

## 0.14.2 - 2026-10-20

### Alterado

- Atalhos de abas na TUI: mantidos exclusivamente `Ctrl+Shift+F1` (Consumo), `Ctrl+Shift+F2`
  (Config) e `Ctrl+Shift+F3` (terceira função: Extensões ou Agente).
- Atalhos duplicados e extras de troca de abas (`Shift+1`/`2`, `F1`/`F2`, `Ctrl+Shift+1`–`9`,
  `Shift+F1`–`F9`, `Ctrl+Shift+Tab` e `Shift+Tab`) foram desativados, liberando teclas de função,
  dígitos e `Shift+Tab` para o agente sem interferência da TUI.

## 0.14.1 - 2026-10-19

### Corrigido

- Atalhos de sub-aba nas visões (Consumo, Extensões, Config): `←`/`→` agora trocam a sub-aba
  aberta; `Shift+Alt+1`–`9` funciona sem o protocolo do kitty (terminal envia símbolo + Alt).

## 0.14.0 - 2026-10-19

### Adicionado

- Aba **Config** na TUI do `terraforme cli`: edita o `config.toml` (agente, modo, AI Jail, preset
  doc, orçamento de tokens, daemon MCP e as preferências da TUI) e grava a cada alteração.
- Seção `[tui]` no `config.toml`: `mouse` (capturar o mouse ao abrir) e `copy_on_select` (copiar
  ao soltar a seleção), ambas ligadas por padrão e aplicadas na hora pela aba Config.
- Seleção de texto na aba do agente: arrastar com o mouse seleciona e copia ao soltar, pelo
  terminal (OSC 52) e, no Linux, por `wl-copy`/`xclip`/`xsel`; `Ctrl+Alt+C` copia a seleção, a
  tela do agente ou o conteúdo da aba aberta; `Ctrl+Alt+S` devolve o mouse ao terminal.
- Atalhos novos: `Ctrl+Alt+N` abre instância, `Ctrl+Alt+W` fecha, `Ctrl+Alt+R` reinicia,
  `Ctrl+Alt+K` lista todos os atalhos, `Shift+Home`/`Shift+End` vão ao início do histórico e de
  volta ao vivo.
- A TUI liga o protocolo de teclado do kitty quando o terminal suporta, para distinguir
  `Ctrl+Shift`/`Ctrl+Alt` das teclas comuns.

### Alterado

- Abas: `Ctrl+Shift+Tab` passa para a próxima e `Ctrl+Shift+1`…`9` vai direto (`Shift+F1`…`F9`
  continuam). Com o protocolo do kitty, `Shift+Tab` chega ao agente (troca de modo do Claude Code).
- Sub-abas: `Ctrl+Alt+Tab` passa para a próxima e `Ctrl+Alt+1`…`9` vai direto (`Alt+1`…`9` e
  `Ctrl+PgUp/PgDn` continuam); na aba do agente, um número além da última instância abre uma nova.
- Os relatórios do RTK saíram da aba Extensões e viraram sub-abas do Consumo; Extensões só aparece
  com AI Memory ou AI Jail.
- Aba **Config** dentro de um space: agente, modo e AI Jail gravam nas preferências do space
  (`<pasta do space>/.terraforme/preferences.toml`, criadas se não existirem) em vez do
  `config.toml` global, os valores mostrados são os que a sessão realmente usa e o rodapé indica
  onde cada campo vale; os demais campos (preset doc, orçamento, TUI, MCP) continuam no
  `config.toml`. Fora de space, nada muda.

### Removido

- `Ctrl+←→` e `Shift+Alt+símbolo` não trocam mais de sub-aba: `Ctrl+←→` volta a chegar ao agente
  (movimento por palavra).

### Corrigido

- Não era possível selecionar e copiar texto do agente: a captura do mouse exigia `Shift`+arrastar.
- Avisos que chegavam com uma aba de conteúdo aberta só apareciam ao voltar para o agente.

## 0.13.0 - 2026-10-19

### Adicionado

- **`terraforme-mcp`** — novo crate binário: daemon MCP compartilhado entre todas as
  instâncias `terraforme-cli`. Inicia sob demanda ao lançar `terraforme cli`, registra
  cada instância como cliente (heartbeat + watchdog de PID), e encerra automaticamente
  após um período de carência configurável quando o último cliente desconecta.
- **Ferramenta `spawn_subagent`** — disponível via MCP para o Claude Code: Claude 1
  pode delegar uma tarefa a um Claude 2 em nova aba do TUI; o resultado (saída + código
  de saída) é devolvido ao agente solicitante quando o subagente conclui.
- **`[mcp]` em `config.toml`** — seção opcional `[mcp]` com `grace_seconds` (padrão 5),
  `subagent_timeout_seconds` (padrão 300) e `max_simultaneous_subagents` (padrão 3).

### Corrigido

- **AI Jail: `bun` não encontrado** — a sandbox usa a home privada e só montava o cache do bun,
  então `bun`/`bunx` (em `~/.bun/bin`, fora do `PATH` lá dentro) não existiam para o agente. O
  terraforme agora os monta em modo leitura em `~/.local/bin` a cada lançamento com o jail.

## 0.12.0 - 2026-10-19

### Adicionado

- **Renomear space** (`terraforme space <nome> --rename <novo-nome>`): muda o
  nome de um space salvo mantendo a pasta e as preferências intactas.

### Corrigido

- **`.ai-jail` não é mais criado automaticamente** no projeto a cada `terraforme
  cli`: o arquivo só é gerado explicitamente via `terraforme extension ai-jail
  --project`. Antes, qualquer sessão com `--jail` criava um `.ai-jail` vazio na
  pasta, fazendo o projeto entrar em modo jailed permanentemente mesmo sem o
  usuário pedir.

## 0.11.1 - 2026-10-19

### Corrigido

- **Rolagem com a roda do mouse no terminal do agente** (`terraforme cli`): a
  roda não rolava a saída do agente — o terminal a convertia em setas, que
  chegavam ao Claude como navegação no histórico de prompts. Agora a TUI captura
  o mouse e a roda rola o histórico do agente (o mesmo do `Shift+PgUp/PgDn`;
  qualquer tecla volta ao vivo), é repassada ao agente quando ele mesmo pede
  eventos de mouse e rola também as abas Consumo e Extensões e o popup de
  avisos. Com a captura, selecionar texto pede `Shift` ao arrastar.

## 0.11.0 - 2026-10-19

### Adicionado

- **Aba Extensões de volta no `terraforme cli`** (como na 0.6): quando o modo
  usa alguma extensão ou o AI Jail está ligado, o menu ganha a aba
  `Extensões`, com os relatórios de cada uma numa sub-aba (RTK: Economia,
  Neste projeto, Por dia, Histórico; AI Memory: Status, Projetos, Handoffs;
  AI Jail: Status, Sandbox). As sub-abas trocam com `Shift+Alt+1`…`9` e
  `Ctrl+←→`, `r` recarrega e `Esc` volta para o agente.
- **`terraforme config --mode custom`**: pergunta quais extensões usar (RTK,
  AI Memory e/ou AI Jail) e grava o modo e a sandbox correspondentes.
- **Preparação do projeto no `terraforme cli`**: quando uma extensão do modo
  precisa de algo no projeto, o `terraforme cli` confere e configura ao
  iniciar. Hoje é o AI Jail: sem `.ai-jail` na pasta, ele é criado com
  `ai-jail --init <agente>` (nunca na pasta do usuário, onde o `.ai-jail` é o
  global).

### Alterado

- O `terraforme cli` não abre mais o aviso das preferências do space (nem o
  de "AI Jail ligado pelo .ai-jail") a cada início; o popup inicial fica só
  para o que mudou nesta execução e para os alertas.

## 0.10.1 - 2026-10-19

### Alterado

- **Ajuda nos erros da linha de comando:** um argumento inválido, faltando ou
  desconhecido mostra o erro seguido da ajuda completa do subcomando digitado
  (por exemplo, `terraforme space --x` mostra a ajuda do `terraforme space`);
  erros na execução de um comando terminam com a dica
  `Para ver como usar: terraforme <comando> --help`.

## 0.10.0 - 2026-10-19

### Adicionado

- **Instância de documentação no terminal (`terraforme cli`):** quando o agente
  precisa gerar uma documentação, ele grava um pedido em
  `~/.config/terraforme/doc-request.json` e o terminal abre uma nova sub-aba
  do agente (`📄 doc`) já com as instruções da tarefa; a instância fica aberta
  até o usuário fechá-la (`Shift+F12`) ou continuar existindo
  (`doc::write_request`, `doc::take_request` no núcleo, watcher na TUI e
  entrega do prompt depois da inicialização do agente).
- **Modo de início `doc` no `terraforme config --cli`:** AI Memory com o MCP
  dela disponível (execuções usam o modo `high`, sem sandbox); a escolha fica
  registrada no `config.toml` (`doc = true`) e volta no resumo da configuração.
- **Preferências por space:** agente, modo e sandbox de cada space em
  `<pasta do space>/.terraforme/preferences.toml`, definidos com
  `terraforme space <nome> --agent/--mode/--jail` (ou junto do `--save`) e
  apagados com `--reset`. O `terraforme cli` aberto na pasta do space (ou numa
  subpasta) usa essas preferências no lugar do `config.toml`; flags da linha
  de comando continuam vencendo. `terraforme space --list` e a abertura do
  space mostram as preferências.
- **Skill embutida `terraforme-doc`:** instalada pelo `terraforme setup`, ensina
  o agente quando e como instanciar a instância de documentação e o que
  incluir nas instruções dela.

## 0.9.0 - 2026-10-18

### Adicionado
- `terraforme setup`: configura a máquina em três passos — identifica os agentes (Claude Code,
  OpenCode) no `PATH`, instala as extensões (RTK, AI Memory, AI Jail) que faltarem e as configura
  em cada agente, e instala as skills embutidas na pasta global de skills de cada agente
  (`~/.claude/skills`, `~/.config/opencode/skills`).
- Skill `terraform-requisitos` embutida no binário (pasta `skills/` do repositório, copiada para o
  agente a partir de `build.rs`): ficou independente do agente (resolve a própria pasta em vez de
  `~/.claude`) e sem a parte do modelo di2win.
- `terraforme config --cli`: lista e define o **modo de início** do `terraforme cli` — `eco` (RTK),
  `high` (AI Memory), `security` (AI Jail, sem extensões) e `custom` (você escolhe as extensões:
  RTK, AI Memory e/ou AI Jail). Sem valor, lista os modos e pergunta o padrão; com valor, define
  direto. O `Mode` ganhou a variante `none` (nenhuma extensão), usada pelo modo `security`.

### Alterado
- A aba **Extensões** saiu do menu principal do `terraforme cli`. Os relatórios continuam
  disponíveis pela linha de comando (`terraforme extension <nome> --status`).

### Removido
- As abas **Monitoramento** e **Setup skills** saíram do menu principal do `terraforme cli`; agora
  ele tem só o agente e a aba Consumo.

## 0.8.1 - 2026-10-18

### Adicionado
- A aba Consumo também lê as estatísticas nativas do OpenCode (`opencode stats --json`), com
  tokens por projeto/modelo, totais diários e semanais e custo estimado. Limites de assinatura,
  janela móvel de 5h e tokens por sessão/subagente não são oferecidos por essa interface.

### Corrigido
- O MCP do AI Memory volta a subir no OpenCode: a ponte `ai-memory mcp-bridge` ganhou
  `--no-session-id`, sem o qual ela procura o `CLAUDE_CODE_SESSION_ID` (inexistente no OpenCode)
  e recusa iniciar, deixando a sessão sem as ferramentas de memória.

## 0.8.0 - 2026-10-18

### Adicionado
- `terraforme extension --setup` detecta Claude Code e OpenCode no `PATH`, permite selecionar os
  agentes e, em seguida, RTK, AI Memory e/ou AI Jail. Configura cada integração escolhida sem
  instalar extensões automaticamente; o AI Jail é configurado uma vez para a máquina.
- A configuração escolhida de AI Memory é aplicada automaticamente ao agente: `ai-memory setup` no
  Claude Code e `ai-memory install-hooks --agent opencode2 --apply` no OpenCode.

## 0.7.0 - 2026-10-18

### Adicionado
- Agente **OpenCode** (2 ou mais novo): `terraforme config --agent opencode` e o `terraforme cli` o
  abre com os mesmos modos e o AI Jail. RTK e AI Memory viram plugins do terraforme ligados por
  sessão (`OPENCODE_CONFIG_CONTENT`, com `--standalone`); os plugins do RTK e do AI Memory em
  `~/.config/opencode/plugins` e o MCP do AI Memory no `opencode.json` saem de lá (com backup
  `.terraforme-bak`). O plugin do `rtk init --opencode` não carrega no OpenCode 2, então o RTK usa um
  plugin próprio do terraforme; o do AI Memory é gerado com `ai-memory install-hooks --agent
  opencode2` quando falta, e o MCP vai pela ponte `ai-memory mcp-bridge`.
- `terraforme config --agent` (ou `--agente`) escolhe o agente padrão: sem valor, lista os agentes
  (com versão) e pergunta qual usar, por número ou nome. `terraforme cli --agent <agente>` troca o
  agente só numa sessão.
- `terraforme extension <nome> --setup` refaz a configuração da extensão; no AI Jail, confere a
  versão e o `bwrap` e cria o `~/.ai-jail` ou acrescenta nele a seção dos agentes que faltam.
- `terraforme extension ai-jail --project` cria o `.ai-jail` da pasta (`ai-jail --init`): o
  `terraforme cli` aberto nela roda no AI Jail mesmo com a sandbox desligada por padrão.
  `terraforme extension ai-jail --default true|false` liga ou desliga a sandbox por padrão.

### Alterado
- `terraforme config --provider` virou `--agent` (o nome antigo continua aceito).
- O `~/.ai-jail` padrão ganha a seção `[commands.opencode]`, com `opencode.ai` e `models.dev`
  liberados na rede filtrada.
- A aba AI Jail mostra a sandbox (`--dry-run`) do agente em uso, não mais sempre a do Claude.

## 0.6.1 - 2026-10-18

### Corrigido
- O atalho direto para as sub-abas do terminal do `terraforme cli` passa de `Alt+1`…`Alt+9` para
  `Shift+Alt+1`…`Shift+Alt+9`, com rótulo `⇧⌥N` (o `Alt+dígito` fica para o agente). `Ctrl+←→` /
  `Ctrl+PgUp/PgDn` continuam para a vizinha (em volta).

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
