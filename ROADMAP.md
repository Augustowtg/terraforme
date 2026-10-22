# Roadmap

O que está planejado e ainda não foi entregue. Cada item aponta para a issue com os detalhes.

## MCP: agentes que conversam e se gerenciam

O daemon `terraforme-mcp` deixa as instâncias abertas pelo terraforme conversarem, abrirem e
fecharem outras instâncias e delegarem tarefas. Documentação: [docs/MCP/mcp.feature.md](docs/MCP/mcp.feature.md).

### Claude Code

- [x] `list_instances` e `send_message`: duas instâncias (no mesmo terminal, em outro terminal ou
  em outro space) conversam pelos apelidos `<space>/<agente>-<n>`.
- [x] `open_instance`: uma instância abre outra numa sub-aba do próprio terminal.
- [x] `close_instance`: uma instância fecha outra (com confirmação do usuário).
- [x] `delegate_task`: entrega uma tarefa a uma instância aberta ou a uma nova, que devolve o
  resultado com `send_message`.
- [x] `spawn_subagent`: tarefa avulsa com `claude --print`, resultado na própria chamada.
- [ ] Abrir a instância em outro terminal ou space (hoje abre no terminal de quem pediu).
- [ ] Escolher o agente, o modelo e o modo da instância aberta (hoje usa os do terminal).
- [ ] Resultado da tarefa delegada pelo hook `Stop` do Claude Code, sem depender de o agente lembrar
  de chamar `send_message`.
- [ ] Árvore de instâncias: saber quem abriu quem, fechar só as que a instância abriu e fechar as
  filhas junto com a mãe.
- [ ] Abrir, fechar e delegar pela UI web (hoje ela só manda mensagens).
- [ ] Reconectar as instâncias abertas a um daemon novo sem reiniciá-las (hoje o `--mcp-config`
  guarda a porta antiga até `⌃⌥R`).
- [ ] Unificar `spawn_subagent` com `delegate_task` (aba com `--print` que fecha sozinha).

### Outros agentes

- [ ] OpenCode: entregar o MCP `terraforme` à sessão (`Adapter::mcp_args`, hoje `None`), via
  config/`OPENCODE_CONFIG_CONTENT`, e liberar as ferramentas sem confirmação.
- [ ] OpenCode: digitar as mensagens recebidas no prompt dele (o formato de entrada da TUI do
  OpenCode é diferente do Claude Code) e `spawn_subagent` com `opencode run`.
- [ ] Antigravity (`agy`): MCP, mensagens e subagentes.
- [ ] Cursor: o adapter ainda não existe.
- [ ] Conversa entre agentes diferentes (ex.: Claude delega ao OpenCode).

## Middleware (experimental)

Com `[tui] middleware` (aba Configurações › Agente) a instância conversa com o agente por um chat
desenhado pelo próprio terraforme, em vez do terminal do agente. Especificação:
[docs/tui/agente/agente.feature.md](docs/tui/agente/agente.feature.md).

- [x] Claude Code: `claude -p --input-format stream-json --output-format stream-json`, com o texto
  da resposta em tempo real, as ferramentas chamadas e os resultados.
- [x] Mensagens do MCP (`send_message`, `delegate_task`, `open_instance`) entregues no chat.
- [x] Pedir a permissão das ferramentas no chat (`--permission-prompt-tool stdio`).
- [x] Markdown das respostas (títulos, listas, blocos de código com a cor da linguagem, paleta do
  VS-Claude).
- [x] Comandos `/` do agente e arquivos com `@` na caixa de mensagem.
- [x] Aba `Middleware` com as instâncias de todos os routers como sub-abas.
- [ ] Interromper a resposta (`Esc`) e mostrar o consumo da sessão no rodapé.
- [x] Antigravity (`agy --input-format stream-json --output-format stream-json`, sem `-p`) e
  OpenCode (`opencode acp`, Agent Client Protocol): texto em tempo real, ferramentas chamadas e
  permissão delas no chat (o Antigravity não pergunta).

## Janela flutuante com o mascote (#34)

A janela flutuante (`terraforme pet`) mostra o tatu-peba sobre as outras janelas, avisa quando um
agente termina e leva à instância escolhida. Documentação: [docs/janela-flutuante.md](docs/janela-flutuante.md).

### Linux

- [x] Estado de cada instância (ocioso, trabalhando, terminou, precisa de você) detectado pela TUI e
  enviado ao daemon MCP.
- [x] Janela sem borda, transparente e sempre no topo, com as animações do mascote por estado.
- [x] Aviso quando um agente termina: animação, balão com a instância e `notify-send`.
- [x] Clique no mascote: menu com os agentes abertos em todos os terminais; escolher um abre a
  sub-aba dele no terminal certo.
- [x] Trazer a janela do terminal para frente no X11 (`xdotool`/`wmctrl`).
- [x] `terraforme pet` e `[tui] pet` (abre junto com `terraforme cli`).
- [x] Trazer a janela do terminal para frente no GNOME/Wayland (extensão
  `terraforme@augustowtg.github.io`, instalada pelo `install.linux.sh`).
- [ ] Trazer a janela do terminal para frente nos outros compositores Wayland (o protocolo não
  deixa um app ativar a janela de outro; depende de cada um, ex.: KWin script, `kitty @`,
  `swaymsg`).
- [ ] Sempre no topo no GNOME/Wayland (o compositor ignora o pedido; hoje fica a notificação do
  sistema).
- [ ] Lembrar a posição da janela entre as execuções.
- [ ] Primeira release do plugin [terraforme-pet](https://github.com/Augustowtg/terraforme-pet)
  (`terraforme-pet-<target>.tar.gz`, glibc): até lá `terraforme plugin install pet` compila com
  `cargo install --git`.
- [ ] Estado vindo dos hooks do agente (ex.: `Stop` do Claude Code) em vez da heurística da saída do
  terminal.
- [ ] Mascote "cansado" quando os limites do plano passam de 80%.

### Windows

- [ ] Janela flutuante (`terraforme-pet` para `x86_64-pc-windows-msvc`), sempre no topo.
- [ ] Trazer a janela do terminal para frente (`SetForegroundWindow` na janela do console /
  Windows Terminal).
- [ ] Notificação nativa (toast) quando um agente termina.
- [ ] Incluir o binário no `install.windows.ps1`, no `install.ps1` e no release.

Até lá, `terraforme pet` no Windows só avisa que a janela ainda não está disponível. O estado das
instâncias já é enviado ao daemon também no Windows.
