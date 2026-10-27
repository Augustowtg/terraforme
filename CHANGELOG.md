# Changelog

Todas as mudanças relevantes do terraforme-cli. O formato segue o
[Keep a Changelog](https://keepachangelog.com/pt-BR/1.1.0/) e o projeto usa
[Versionamento Semântico](https://semver.org/lang/pt-BR/).

## [0.8.17] - 2026-10-27

### Adicionado

- Binários compilados na release: o workflow `release` do GitHub Actions compila o `terra` para Linux (`terra-x86_64-unknown-linux-gnu`) e Windows (`terra-x86_64-pc-windows-msvc.exe`) a cada tag `vX.Y.Z` e anexa os dois à release da tag, que ele cria com as notas do CHANGELOG se ainda não existir. Pelo Actions, dá para rodar o workflow numa tag já publicada.

### Alterado

- Instalação direto do GitHub sem o Rust: o `install.sh` baixa a última release (antes, a `main`) e o binário compilado dela para o sistema, e o `install-linux.sh` e o `install-win.sh` instalam esse binário em vez de compilar (o passo vira "Usar o binário compilado da release"). Sem binário (tag antiga, branch ou outro processador), compilam com o `cargo`, como antes.
- O instalador pula o RTK quando não há `cargo`, com um aviso no fim, em vez de falhar.

## [0.8.16] - 2026-10-27

### Corrigido

- Realce de código: em Rust, `'\n'` passava por lifetime e não ficava com a cor de string; no shell, `$#` e `${#x}` viravam comentário até o fim da linha; um bloco de linguagem desconhecida ganhava cor nos números (agora fica sem realce, como antes).
- `install.ps1`: sem o Git Bash, o script não fecha mais a janela do PowerShell (`return` em vez de `exit`, que no `irm | iex` encerrava a sessão).

## [0.8.15] - 2026-10-27

### Adicionado

- Instalação direto do GitHub, sem clonar o repositório: `curl -fsSL https://raw.githubusercontent.com/Augustowtg/terraforme/main/install.sh | bash` (Linux ou Git Bash) e `irm https://raw.githubusercontent.com/Augustowtg/terraforme/main/install.ps1 | iex` (PowerShell, que roda o instalador no Git Bash). O `install.sh` baixa o código de `TERRAFORME_REF` (padrão: `main`) e chama o `install-linux.sh` ou o `install-win.sh`, com as teclas lidas do terminal; `bash -s -- -y` instala sem a pergunta.

## [0.8.14] - 2026-10-27

### Adicionado

- Chat: os blocos de código cercados por ``` saem realçados, do mesmo jeito que os blocos `md` saem estilizados: palavras-chave, tipos, funções, strings, números e comentários ganham cor conforme a linguagem do canto da caixa (`rust`, `py`, `ts`/`js`, `go`, `c`/`java`..., `sh`, `sql`, `json`, `toml`/`yaml`). Sem linguagem conhecida, o bloco fica como antes.

## [0.8.13] - 2026-10-27

### Adicionado

- Uma skill escolhida na lista da `/` vira um badge no input, como o arquivo do `@` (`/docs`), para escrever o pedido depois dela; o Backspace apaga o badge inteiro.

### Corrigido

- As skills e os servidores MCP do usuário não ficavam ativos no chat: o `claude` rodava com `--disable-slash-commands` e `--strict-mcp-config`, que só deixavam os MCP das extensões. Agora o chat usa o ambiente do Claude Code, como o `claude` interativo.

## [0.8.12] - 2026-10-27

### Adicionado

- Orquestrador: o agente principal delega e recebe de volta. A ferramenta `chat` do MCP do terra passa a retornar o `chat_id` da delegação, e duas ferramentas novas fecham o ciclo:
  - `wait`: o principal espera o chat delegado; volta com a resposta final dele (`done`, em `reply`), com os pedidos de permissão pendentes dos chats que ele delegou (`permission`) ou, sem novidade no tempo pedido, com `running`.
  - `decide`: o principal aprova ou nega cada pedido (ao negar, a mensagem vai para o agente delegado). Só quem delegou decide.
- Os chats delegados pedem permissão ao principal: o agente deles usa a ferramenta `permission` do MCP do terra como `--permission-prompt-tool`, e o pedido fica esperando a decisão (até 10 min; sem decisão, é negado). No Yolo, nada é pedido.
- A primeira mensagem do chat delegado diz que a última resposta volta para quem delegou, e a ferramenta `spaces` mostra o `id` da delegação e a última resposta (`reply`) dos chats delegados.

### Alterado

- O MCP do terra atende as ferramentas que esperam (`wait` e `permission`) cada uma numa thread, sem travar as outras requisições.

## [0.8.11] - 2026-10-27

### Adicionado

- Orquestrador: os chats abertos por um agente (a ferramenta `chat` do MCP do terra) começam em **Accept Edits**, para o agente delegado editar sem parar a cada arquivo. A seção MCP das Configurações ganha **Modo dos chats** para escolher outro modo (Do provedor, Agente, Yolo, Manual, Accept Edits ou Plan Mode); fica no `~/.terraforme/config.json` (`"mcp": {"spawn_mode": "accept_edits"}`).

## [0.8.10] - 2026-10-27

### Adicionado

- Chat: todos os comandos da `/` do Claude passam a rodar pelo terra. Os que só existem no modo interativo do Claude (ex.: `/hooks`, `/permissions`, `/plugin`, `/resume`, `/memory`, `/agents`...) abrem a interface original do Claude, fora da tela do terra, na mesma sessão do chat (`--resume`) e com o comando (e os argumentos) já rodando; ao sair, o terra volta, avisa no histórico e recarrega os comandos e as skills.
- Adapters: o contrato ganha `command_route()`, que diz se um comando vai para o agente, abre a interface original ou não roda, e a `Interactive` ganha a sessão e o comando.

### Removido

- Os comandos de pagamento e plano do Claude (`/upgrade`, `/rate-limit-options`, `/passes`, `/stickers`, `/extra-usage`, `/usage-credits`) saem da lista da `/`; escritos à mão, o terra avisa que não os roda.

## [0.8.9] - 2026-10-27

### Adicionado

- Configurações: a seção Skills, embaixo da seção MCP, mostra em Instaladas as skills do Claude: as do usuário (`~/.claude/skills`, ou a pasta da conta escolhida), as do projeto (`.claude/skills` do workspace) e as dos plugins instalados para o usuário ou para o workspace (ex.: `postman:api-mocking`). `←`/`→` passam de uma para outra, com a origem e a descrição de cada uma.
- Adapters: o contrato do provedor ganha `skills()`, e o do Claude lê as skills de um módulo próprio (`claude/skills.rs`), no mesmo molde do dos servidores MCP.

## [0.8.8] - 2026-10-27

### Corrigido

- Consumo: a aba e a barra de consumo paravam no erro **HTTP 429** do Claude. Quando uma consulta falhava, a janela já zerada continuava na tela e o terra consultava de novo a cada atualização da tela, sem parar. Agora a zerada só dispara uma nova consulta uma vez e, com erro, a próxima tentativa fica para o minuto seguinte.
- A aba e a barra de consumo passam a dividir a mesma consulta: uma resposta vale por 50 s para as duas, e só uma consulta por conta fica em andamento por vez.
- Depois de um 429, o terra para de consultar por 1 min, e a espera dobra a cada 429 seguido, até 15 min. Nesse meio-tempo, a aba avisa quando tenta de novo.

## [0.8.7] - 2026-10-27

### Adicionado

- Configurações: a seção MCP mostra, em Conectados, os servidores MCP conectados ao Claude (do usuário e os locais, do `.claude.json`, e os do projeto, do `.mcp.json`) e os que o terraforme acrescenta (ex.: o `ai-memory` da extensão e o `terra`). `←`/`→` passam de um para outro, com a origem e o comando ou o endereço de cada um.

## [0.8.6] - 2026-10-27

### Adicionado

- Chat: as respostas do agente aparecem com o markdown estilizado (títulos, listas, citações, **negrito**, *itálico* e `código`, sem os marcadores). Os blocos cercados por ``` viram uma caixa de borda arredondada, igual à do input, com a linguagem (`md`, `rust`...) no canto superior esquerdo; num bloco `md`, o conteúdo também sai estilizado. A borda não entra no texto copiado.

## [0.8.5] - 2026-10-27

### Adicionado

- Orquestrador: cada terra manda ao MCP os chats abertos (o Principal e os da barra de chats) com o consumo de cada um, e a ferramenta `spaces` passa a mostrá-los: `tokens` (entrada e saída), `cache_tokens` (criação e leitura do cache) e `busy` (se o agente ainda está respondendo). Assim um agente consegue medir quanto um chat aberto pelo orquestrador gastou.

## [0.8.4] - 2026-10-27

### Corrigido

- A barra de consumo (o limite da sessão de 5 horas) volta a aparecer nos chats abertos com `Alt+N` ou pelo orquestrador: eles recebem o consumo da sessão ao abrir e a cada atualização, e as mudanças da Config passam a valer para todos os chats, não só para o do terra.

## [0.8.3] - 2026-10-27

### Adicionado

- Aba Chat: `Alt+N` abre um chat novo e vazio no mesmo terra, com as configurações e extensões do workspace, e já passa para ele. Ele aparece na barra de chats (**Chat 2**, **Chat 3**...) e fecha com `Alt+W`, como os abertos pelo orquestrador.

## [0.8.2] - 2026-10-27

### Adicionado

- Consumo geral ao lado esquerdo do "Sair": a soma dos tokens de todos os chats deste terra (o do terra, os abertos pelo orquestrador e os já fechados). Conta como o consumo embaixo do input: entrada e saída, sem os tokens de cache.
- Config: em Consumo, a opção "Consumo geral" (Visível/Oculto) mostra ou esconde esse consumo. Por padrão fica visível.

## [0.8.1] - 2026-10-27

### Corrigido

- Orquestrador: um MCP de outra versão aberto na porta 30303 (o terra foi atualizado com ele rodando) é fechado e aberto de novo com a versão atual. Antes, o MCP antigo continuava respondendo e o `/mcp` do Claude dava "endpoint not found".

## [0.8.0] - 2026-10-27

### Adicionado

- Orquestrador: o agente do chat recebe o MCP do terra e ganha a ferramenta **`chat`**, que abre um chat novo num terra aberto (por padrão, no próprio; ou num outro, pelo `pid` ou pelo nome do workspace) e manda a primeira mensagem, dizendo de qual workspace ela veio. Assim o chat A fala com um chat B.
- Aba Chat: os chats abertos pelo orquestrador aparecem numa barra de chats embaixo da barra de abas, no mesmo tema (**Principal**, depois cada chat novo). `Alt+←/→` troca de chat e `Alt+W` fecha o chat aberto pelo orquestrador.
- Extensões: um servidor MCP pode ser HTTP (com `url`), além de stdio.

## [0.7.3] - 2026-10-27

### Alterado

- Aba Orquestrador: passa a ficar entre as abas Extensões e Configurações (antes ficava depois das Configurações), e o `Tab` segue essa ordem.

## [0.7.2] - 2026-10-27

### Alterado

- Aba Orquestrador: a lista passa a se chamar **Sessões** e agrupa os terra abertos pelo workspace. Embaixo de cada space aparece cada terra aberto nele, com o pid e o estado (**sessão ativa** ou **terra cli aberto**), os com sessão ativa primeiro; ao rolar, o space escolhido aparece junto com os terra dele.

### Removido

- Aba Orquestrador: a contagem de terra ao lado do estado do space (ex.: `terra cli aberto (2)`), agora que cada terra aparece embaixo dele.

## [0.7.1] - 2026-10-27

### Alterado

- A aba **MCP** passa a se chamar **Orquestrador** e continua ao lado da aba de configurações; o título dela também é `Orquestrador · http://127.0.0.1:30303/mcp`.
- A aba **Config** passa a se chamar **Configurações**, assim como os textos que apontam para ela (no `/model`, na aba Extensões e na Interface Original do provedor).

## [0.7.0] - 2026-10-27

### Adicionado

- **MCP do terra**: um servidor MCP (JSON-RPC 2.0 por HTTP) em `http://127.0.0.1:30303/mcp`, um só para a máquina, que orquestra os terra abertos. Roda num processo próprio, `terra mcp` (o subcomando novo), que continua rodando depois que o terra fecha. Fala o `initialize`, o `ping`, o `tools/list` e o `tools/call` do MCP, com a ferramenta `spaces` (os terra abertos), e os métodos do terra: `terra/announce`, `terra/leave`, `terra/spaces` e `terra/shutdown`.
- Config: a seção **MCP**, embaixo do Consumo e separada da seção Agente, com o **Servidor** (Ativado ou Desativado, **desativado** por padrão) e o endereço. Com o MCP online, ela mostra uma **bola verde** (`MCP ●`). É da máquina, e não do workspace: fica no `~/.terraforme/config.json` novo (`{"mcp": {"enabled": true}}`).
- Com o MCP ativado, o `terra cli` verifica ao iniciar se já há um MCP na porta 30303: se houver, não abre outro; se não, abre um num processo novo. Cada terra se registra nele e avisa, a cada 2 segundos, o workspace e se há uma sessão ativa com o agente; ao sair, sai da lista (e, sem avisar por 6 segundos, também sai). Ativar na Config abre o MCP na hora, e desativar fecha ele para todos os terra.
- A aba **MCP**, o orquestrador, depois da Config e só com o MCP online: os spaces (os workspaces salvos e os abertos em algum terra) com o que há em cada um: **sessão ativa** (bola verde), **terra cli aberto** (amarela) ou **fechado** (vazia), e quantos terra estão abertos quando há mais de um. `↑`/`↓` escolhem; a lista se atualiza a cada segundo.

## [0.6.5] - 2026-10-27

### Adicionado

- Instaladores: o passo **Desinstalar o terraforme antigo**, que só aparece com o terraforme de antes do terra na máquina (o `terraforme`, o `terraforme-mcp` ou o `terraforme-pet` no `PATH` ou em `~/.local/bin`). Pergunta SIM ou NÃO (com `-y`, desinstala); com SIM, encerra o daemon e o mascote (no Windows, também o `terraforme`, porque um `.exe` aberto não pode ser apagado), remove os binários e guarda a config e os dados com o sufixo `.antigo-<data>`, em vez de apagar. O RTK, o ai-memory e o `~/.terraforme` do terra ficam.
- `install-ui.sh`: `ui_ask_step`, um passo que pergunta SIM ou NÃO e, com SIM, roda como o `ui_step`.

## [0.6.4] - 2026-10-27

### Adicionado

- Instaladores: antes de perguntar, verificam quais extensões já estão instaladas (o `rtk` e o `ai-memory` no `PATH`, como o terra as detecta). No menu, as instaladas aparecem com uma **bola verde** no lugar da caixa (`● 1. RTK - descrição (instalada)`), não podem ser marcadas e o `↑`/`↓` pula por elas; as outras continuam com a caixa para marcar.
- `install-ui.sh`: `UI_INSTALLED`, as opções do `ui_select` que já estão instaladas, e `ui_select_next`, que acha a próxima opção que pode ser marcada.

### Alterado

- Com todas as extensões instaladas, o instalador não pergunta "Instalar as extensões?": o passo nem entra na lista, e o resumo final mostra `Extensões já instaladas: RTK e ai-memory`. Com `-y`, só as que faltam são instaladas.

## [0.6.3] - 2026-10-27

### Alterado

- A barra de consumo vem **oculta** por padrão, como a de contexto: sem `agent.usage.display` no `config.json`, o Display da seção Consumo da Config fica em Oculto (e o terra não busca o consumo para ela). Para vê-la, troque para Visível.

## [0.6.2] - 2026-10-27

### Adicionado

- Instaladores: depois do **SIM** em "Instalar as extensões?", um menu no estilo da lista de extensões da Config (`▌ [x] 1. RTK - descrição`), com todas marcadas, para escolher quais instalar: `↑`/`↓` andam, `Espaço` marca ou desmarca, `Enter` confirma. Com `-y`, instala todas.
- `install-ui.sh`: `ui_select`, o menu de várias opções (as marcadas ficam em `UI_SELECTED`), e `ui_add_step`, que acrescenta um passo no fim da lista.
- **Barra de consumo** no chat, à esquerda da barra de contexto: quanto resta da sessão de 5 horas do provedor e quando ela zera (ex.: `58% · reseta 15:00`). A cor vai pelo que resta: verde, laranja de 50% para baixo, vermelho de 25% para baixo e cinza no 0%. É carregada só com ela visível (sem os tokens por dia da aba Consumo), a cada minuto, ao trocar de conta, depois de um login e depois de a sessão zerar.
- Seção **Consumo** na aba Config, embaixo do Contexto, com o **Display** da barra de consumo: Visível (o padrão) ou Oculto. Fica em `agent.usage.display` no `config.json`.
- `UsageWindow.session`, que marca a janela da sessão (a `five_hour` do Claude), e `Usage::session()`.

### Alterado

- Os passos **Instalar o RTK** e **Instalar e configurar o ai-memory** só entram na lista do instalador depois do SIM, e só os das extensões escolhidas no menu.
- Config, seção Extensões: as extensões que não estão instaladas aparecem como **indisponível** (`(indisponível)` no fim da linha da lista e `indisponível (não instalada)` nos detalhes da que está em foco).

### Removido

- Instaladores: os passos das extensões como pulados (`– passo (pulado)`) com o NÃO, e o `ui_skip` do `install-ui.sh`, que não é mais usado.

## [0.6.1] - 2026-10-27

### Adicionado

- Os instaladores (`install-linux.sh` e `install-win.sh`) perguntam, depois de instalar o terra, se é para instalar as extensões. Com **SIM** (ou com `-y`), dois passos novos: **Instalar o RTK** (`cargo install --git https://github.com/rtk-ai/rtk rtk`, se ainda não estiver no `PATH`) e **Instalar e configurar o ai-memory** (o `install.sh` oficial no Linux, o `install.ps1` pelo PowerShell no Windows, e depois o `ai-memory setup --yes`, que deixa o servidor rodando como serviço do usuário e liga os agentes detectados). Com **NÃO**, os dois aparecem como pulados.
- `install-ui.sh`: `ui_ask`, um passo com pergunta SIM/NÃO em que o NÃO não cancela a instalação, e `ui_skip`, que marca um passo como pulado (`– passo (pulado)`).

## [0.6.0] - 2026-10-27

### Adicionado

- **Extensões**: aplicações secundárias que melhoram o agente do chat. Cada uma é uma estratégia (padrão **Strategy**, o trait `Extension`) registrada num **Registry**: diz se está instalada, o que acrescenta ao agente (hooks e servidores MCP) e o relatório que mostra. Para adicionar uma, basta implementar o trait e incluí-la em `extension::extensions::all`. As primeiras:
  - **[RTK](https://github.com/rtk-ai/rtk)**: o hook `PreToolUse` do Bash (`rtk hook claude`) enxuga a saída dos comandos antes de ela chegar ao agente.
  - **[ai-memory](https://github.com/akitaonrails/ai-memory)**: o servidor MCP e os hooks do ciclo de vida que o próprio `ai-memory` informa para o Claude Code, para a memória de longo prazo entre sessões.
- Seção **Extensões** na aba Config, à direita da seção Agente (embaixo, em telas estreitas): a lista das extensões no estilo da lista de modelos (`1. RTK - descrição`), com `[x]` nas ativas, **paginada** (5 por página, `←`/`→` trocam de página). `Espaço` (ou `Enter`) ativa ou desativa a extensão; `Shift+Tab` alterna entre o Agente e as Extensões. As ativas ficam em `extensions.enabled` no `config.json` do workspace (sem a chave, nenhuma). Uma extensão que não está instalada não pode ser ativada.
- Aba **Extensões** na barra de abas, entre Consumo e Config: a lista das extensões (ativa, inativa ou não instalada) e o relatório da escolhida com `↑`/`↓`: no RTK, os tokens economizados (total, economia média, comandos) e o gráfico dos tokens economizados por dia; no ai-memory, páginas, sessões, observações, tamanho do banco e o estado dos embeddings e do LLM. Carrega ao abrir a aba, atualiza a cada minuto com ela aberta e com `r`.

### Alterado

- Com extensões ativas, as mensagens do chat levam os servidores MCP delas (`--mcp-config`, ainda com o `--strict-mcp-config`) e os hooks delas (`--settings`). Com uma extensão de hooks ativa, o `disableAllHooks` sai, porque ele desliga até os hooks do `--settings`: os hooks do Claude Code do usuário também passam a rodar. A Interface **Original do provedor** também abre com as extensões ativas.

## [0.5.11] - 2026-10-27

### Alterado

- A **barra de contexto** (canto inferior direito) não conta mais os **tokens de cache** (criação e leitura): ela soma só a entrada e a saída da última chamada ao modelo, como o consumo. A **Auto compactação** usa o mesmo número.

## [0.5.10] - 2026-10-27

### Corrigido

- A **barra de contexto sumia depois de compactar** (com o `/compact` ou a Auto compactação), mesmo com o Display visível: o terra esperava o tamanho novo na próxima mensagem. Agora ela fica e mostra na hora o contexto que sobrou, informado pelo `claude` no fim da compactação.

## [0.5.9] - 2026-10-27

### Adicionado

- **Argumentos nos comandos do padrão do router**: na lista da `/`, o `Tab` num comando sem interface no terra completa o input com o comando e um espaço (ex.: `/config `), para escrever os argumentos e mandar com `Enter` (ex.: `/config model=opus`, que muda a configuração do Claude). O `Enter` na lista continua rodando o comando na hora, sem argumentos. Nos comandos com interface no terra, `Tab` e `Enter` continuam rodando a interface.

### Alterado

- Escrito à mão e enviado sem argumentos, um comando com interface no terra (ex.: `/clear `, com espaço no fim) roda a interface, em vez de ir para o agente.
- Um comando com `/` enviado pelo input não arma mais a **Auto compactação**: ela é para as mensagens, não para os comandos.

## [0.5.8] - 2026-10-27

### Alterado

- Os comandos da `/` que **não têm interface no terra** passam a usar o **padrão do router** (o provedor): escolhidos, eles vão para o agente na sessão atual, e a resposta dele aparece no chat. Por exemplo, o `/context` mostra o uso do contexto e o `/cost` mostra os limites do Claude, e as skills também rodam. Os que só existem no modo interativo do Claude Code (ex.: `/status`, `/help`) respondem que não estão disponíveis. Antes, o terra só avisava que o comando não tinha interface.
- Na lista da `/`, esses comandos continuam esmaecidos, agora com "padrão do router" no lugar de "sem interface no terra".
- Com o agente respondendo, um desses comandos pede para esperar o fim da resposta.

### Corrigido

- O **`/compact`** não funcionava: o `claude` rodava com `--disable-slash-commands` e respondia "/compact isn't available in this environment". As mensagens que começam com `/` agora rodam com os comandos ligados. As outras continuam sem eles, para o contexto fixo não crescer.

## [0.5.7] - 2026-10-27

### Adicionado

- **`↑`/`↓` no input do chat** (escrevendo) mostram as mensagens já enviadas, da mais nova para a mais antiga, como num terminal. Valem as de todas as sessões desde que o terra abriu, inclusive depois do `/clear`, e as menções de arquivo voltam como badges. O `↓` depois da mais nova volta ao texto que estava sendo escrito, e editar uma mensagem recuperada a transforma no texto em edição. A mesma mensagem enviada duas vezes seguidas aparece uma vez só.
- Com o texto em várias linhas, `↑`/`↓` primeiro andam entre as linhas, mantendo a coluna do cursor, e só mudam de mensagem na primeira ou na última linha.

## [0.5.6] - 2026-10-27

### Alterado

- Com a **Auto compactação** ligada, o terra compacta a conversa **no máximo uma vez por mensagem enviada pelo usuário**: só no fim da resposta a uma mensagem, nunca no fim de um `/compact` (pedido à mão ou automático, mesmo que ele falhe) e não quando o próprio agente já compactou a conversa naquela resposta. Interromper a resposta também cancela a auto compactação dela.

## [0.5.4] - 2026-10-27

### Alterado

- A aba **Consumo** mostra **quanto falta** de cada limite, e não mais quanto foi usado: a barra vem cheia e esvazia conforme o uso, com a porcentagem que resta (ex.: `58% restantes`). As cores continuam as mesmas: verde, **amarela** quando restam 30% ou menos e **vermelha** quando restam 10% ou menos.
- O **Crédito extra (mensal)** mostra o valor que resta no mês (ex.: `US$ 38,22 restantes de US$ 40,00 no mês`), no lugar do valor gasto.

## [0.5.3] - 2026-10-27

### Alterado

- Novo padrão da seção Contexto (para workspaces sem a configuração no `config.json`): **Display Oculto**, **Auto compactação Não**, **Limite de tokens 200K** e **Compactar em 75%** (150K tokens). Antes, a barra vinha visível e o limite era o do modelo.
- No `config.json`, `agent.context` sem a chave `limit` fica no padrão (200K); `"limit": null` continua sendo **Do modelo**. Workspaces já salvos mantêm o que está gravado neles.

## [0.5.2] - 2026-10-27

### Adicionado

- **Limite de tokens** na seção Contexto da aba Config, entre Auto compactação e Compactar em: **Do modelo** (o padrão, mostrado com o limite informado pelo agente, ex.: `Do modelo · 200K`) ou um limite fixo (50K, 100K, 150K, 200K, 250K, 300K, 400K, 500K, 750K ou 1M). O limite escolhido substitui o do modelo na barra de contexto e no ponto de compactar (ex.: 75% de 100K = 75K). Salvo em `agent.context.limit` no `config.json` (`null` é o do modelo); editando o arquivo, qualquer número de tokens vale.

## [0.5.1] - 2026-10-27

### Adicionado

- Seção **Contexto** na aba Config, dentro da seção Agente e separada dela por uma linha (que vai só até onde vai a seção, não a tela inteira), salva no `config.json` do workspace (`agent.context`):
  - **Display**: **Visível** (padrão) ou **Oculto**, para mostrar ou esconder a barra de contexto;
  - **Auto compactação**: **Não** (padrão) ou **Sim**; ligada, o terra manda o `/compact` sozinho quando uma resposta deixa o contexto no ponto de compactar (uma vez por resposta: se a compactação não reduzir o contexto, ele não tenta de novo em loop);
  - **Compactar em**: o ponto de compactar, de 50% a 95% do limite do modelo (padrão **75%**), mostrado também em tokens (ex.: `75% · 150K tokens`).

### Alterado

- A barra de contexto vai até o **ponto de compactar** (e não até o limite do modelo): fica **laranja** na metade dele, **vermelha** a partir de 75% dele e mostra **Compacte a conversa** ao chegar nele. O texto da barra mostra o contexto sobre o ponto (ex.: `45K / 150K (30%)`).
- A coluna dos nomes da aba Config ficou 2 colunas mais larga, para caber "Auto compactação".

## [0.5.0] - 2026-10-27

### Adicionado

- **Barra de contexto** no painel do chat, na mesma linha dos badges de Modo e Consumo, no canto inferior direito (embaixo do status do agente). Ela mostra o tamanho do contexto da sessão com o número dentro da barra (ex.: `45K / 200K (22%)`) e muda de cor conforme ele se aproxima do limite do modelo: **verde**, **laranja** a partir da metade e **vermelho** a partir de 75%. No limite, passa a mostrar **Compacte a conversa**. É só um indicativo: aparece depois da primeira resposta da sessão e some no `/clear` ou ao trocar de conta.
- O limite de contexto vem do próprio agente (o `contextWindow` do modelo, no fim de cada resposta); até ele chegar, vale 200K.
- **`/compact`** passa a ter interface no terra: pede ao agente para resumir a conversa na mesma sessão, liberando o contexto. Ao terminar, o chat avisa "Conversa compactada." e a barra volta a crescer a partir da próxima resposta.

## [0.4.20] - 2026-10-27

### Corrigido

- O `terra cli` **sempre abre**, mesmo com problemas:
  - um **panic no adapter do provedor** (em qualquer chamada, ao detectar o provedor ou numa resposta) não derruba mais a tela: vira o valor padrão (ex.: sem contas) ou um erro no chat, e o provedor que não pôde ser criado fica **Desconectado** com o motivo;
  - um panic numa **thread de fundo** do provedor (catálogo, consumo, leitura da resposta) não restaura mais o terminal no meio da tela: vira um erro no chat e encerra a resposta em andamento;
  - sem conseguir abrir o `terra.sqlite`, ler o diretório atual ou achar o home, o terra abre do mesmo jeito (banco em memória, configurações numa pasta temporária) e mostra o problema no chat. Antes, ele saía com `erro:`.

## [0.4.19] - 2026-10-27

### Alterado

- No instalador, o resumo de um passo aparece **só quando o instalador chega nele** (●) e some quando ele termina (✔). Os passos que ainda não começaram (○) ficam só com a label.

## [0.4.18] - 2026-10-27

### Adicionado

- O instalador (`install-linux.sh` e `install-win.sh`) mostra, em cinza ao lado de cada passo, um **resumo do que ele faz** (ex.: `cargo build --release`, o binário copiado, os arquivos de shell alterados). O resumo some quando o passo termina (✔); passos sem resumo, como a confirmação, ficam só com a label.

## [0.4.17] - 2026-10-27

### Alterado

- Trocar a Interface para **Agente Terraforme** não fecha mais o `claude` da Interface **Original do provedor** quando o cliente **já enviou alguma mensagem** nele: a sessão continua rodando e reaparece, como estava, ao voltar a Interface. Sem nenhuma mensagem enviada, ele é fechado como antes. Sair do terra continua encerrando o processo.

## [0.4.16] - 2026-10-27

### Alterado

- Na Interface **Original do provedor**, o `claude` interativo roda **dentro da tela do terra**, na área de conteúdo da aba Chat, mantendo a **barra de abas** e o rodapé (antes, o terra saía da tela enquanto ele rodava). Ele roda num pseudo-terminal e acompanha o tamanho da tela.
  - Com o terminal em foco, todas as teclas vão para o `claude` (inclusive `Tab`, `Esc`, `Shift+Tab` e `Ctrl+C`); **`Ctrl+Q`** volta às abas e **`Enter`** na aba Chat volta ao terminal.
  - Ao sair do `claude`, a aba mostra como ele terminou; `Enter` abre de novo. Voltar a Interface para Agente Terraforme ou sair do terra encerra o processo.

## [0.4.15] - 2026-10-27

### Adicionado

- Configuração **Interface** na aba Config (salva como `"interface"` no `config.json` do workspace):
  - **Agente Terraforme** (padrão): o chat e o input do terra, como antes;
  - **Original do provedor**: a aba Chat abre a interface original do provedor (o `claude` interativo) no diretório do workspace, ao iniciar e com `Enter`, com a conta, o modelo, o esforço e o modo inicial da Config (o Yolo vira `bypassPermissions`). Ao sair dele, a tela do terra volta.

## [0.4.14] - 2026-10-27

### Alterado

- Ao iniciar, o terra lê o `config.json` do workspace; **sem ele, começa com o padrão do router** (o provedor): modo inicial **Do provedor**, modelo e esforço padrão do provedor e a conta **Default**. Antes, um workspace sem o arquivo começava com as configurações globais do `terra.sqlite`.

### Removido

- As configurações globais do `terra.sqlite` (tabela `settings`) não são mais lidas. A tabela continua no banco, sem uso.
- A linha **Salvo em** da aba Config (o caminho do `config.json`), adicionada na 0.4.12.

## [0.4.13] - 2026-10-27

### Adicionado

- Ao sair (**Sim** na confirmação) de um workspace que **não está salvo**, o terra pergunta **"Este workspace não está salvo. Deseja salvar?"** com 3 opções:
  - **Sim**: salva o diretório como workspace (com o nome da pasta, como o `terra workspace --save`) e sai;
  - **Não**: sai sem salvar; na próxima vez pergunta de novo;
  - **Não (não perguntar novamente)**: sai e não pergunta mais neste workspace (grava `"workspace": {"ask_to_save": false}` no `config.json` dele).
  - `←`/`→` escolhem, `Enter` confirma e `Esc` desiste de sair. O `Ctrl+C` continua saindo direto.

## [0.4.12] - 2026-10-27

### Alterado

- As configurações da aba **Config** (conta, modo inicial, modelo e o esforço padrão do `/model`) passam a ser de cada **workspace**, salvas em `~/.terraforme/workspace/<nome do workspace>/config.json`:
  - o nome é o do workspace salvo para o diretório ou, sem um, o nome da pasta (o mesmo padrão do `terra workspace --save`);
  - um workspace sem o arquivo começa com as configurações globais de antes (do `terra.sqlite`), que não mudam mais;
  - a Config mostra onde salva (`Salvo em ~/.terraforme/workspace/terraforme-cli/config.json`);
  - outras chaves que existirem no `config.json` são mantidas ao salvar.

## [0.4.11] - 2026-10-27

### Corrigido

- O input aceita o `I` maiúsculo (Caps Lock ou Shift) para entrar no modo de escrita, como o `i`.

## [0.4.10] - 2026-10-27

### Adicionado

- **Todos os comandos do Claude na lista da `/`** (157 no Claude Code 2.1.308, antes 53):
  - os do modo interativo que o `claude -p` não informa no `initialize`, como `/help`, `/status`, `/resume`, `/permissions`, `/memory` e `/logout` (lista em `claude/interactive.rs`);
  - as skills e os comandos de plugins que o `claude` informa (ex.: `/graphify`, `/pdf`, `/postman:api-testing`), que antes ficavam de fora;
  - os que ainda não têm interface no terra continuam esmaecidos, com "sem interface no terra".
- **`/usage` pelo terra**: abre a aba **Consumo** (limites, crédito extra e o gráfico por dia).
- O **`/login`** continua pelo terra (o da 0.4.8), agora com a descrição que vem do Claude.

### Corrigido

- Na lista da `/`, os nomes longos (ex.: `/ai-memory-durable-pages`) não grudam mais na descrição.

## [0.4.9] - 2026-10-27

### Adicionado

- Gráfico **Consumo por dia** na aba Consumo, ao lado das barras de limite e com a mesma altura da seção dos limites (embaixo delas em telas com menos de 100 colunas):
  - botões **Sem cache** (verde, o padrão) e **Com cache** (vermelho), escolhidos com o mouse (ou `c`); o escolhido fica preenchido;
  - uma barra sólida por dia dos últimos 14 dias, com o dia do mês embaixo e a de hoje em destaque;
  - o maior valor (`máx. 892.6K · últimos 14 dias`) e o resumo `Hoje 201.1K · média 196.3K/dia · total 2.7M`;
  - em telas estreitas, mostra só os dias que cabem;
  - **clicar num dia** troca o gráfico para as **horas daquele dia** (00 a 23), com o consumo do dia embaixo: `27-10-2026 · total 224.7K · pico às 10h (103.2K)`. O botão **‹ Dias** (ou `Esc`) volta aos dias; os botões de cache também valem nas horas.
- No Claude, os tokens vêm dos transcripts do Claude Code na pasta da conta (`projects/**/*.jsonl`), que incluem as conversas do terra: por dia e por hora, a entrada e a saída e, à parte, o cache (criação e leitura), no fuso local, contando cada resposta uma vez. Funciona mesmo sem login válido (é local).
- `ChatAdapter::load_daily`: os tokens por dia de cada provedor, em segundo plano.

## [0.4.8] - 2026-10-27

### Corrigido

- **`/login`** (mesma interface da 0.4.7, no terminal fora da tela do terra):
  - o **código colado não aparecia** (o prompt do `claude auth login` não mostra o que se digita) e parecia que o terminal não aceitava o código. Agora o terra pede o código com eco: o que é colado aparece, `Enter` envia, código inválido pede de novo e `Enter` vazio cancela;
  - o **link não abria inteiro** e a autorização ficava inválida. Agora o link sai como hiperlink do terminal (OSC 8), que abre o link completo mesmo quebrado em várias linhas, e já vai para a área de transferência;
  - no fim, o histórico diz se deu certo ou o motivo (ex.: `Login failed: ...`).
- A volta do login não pergunta mais a posição do cursor ao terminal (podia falhar com `The cursor position could not be read`).

## [0.4.7] - 2026-10-27

### Adicionado

- **`/login`** no chat, para entrar de novo na conta escolhida na Config quando o token expira ou é revogado (ex.: `Failed to authenticate: OAuth token revoked. Please log in again`):
  - o terra sai da tela e roda o `claude auth login` na pasta da conta (`CLAUDE_CONFIG_DIR`), com o e-mail dela já preenchido; o login abre o navegador e, se pedir, o código é colado no terminal;
  - no fim, a tela volta, o histórico diz se o login deu certo e os comandos, os modelos e o consumo são recarregados;
  - o `/login` é um comando do terra: aparece na lista da `/` mesmo sem vir do agente (`ChatAdapter::login`).
- Quando o agente falha por autenticação, o chat sugere: `Use /login para entrar de novo na conta ...`.

### Alterado

- Na aba Consumo, o aviso de login expirado ou revogado agora indica o `/login`.

## [0.4.6] - 2026-10-27

### Adicionado

- **Crédito extra** na aba Consumo: quando a conta tem o crédito extra ativo, aparece a barra **Crédito extra (mensal)** com a porcentagem do limite mensal gasta e o valor embaixo (ex.: `US$ 1,78 de US$ 40,00 usados no mês`, e `· limite atingido` quando chega ao limite). Sem crédito extra ativo, a barra não aparece.

## [0.4.5] - 2026-10-27

### Adicionado

- Aba **Consumo** (antes "Em construção"):
  - **Route**: o provedor e a conta do chat, com o plano (ex.: `Claude · Default · eu@gmail.com · Pro`);
  - uma **barra de 0 a 100%** por limite de uso, com a porcentagem: verde, amarela a partir de 70% e vermelha a partir de 90%;
  - embaixo de cada barra, quando ela zera: `Reseta DD-MM-YYYY HH:MM · zera em 2h 13m` (no fuso local);
  - carrega ao abrir a aba, atualiza a cada minuto com ela aberta (e quando um limite zera ou a conta muda) e com `r`.
- No Claude, os limites são os do `/usage` do Claude Code (Sessão de 5 h, Semana e, quando o plano tem, Opus e Sonnet), do endpoint de uso (`/api/oauth/usage`) com o token da conta escolhida. Não chama o modelo nem gasta tokens; a chamada usa o `curl`, com o token pelo stdin (fora da linha de comando).
- `ChatAdapter::load_usage`: o consumo de cada provedor, em segundo plano.
- Dependência `jiff`, para as datas no fuso local.

## [0.4.4] - 2026-10-27

### Adicionado

- Interface do terra para o **`/model`**, com a lista de modelos e os níveis de esforço vindos do agente (catálogo do adapter):
  - **Selecione o modelo** (título em amarelo), uma descrição e a lista com o item em foco em verde;
  - **Default (modelo padrão atual)**: `Enter` abre o mesmo menu com o título **Selecione o modelo default**, que salva o modelo e o esforço padrão nas configurações (a Config passa a mostrar esse modelo);
  - os modelos numerados no formato `1. Opus - 5.5 - descrição`, com nome, versão e descrição informados pelo Claude; até 5 visíveis, com `... N modelos disponíveis`;
  - **Esforço**: barra verde com os níveis que o modelo em foco aceita (cada modelo tem os seus; alguns não têm); `←`/`→` ajustam;
  - `Enter` num modelo troca o modelo e o esforço desta conversa; `Esc` volta do menu do default ou fecha.
- O agente recebe o esforço com `--effort` (`Request.effort`); o esforço padrão fica salvo (`agent.effort`).
- A Config lista os modelos do catálogo do agente (antes, só Opus, Sonnet e Haiku).

## [0.4.3] - 2026-10-27

### Adicionado

- Comandos com `/` no input, como no Claude Code:
  - a **lista vem do agente**: o adapter carrega em segundo plano o catálogo do provedor (`ChatAdapter::load_catalog`). O Claude informa os comandos embutidos dele pelo pedido `initialize` do protocolo do Agent SDK, em ~1 s e sem chamar a API (não gasta tokens); trocar de conta recarrega;
  - a **função de cada comando é uma interface do terra**: a primeira é o `/clear`, que começa uma conversa nova. Os outros aparecem esmaecidos, com "sem interface no terra", e avisam no histórico se escolhidos;
  - a lista abre acima do input (no mesmo estilo da lista de arquivos do `@`) só quando a `/` está no começo do texto, com `/nome` e a descrição; o que vem depois da `/` filtra (nome, depois descrição, com os que têm interface primeiro); `↑`/`↓` escolhem, `Tab`/`Enter` executam e `Esc` fecha.
- O catálogo também traz os modelos do provedor, com descrição e níveis de esforço (usados pelo `/model`).

### Alterado

- As listas que abrem acima do input (`@` e `/`) usam o mesmo widget (`src/tui/widgets/popup.rs`).

## [0.4.2] - 2026-10-26

### Adicionado

- Configuração **Conta** na seção Agente da aba Config, para trocar a conta do Claude usada pelo chat:
  - **Default**: o `claude` usa a conta padrão dele (sem `CLAUDE_CONFIG_DIR`); o rótulo mostra o e-mail dela;
  - **Pessoal** e **Trabalho**: descobertas nas pastas `~/.claude` e `~/.claude-*` que têm credenciais. O tipo vem da organização (a pessoal da Anthropic se chama "…'s Organization") e o rótulo mostra o e-mail, e a organização nas de trabalho;
  - a conta escolhida vai para o `claude` como `CLAUDE_CONFIG_DIR`, e o modo "Do provedor" passa a ler o `defaultMode` dela;
  - trocar de conta com uma conversa aberta começa uma conversa nova (a sessão era da outra conta) e avisa no histórico;
  - a escolha fica salva (`agent.account`) e vale ao abrir o `terra cli` de novo.
- O contrato `ChatAdapter` ganhou `accounts()` e o `Request` ganhou `account`.

## [0.4.1] - 2026-10-26

### Adicionado

- Seção **Agente** na aba Config (antes "Em construção"), no padrão visual da aplicação (título de seção, `▌` e destaque no item em foco, legenda no rodapé):
  - **Provedor**: qual agente o chat usa e se está disponível;
  - **Modo inicial**: o modo com que o chat começa, "Do provedor" (segue o `defaultMode` do Claude, como antes) ou um modo fixo (Agente, Yolo, Manual, Accept Edits, Plan);
  - **Modelo**: Padrão ou um dos modelos do provedor (Opus, Sonnet, Haiku), enviado com `--model` nas próximas mensagens.
- `↑`/`↓` escolhem a configuração e `←`/`→` ou `Enter` trocam o valor; a mudança vale na hora para o chat (trocar só o modelo não desfaz um Shift+Tab).
- As configurações ficam salvas na tabela `settings` do `~/.terraforme/terra.sqlite`.
- O contrato `ChatAdapter` ganhou `models()` e o `Request` ganhou `model`, para cada provedor oferecer os seus modelos.

## [0.4.0] - 2026-10-26

### Adicionado

- Menção de arquivos no input com `@`:
  - digitar `@` abre, acima do input, a lista de arquivos e pastas do diretório onde o `terra cli` foi aberto, filtrada pelo que vem depois do `@` (começo do nome do arquivo, parte do nome, parte do caminho e, por último, letras na ordem);
  - a lista é atualizada em tempo real: arquivos e pastas criados, removidos ou renomeados aparecem enquanto ela está aberta (observador `notify`, com o `.gitignore` respeitado e arquivos ocultos de fora);
  - `↑`/`↓` (ou `Ctrl+P`/`Ctrl+N`) escolhem, `Tab`/`Enter` inserem e `Esc` fecha; um espaço encerra a consulta, e um `@` no meio de uma palavra (ex.: um e-mail) não abre a lista;
  - o caminho vira um badge cinza no estilo da aplicação (`@/src/main.rs`) que se comporta como um caractere só: o cursor passa por ele e o `Backspace` o apaga inteiro;
  - o agente recebe a menção como `@src/main.rs`.

### Corrigido

- Sem badges embaixo, a caixa de input ficava com só uma linha de texto e duas linhas vazias abaixo (desde a 0.3.4). Ela volta a ter as três linhas.

### Alterado

- Os testes da caixa de input foram para `src/tui/chat/input/tests.rs`, para nenhum arquivo passar de 1000 linhas.

## [0.3.9] - 2026-10-26

### Adicionado

- Padrão Adapter no chat: o chat fala com o agente só pelo contrato `ChatAdapter` (`src/agent/adapter.rs`), com `Session` (uma resposta em andamento) e `Request` (texto, modo e sessão). O Claude CLI virou o `ClaudeAdapter` (`src/agent/adapters/claude.rs`), e os provedores ficam registrados em `src/agent/adapters.rs`. Para adicionar um provedor (route), basta implementar o trait e registrá-lo, sem mudar o chat.
- Testes do fluxo completo do chat (enviar, receber, continuar a sessão, interromper) com um adapter de mentira, sem rodar o `claude`.

### Alterado

- `PermissionMode` passou a ser do domínio do agente (`src/agent/mode.rs`); a tradução para os modos do Claude (`auto`, `manual`, `acceptEdits`, `plan`) ficou no `ClaudeAdapter`, e a cor de cada modo na interface.
- O evento de início informa o modo já traduzido (`ReportedMode`: conhecido, desconhecido ou não informado).

## [0.3.8] - 2026-10-26

### Alterado

- Organização do código por funcionalidade, sem mudar o comportamento. Nenhum arquivo passa de 800 linhas (o `chat.rs` tinha 1525):
  - `agent/`: `event.rs` (eventos), `claude.rs` (processo e permissões) e `claude/stream.rs` (parser do `stream-json`);
  - `platform/`: `clipboard.rs` e `keyboard.rs` (antes `tui/clipboard.rs` e `tui/locks.rs`);
  - `tui/workspace.rs`: navegador de workspaces, que estava dentro do `tui.rs`;
  - `tui/widgets/badge.rs`;
  - `tui/chat/`: `input.rs` (caixa de input), `history.rs`, `mode.rs` (modo e status) e `tests.rs`; o `chat.rs` só liga as partes.

## [0.3.7] - 2026-10-26

### Alterado

- Yolo deixou de ser um modo do agente e passou a ser do terra: o agente roda pedindo permissão normalmente (`--permission-mode manual`), e o terra aprova automaticamente tudo o que ele pedir (`--permission-prompt-tool stdio`). Por isso o Yolo é o único modo que não é sincronizado com o agente.
- O Yolo nunca vem do agente nem dos settings: um `defaultMode` `bypassPermissions` faz o terra começar no modo padrão, Agente.
- As mensagens vão ao `claude` em `--input-format stream-json`, com o stdin aberto durante a resposta para responder aos pedidos de permissão.

## [0.3.6] - 2026-10-26

### Adicionado

- Sincronia do modo de permissão com o agente:
  - o terminal lê o modo em que o agente diz estar (no início de cada resposta) e, se não for o pedido, passa a mostrar o modo real e avisa no histórico;
  - o `terra cli` começa no modo padrão configurado no Claude (`permissions.defaultMode` em `settings.local.json`, `.claude/settings.json` do projeto ou `~/.claude/settings.json`);
  - trocar o modo com `Shift+Tab` durante uma resposta mostra `Modo: Agente → Yolo`: o agente segue no modo atual e o novo vale a partir da próxima mensagem.

### Alterado

- Com o agente ativo (respondendo ou com sessão iniciada), o badge de modo fica sempre visível, inclusive no modo padrão (Agente).

## [0.3.5] - 2026-10-26

### Alterado

- O badge de modo ganhou o prefixo "Modo:" (ex.: `Modo: Yolo`).

## [0.3.4] - 2026-10-26

### Alterado

- Os badges de baixo só aparecem quando ativos: o de modo, quando o modo não é o padrão (Agente); o de consumo, depois do primeiro token. Sem nenhum, a linha dos badges some e o espaço fica para a conversa.

## [0.3.3] - 2026-10-26

### Alterado

- Os indicadores `● Caps` e `● Insert` só aparecem na borda do input quando estão ligados (antes, ficavam cinza quando desligados).
- "Insert sobrescrever" saiu da legenda do rodapé.

## [0.3.2] - 2026-10-26

### Adicionado

- Seleção de texto no histórico do chat com o mouse: arrastar seleciona um trecho, e um duplo clique seleciona a mensagem inteira. O texto vai para a área de transferência ao soltar (sem os prefixos `⏺`/`›`), e o rodapé mostra "Copiado (N caracteres)".
- Cópia pelo `wl-copy`, `xclip` ou `xsel` no Linux; sem eles (e no Windows), pela sequência OSC 52 do terminal.
- Barra de rolagem no histórico: só aparece ao rolar, se a conversa não cabe na tela, e some 2 s depois.

### Alterado

- A roda do mouse rola a conversa em qualquer ponto da aba Chat (antes, só em cima do histórico).
- As mensagens do histórico quebram por palavra, com as continuações alinhadas ao texto.

## [0.3.1] - 2026-10-26

### Corrigido

- Consumo alto de tokens: um "oi tudo bem" gastava ~25k tokens, quase tudo contexto fixo do Claude Code carregado do ambiente do usuário (servidores MCP, lista de skills e hooks). O `claude` agora roda com `--strict-mcp-config`, `--disable-slash-commands` e hooks desligados, e a mesma mensagem fica em ~12k (−52%). As ferramentas embutidas do Claude Code continuam disponíveis.
- O badge de consumo conta só os tokens de entrada e saída; os de cache (o contexto fixo de ~12k por chamada) ficam de fora. Um "oi tudo bem" passa a aparecer como ~80 tokens.

## [0.3.0] - 2026-10-26

### Adicionado

- Chat de verdade na aba Chat, usando o Claude CLI (`claude`) por baixo:
  - `Enter` envia a mensagem; cada envio roda `claude -p --output-format stream-json` no diretório onde o `terra cli` foi aberto, e as mensagens seguintes continuam a mesma sessão (`--resume`);
  - a resposta chega em tempo real no histórico, junto com as ferramentas usadas (`⏺ Bash(ls)`) e a primeira linha do resultado (`⎿ ...`);
  - o modo de permissão do badge vai para o `--permission-mode`: Agente → `auto`, Yolo → `bypassPermissions`, Manual → `manual`, Accept Edits → `acceptEdits`, Plan Mode → `plan`;
  - status real (Desconectado sem o `claude` no PATH, Ocioso, Pensando, Erro), atividade ("Pensando...", "Usando Bash...") e modelo informado pelo Claude no título (ex.: `Opus - 5.5`);
  - o badge de consumo soma os tokens de cada resposta;
  - `Esc` no modo NORMAL interrompe a resposta em andamento;
  - histórico com rolagem por `PageUp`/`PageDown` e pela roda do mouse.

### Alterado

- O badge de consumo começa em 0 e o modelo aparece como "Modelo padrão" até a primeira resposta.

## [0.2.7] - 2026-10-26

### Adicionado

- Indicadores `● Caps` e `● Insert` na borda de baixo do input: a bola fica verde quando o Caps Lock ou o modo sobrescrever (tecla `Insert`) está ligado, e cinza quando desligado.
- Leitura do Caps Lock direto do sistema, já que o terminal não informa: no Linux, pelo LED do teclado em `/sys/class/leds`; no Windows, pela API do teclado (`GetKeyState`).

## [0.2.6] - 2026-10-26

### Corrigido

- Pular linha não funcionava no GNOME Terminal (e em outros terminais baseados em VTE): eles enviam `Shift+Enter` igual ao `Enter` e não têm o protocolo de teclado do kitty. Agora `\` + `Enter` também pula linha (a `\` vira a quebra), em qualquer terminal. `Alt+Enter` e `Ctrl+J` continuam funcionando.

## [0.2.5] - 2026-10-26

### Adicionado

- `Shift+Enter` pula uma linha no input. Nos terminais que não diferenciam `Shift+Enter` de `Enter`, `Alt+Enter` e `Ctrl+J` fazem o mesmo.
- O `terra cli` ativa o protocolo de teclado do kitty quando o terminal suporta, para reconhecer o `Shift+Enter`.

### Alterado

- O input mostra várias linhas; quando passa de três, rola para acompanhar o cursor.
- No modo sobrescrever, a quebra de linha não é substituída (as linhas não se juntam).

## [0.2.4] - 2026-10-26

### Adicionado

- Seleção do texto do input com o mouse: clicar posiciona o cursor (e começa a escrever); arrastar seleciona, com o trecho destacado.
- Com texto selecionado, digitar substitui a seleção e `Backspace`/`Delete` a apagam; as setas, `Home` e `End` desfazem a seleção.

### Alterado

- O cursor `|` passa a ser o cursor do próprio terminal em formato de barra (continua piscando). Ele fica entre dois caracteres sem ocupar uma coluna, o que permite saber exatamente onde o mouse clicou.
- O texto do input é quebrado por caractere (antes, por palavra).
- O `terra cli` captura o mouse enquanto está aberto; na maioria dos terminais, segurar `Shift` ao arrastar usa a seleção nativa do terminal.

## [0.2.3] - 2026-10-26

### Alterado

- A linha de atividade do agente (spinner com a descrição) só aparece quando o agente informar o que está fazendo. Sem agente, fica vazia.
- Título da caixa de mensagem no formato `[Router] Modelo - Versão`: o router (ex.: Claude) aparece como botão, seguido de `Opus - 5.5`.

## [0.2.2] - 2026-10-26

### Corrigido

- O status "Trabalhando" foi substituído por "Pensando".
- Sem agente conectado, o status mostrado é "Desconectado" (novo status padrão).

## [0.2.1] - 2026-10-26

### Alterado

- A caixa de mensagem não mostra mais "-- INSERT --" / "-- NORMAL --". A borda em destaque continua indicando que dá para escrever.

### Adicionado

- Tecla `Insert` alterna o modo de sobrescrever: o texto digitado substitui os caracteres à frente do cursor.

## [0.2.0] - 2026-10-26

### Adicionado

- Painel de entrada e status do agente na aba Chat (só a interface, com valores mock):
  - linha de atividade com spinner animado ("Pensando...");
  - caixa de input com o modelo no título à esquerda e o status do agente à direita (Ocioso, Trabalhando, Aguardando, Erro);
  - modos estilo Vim: NORMAL e INSERT (`i`/`a` para entrar, `Esc` para sair), com cursor `|` piscando e indicação do modo na borda;
  - badge do modo de permissão, alternado com `Shift+Tab`: Agente → Yolo → Manual → Accept Edits → Plan Mode;
  - badge de consumo em formato compacto (950, 12.3K, 1M).
- Helper de badge reutilizável e variantes de cor no tema.
- Tick de 100 ms no loop da tela principal, para as animações.

### Alterado

- O binário passa a se chamar `terra` (antes `terraforme`), para não conflitar com o `terraforme` principal. A função de shell `terra` chama `command terra`.
- O instalador do Windows remove o `terraforme.exe` antigo de `~/.terraforme/bin`.

## [0.1.2] - 2026-10-26

### Corrigido

- `Esc` no `terra cli` fechava a tela direto. Agora leva o foco ao botão vermelho **Sair**; `Enter` nele abre a confirmação Sim/Não.

## [0.1.1] - 2026-10-26

### Adicionado

- Interface nos instaladores: banner TERRAFORME, versão, barra de progresso com os passos e confirmação SIM/NÃO selecionada pelas setas.
- Opção `-y` nos instaladores para instalar sem confirmação.

## [0.1.0] - 2026-10-26

- Primeira versão: `terra cli` (abas Chat, Consumo e Config) e `terra workspace` (recentes e salvos).

