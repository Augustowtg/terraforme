# Feature: MCP do terraforme

O daemon `terraforme-mcp` (a [engine](../tui/engine/engine.feature.md)) liga todas as instâncias
de agente abertas pelo `terraforme cli`, em qualquer terminal e space. Com o MCP `terraforme`,
cada instância conversa com as outras, abre e fecha instâncias e delega tarefas: o terraforme
cria e gerencia agentes, e não só os abre.

Foco atual: **Claude Code**. O que falta para os outros agentes está no [ROADMAP](../../ROADMAP.md).

## Comportamento

### Ferramentas

| Ferramenta | O que faz | Pede confirmação |
| --- | --- | --- |
| `list_instances` | Instâncias abertas em todos os spaces (apelido, agente, space, pasta; a sua com `you=true`). | não |
| `send_message(to, text)` | Digita a mensagem como prompt na instância `to`, com o apelido de quem mandou para a resposta (`to="web"` escreve só na [UI web](../tui/webserver/webserver.feature.md)). | não |
| `open_instance(text?)` | Abre uma instância nova do mesmo agente numa sub-aba do terminal de quem pediu e devolve `{opened: <apelido>}`; `text` chega como primeira mensagem. | não |
| `delegate_task(task, to?)` | Entrega a tarefa a `to` ou, sem `to`, a uma instância nova, e espera: o `send_message` de volta dela vira o `result` da chamada (até 30 min). Devolve `{delegated_to, opened, result}`. | não |
| `close_instance(to)` | Encerra outra instância (nunca a própria). Devolve `{closed: <apelido>}`. | **sim** |
| `spawn_subagent(task, cwd?, model?)` | Roda a tarefa com `claude --print` numa sub-aba temporária e devolve `{output, exit_code}` na própria chamada. | **sim** |

- Apelidos: `<space ou pasta>/<agente>-<n>` (ex.: `api/claude-2`), o mesmo número da sub-aba;
  a busca não diferencia maiúsculas e aceita também o id interno.
- "Pede confirmação" = fora da lista `permissions.allow` que o terraforme passa ao Claude Code em
  `--settings` (`Claude::setup`).

### Duas instâncias conversando

```gherkin
Dado que "api/claude-1" e "api/claude-2" estão abertas
Quando "api/claude-1" chama send_message(to="api/claude-2", text="qual o formato do endpoint X?")
Então o terminal de "api/claude-2" digita a mensagem como prompt, com o remetente
E "api/claude-2" responde com send_message(to="api/claude-1", ...)
E as duas mensagens aparecem no histórico da UI web
```

### Delegar uma tarefa a uma instância nova

```gherkin
Dado que "api/claude-1" está aberta e o terminal tem menos de 9 instâncias
Quando ela chama delegate_task(task="rode os testes e me diga o que falhou")
Então o terminal dela abre "api/claude-2" sem tirar o foco do usuário (aviso "＋ claude 2 aberto por api/claude-1")
E a tarefa chega a "api/claude-2" com o pedido de devolver o resultado a "api/claude-1"
E a chamada de "api/claude-1" fica esperando
Quando "api/claude-2" termina e chama send_message(to="api/claude-1", text="2 testes falharam")
Então a chamada volta com delegated_to="api/claude-2" e result="2 testes falharam"
E a mensagem não é digitada em "api/claude-1" (aparece no histórico da UI web)
```

### Tarefa delegada sem resposta

```gherkin
Dado que "api/claude-1" espera o resultado de "api/claude-2"
Quando "api/claude-2" é fechada antes de responder
Então delegate_task devolve erro "api/claude-2 fechou sem devolver o resultado"
Quando passam 30 min sem resposta
Então delegate_task volta com result=null e um aviso
E a resposta, quando vier, chega em "api/claude-1" como um prompt comum
Quando o Claude Code cancela a chamada antes disso
Então a resposta, quando vier, também chega como um prompt comum
```

### Delegar a uma instância já aberta

```gherkin
Quando "api/claude-1" chama delegate_task(task="...", to="web/claude-1")
Então a tarefa é digitada em "web/claude-1" (outro space ou terminal) com o pedido de resposta
E a chamada espera o resultado do mesmo jeito
```

### Entrega da mensagem como prompt

```gherkin
Dado que uma mensagem chega para uma instância (send_message, open_instance ou delegate_task)
Quando o agente já ligou o bracketed paste (ou passaram 20s da abertura) e a saída está parada há 500 ms
Então o terminal cola o texto e manda o Enter 300 ms depois, enviando o prompt
```

No [Middleware](../tui/agente/agente.feature.md), a mensagem entra direto no chat, sem esperar.

### Fechar uma instância

```gherkin
Quando "api/claude-1" chama close_instance(to="api/claude-2")
Então o Claude Code pede confirmação ao usuário
E o terminal de "api/claude-2" encerra o processo e a sub-aba some (aviso "✕ claude 2 fechado por api/claude-1")
```

### Subagente temporário

```gherkin
Quando "api/claude-1" chama spawn_subagent(task="resuma o README")
Então o Claude Code pede confirmação ao usuário
E o terminal abre uma sub-aba com "claude --print" e passa o foco para ela
Quando o processo termina
Então a chamada volta com output e exit_code, e a sub-aba fecha
```

### Limites e erros

```gherkin
Quando o terminal de quem pede já tem 9 instâncias
Então open_instance/delegate_task devolvem erro pedindo para fechar uma com close_instance

Quando a instância não se registra em 10s (ex.: o agente não pôde ser iniciado)
Então a chamada devolve erro e o motivo aparece na TUI

Quando a instância pede para fechar a si mesma, delegar a si mesma ou mandar mensagem para si mesma
Então a chamada devolve erro

Quando já há "Subagentes simultâneos" spawn_subagent em andamento (padrão 3)
Então spawn_subagent devolve erro; e passado o "Timeout do subagente" (padrão 300s), erro de timeout
```

## Como funciona

- Cada instância recebe o servidor por `Adapter::mcp_args` (Claude: `--mcp-config` com os
  cabeçalhos `x-client-id`/`x-instance-id`). Com AI Jail, `jail::wrap` encaminha a porta da
  engine, por isso a engine sobe antes do comando ser montado.
- O `Hub` (`crates/terraforme-mcp/src/hub.rs`) guarda as instâncias de todos os terminais e o
  histórico de mensagens (até 500).
- Abrir (`open_instance`/`delegate_task` sem `to`):
  1. O daemon escolhe o id da instância nova e manda `POST /open` (`instance_id`, `from`, `text`)
     ao servidor de controle do terminal de quem pediu.
  2. A TUI abre a instância com esse id sem mudar o foco (`App::open_requested`); ela se registra
     no daemon como qualquer outra e o daemon devolve o apelido quando o registro chega.
  3. `text` (ou a tarefa delegada) entra na fila do prompt (`Agent::queue`) e é digitado quando o
     agente termina de iniciar (`Agent::deliver_pending`).
- Delegação: o daemon guarda quem espera o resultado de quem (`Hub::wait`, 30 min). O
  `send_message` do trabalhador para quem delegou resolve a espera (`Hub::resolve`) e vira o
  `result` em vez de ser digitado.
- `close_instance` manda `POST /close` ao terminal da instância, que mata o processo; a sub-aba
  some quando ele termina.
- `spawn_subagent` manda `POST /spawn` ao terminal; o resultado volta por `POST /spawn-result`.
- A instância nova usa o agente, o modo e a sandbox atuais do terminal (os da aba Config, se
  mudaram).

### Rotas HTTP do daemon

| Rota | Quem chama | Uso |
| --- | --- | --- |
| `POST /` | agentes | JSON-RPC do MCP (`tools/list`, `tools/call`) |
| `POST /register`, `/heartbeat`, `/unregister` | TUI | registro do terminal ([engine](../tui/engine/engine.feature.md)) |
| `POST /instances/register`, `/instances/unregister` | TUI | registro de cada instância |
| `POST /instances/state` | TUI | atividade da instância (janela flutuante) |
| `POST /spawn-result` | TUI | saída do `spawn_subagent` |
| `GET /pet/state`, `POST /pet/focus` | janela flutuante | ver [janela flutuante](../janela-flutuante.md) |
| `GET /ui`, `/api/*` | navegador | ver [Webserver](../tui/webserver/webserver.feature.md) |

O servidor de controle de cada TUI recebe `POST /open`, `/close`, `/message`, `/spawn` e `/focus`.

## Limitações

- Só o Claude Code recebe o MCP; o resto está no [ROADMAP](../../ROADMAP.md).
- `spawn_subagent` chama o binário do agente com flags do Claude (`--print
  --dangerously-skip-permissions -p`), sem os plugins do modo nem o AI Jail, e tira o foco do
  usuário (ao contrário de `open_instance`).
- Agentes abertos antes de uma troca de engine mantêm a porta antiga até serem reiniciados.
