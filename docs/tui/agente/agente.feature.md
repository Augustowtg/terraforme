# Feature: Middleware (Experimental)

Interface de chat do próprio terraforme no lugar do terminal do agente: o terraforme conversa com
o router (LLM) por baixo, em pipes, e desenha as mensagens e a caixa de entrada. Implementação em
`crates/terraforme-cli/src/chat.rs`, `markdown.rs` e `terraforme_core::chat`.

## Comportamento

### Como ligar

- Campo `Middleware (Experimental)` na seção **Agente** da aba
  [Configurações](../config/config.feature.md), logo abaixo da Conta: `sim` | `não`
  (`Espaço`/`Enter`/`←→` alternam, `Del`/`d` volta ao padrão `não`).
- Grava `[tui] middleware` no `config.toml` (nunca nas preferências do space) e vale nas próximas
  instâncias (`⌃⌥N`, ou `⌃⌥R` para trocar a aberta).
- Com ele ligado, a aba do agente se chama **Middleware** e as instâncias de todos os routers são
  sub-abas dela (`⌃⌥N` abre outra, `⌃⌥1-9` troca).

### Tela

```text
 ⌃⌥1 claude 1  ⌥2 ou ⌃⌥N nova instância · ⌃⌥W fecha · ⌃⌥C copia a conversa
· sessão iniciada · modelo claude-opus-5-5          ← notas (cinza)

› você                                               ← mensagens do usuário (ciano)
  liste os arquivos

◆ claude                                             ← resposta do agente (verde, em tempo real)
  Vou listar.
  ⚙ Bash  ls                                         ← ferramenta chamada (amarelo)
    ↳ Cargo.toml…                                    ← resultado (cinza; vermelho se erro)
┌ mensagem ───────────────────────────────────────┐
│ █                                               │  ← caixa de entrada (até 6 linhas)
└ Enter envia · Alt+Enter quebra a linha · / comandos · @ arquivos ┘
```

- Respostas com Markdown formatado (títulos, listas, citações, **negrito**, `código`, links) e
  blocos de código coloridos pela linguagem (sintaxes do `bat`), na paleta do
  [VS-Claude](https://github.com/EstarinAzx/VS-Claude) (`markdown.rs`).
- Enquanto o agente responde, o rodapé mostra `⏳ <agente> trabalhando…`; ao terminar, a
  instância fica "precisa de você" na [janela flutuante](../../janela-flutuante.md).
- Mensagens do [MCP](../../MCP/mcp.feature.md) (`send_message`, `delegate_task`,
  `open_instance`) entram direto como mensagem do usuário, sem esperar o agente ficar quieto.
- Se o agente sai com erro, o stderr dele (últimas 20 linhas) aparece num aviso.

### Teclas

| Tecla | Ação |
| --- | --- |
| `Enter` | envia |
| `Alt+Enter` (ou `Shift+Enter` com o protocolo do kitty) | quebra a linha |
| `←→` `Home` `End` `Backspace` `Delete` | editam |
| `Ctrl+U` | limpa a caixa |
| `PgUp`/`PgDn`, `↑↓` com a caixa vazia, roda do mouse | rolam a conversa |
| `Ctrl+Alt+C` | copia a conversa inteira |

Os atalhos da TUI (`Ctrl+Alt+…`, `Ctrl+Shift+F…`) seguem iguais ([menu](../tui.feature.md)).

### Completar `/` e `@`

- `/` no começo da mensagem lista os comandos do agente (com descrição e argumentos).
- `@` lista arquivos e pastas da pasta atual, respeitando o `.gitignore` (lidos no primeiro `@`).
- `↑↓` escolhem, `Tab` completa, `Enter` completa (e envia um comando sem argumentos), `Esc` fecha.

### Permissões

- Quando uma ferramenta precisa de permissão, o pedido substitui a caixa de mensagem
  (`permitir Bash?` com o comando).
- `1` Sim, `2` Sim e não perguntar de novo (grava a regra sugerida pelo agente, como no
  terminal), `3`/`Esc` Não; `↑↓` + `Enter` também escolhem.
- Enquanto espera, a instância fica "precisa de você" na janela flutuante.
- Cada agente pergunta do jeito dele: o Claude Code grava as regras do "não perguntar de novo",
  o OpenCode escolhe a opção `always` que ele mesmo oferece e o Antigravity não pergunta (no modo
  print as ferramentas rodam liberadas).

## Como funciona

- `Agent::spawn` roda os agentes que têm `Adapter::chat_args` com pipes em vez de PTY
  (`master: None`): cada linha do stdout vira `Msg::Output`, o stderr `Msg::Stderr`. As teclas vão
  para `Agent::chat_key` (não para `key_bytes`) e o que vai ao agente passa por `Chat::outbox`
  (`Agent::flush_chat`). Argumentos que atrapalham o modo saem da linha de comando
  (`Adapter::chat_drop_args`).
- **Claude Code**: `claude -p --input-format stream-json --output-format stream-json --verbose
  --include-partial-messages --permission-prompt-tool stdio` (`chat::CLAUDE_ARGS`), com os mesmos
  argumentos do terminal (plugins do modo, sessão, MCP, AI Jail).
  - Ao abrir manda o `initialize` do SDK; a resposta traz os comandos `/`
    (`ChatEvent::Commands`).
  - `can_use_tool` vira `ChatEvent::Permission`, respondido por `claude_permission`; outros
    pedidos de controle recebem erro na hora.
  - Mensagens de subagentes (`parent_tool_use_id`) ficam de fora.
- **Antigravity**: `agy --input-format stream-json --output-format stream-json`
  (`chat::AGY_ARGS`), sem `-p`: a mensagem vira o evento `user` (`agy_input`), o `init` abre a
  conversa, cada `step_update` anda um passo (pedaços de texto ou uma ferramenta, do começo ao
  fim) e o `result` fecha o turno (`agy_events`).
- **OpenCode**: `opencode acp` (`chat::OPENCODE_ARGS`), um JSON-RPC por linha no stdio (Agent
  Client Protocol). O handshake `initialize` → `session/new` abre a sessão (`acp_events`), que o
  chat guarda (`ChatEvent::Session`) e repassa em todos os pedidos; o que o usuário mandar antes
  dela fica guardado até abrir. As mensagens são um `session/prompt` (`acp_input`), as respostas
  chegam em `session/update` e os pedidos de permissão são respondidos por `acp_permission`.
- O Markdown é renderizado por entrada e guardado em cache por largura/tamanho do texto.

## Limitações

- O Middleware cobre Claude Code, Antigravity e OpenCode; os outros agentes continuam abrindo no
  terminal, com um aviso. O que falta está no [ROADMAP](../../../ROADMAP.md).
- Comandos que só existem no terminal do Claude (ex.: `/doctor`, `/color`) não aparecem na lista.
