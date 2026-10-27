# Changelog

Todas as mudanças relevantes do terraforme-cli. O formato segue o
[Keep a Changelog](https://keepachangelog.com/pt-BR/1.1.0/) e o projeto usa
[Versionamento Semântico](https://semver.org/lang/pt-BR/).

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

