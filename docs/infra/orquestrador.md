# Daemon orquestrador (hub)

O orquestrador é um processo só por usuário (o **hub**) que acompanha todos os
terminais do terra abertos na máquina. Ele é a base para monitorar e,
depois, controlar os terminais e os sub agentes. Nesta primeira versão
(v0.9.0), o hub:

- sobe sozinho quando o primeiro `terra cli` abre e fecha sozinho quando o último fecha;
- recebe de cada terminal o que ele está fazendo (parado ou rodando, a tarefa do agente, os chats abertos);
- separa os terminais **por workspace** na aba **Orquestrador** e no `terra status`.

```
 Orquestrador

 Workspaces
   terraforme-cli  ·  3 instâncias ativas + 2 sub agentes
     7f3a12bc  pid 4812    alive  Pensando...        3 chats  aberto há 5m  sinal há 1s  ← este
     91bd04aa  pid 4977    alive  parado             1 chat   aberto há 2m  sinal há 0s
     c0ffee12  pid 5120    stale  Usando Bash...     1 chat   aberto há 1m  sinal há 9s
   smart-mecanica  ·  1 instância ativa
     5a5a0b0c  pid 6001    alive  parado             1 chat   aberto há 40s sinal há 1s

 Configurações
   Dos chats delegados; por enquanto, só ficam salvas.
   Provedor            Do workspace (Claude)
   ...
```

O código fica em `src/hub/`:

| Arquivo | Papel |
| --- | --- |
| `src/hub.rs` | Constantes (protocolo, tempos) e a `Conn`, a conexão compartilhada entre leitura e escrita |
| `src/hub/endpoint.rs` | `Endpoint`: onde fica o socket/pipe, o `hub.lock` e o `hub.log` |
| `src/hub/daemon.rs` | O processo do hub (`terra __hub`): bind, lock, conexões, saúde e saída |
| `src/hub/client.rs` | `Reporter` (a instância), `Monitor` (a aba) e `snapshot` (o `terra status`) |
| `src/hub/protocol.rs` | As mensagens NDJSON, o `Status` e a `Instance` |
| `src/hub/view.rs` | A lista para pessoas: agrupada por workspace, com o resumo e a linha de cada instância |

## Papéis

| Papel | Quem | O que faz |
| --- | --- | --- |
| **hub** | `terra __hub` (escondido) | Escuta no endpoint, guarda o estado das instâncias e repassa o que muda aos monitores |
| **instance** | cada `terra cli` | Conecta ao abrir, manda o estado quando muda e um `heartbeat` quando não muda |
| **monitor** | a aba Orquestrador e o `terra status` | Assina a lista: recebe o `snapshot` e depois só os deltas |

## 1. Endpoint: onde todo mundo se encontra

Todos os papéis usam o mesmo `Endpoint::current()`:

| SO | Socket/pipe | Lockfile |
| --- | --- | --- |
| Linux | `$XDG_RUNTIME_DIR/terraforme/hub.sock` (sem ele: `/tmp/terraforme-<uid>/hub.sock`) | `~/.terraforme/hub.lock` |
| macOS | `/tmp/terraforme-<uid>/hub.sock` (caminho curto de propósito: o limite do `sun_path` é de 104 bytes) | `~/.terraforme/hub.lock` |
| Windows | `\\.\pipe\terraforme-<usuario>-hub` | `%LOCALAPPDATA%\terraforme\hub.lock` |

O `<uid>` ou o `<usuario>` no nome garante que dois usuários na mesma máquina
não se misturem. No Unix, a pasta do socket é criada com permissão `0700`.
O log do hub (`hub.log`) fica ao lado do lock: o hub não tem terminal.

O transporte é o crate [`interprocess`](https://crates.io/crates/interprocess)
(`local_socket`): socket Unix no Linux e no macOS, named pipe no Windows, com
a mesma API síncrona.

## 2. Ciclo de vida do hub

```
serve():
    se já conecta no endpoint → outro hub está vivo → sai(0)
    bind(endpoint)
    se falhou por "endereço em uso" (no Windows, "acesso negado"):
        se conseguir conectar → outro hub está vivo → sai(0)
        senão → socket órfão → apaga o arquivo → bind de novo
                 (se perder a corrida de novo → sai(0))
    pasta 0700 (Unix)
    grava o hub.lock de forma atômica (hub.lock.tmp + rename):
        { pid, start_time, endpoint, proto: 1, versao_app }
    log em hub.log
    loop (accept não bloqueante, a cada 50 ms):
        aceita conexão → uma thread por cliente
        a cada 1 s: marca stale e tira as gone antigas
        0 conexões por 5 s → apaga o lock (se ainda é dele) → sai
```

- **Corrida de dois hubs**: dois terras abrindo juntos podem iniciar dois hubs; só um consegue o `bind` e o outro sai. No Windows, o pipe é criado com `FILE_FLAG_FIRST_PIPE_INSTANCE`, então o segundo também falha.
- **Fecha com o último terminal**: o pedido foi que o hub feche assim que todos os terminais fecharem. Por isso a espera é de **5 s** sem nenhuma conexão (e não os 10 min do rascunho): o intervalo só cobre quem fecha um terra e abre outro em seguida. A aba Orquestrador e o `terra status` também contam como conexão enquanto estão abertos.
- **Um hub só**: o primeiro terra que abre inicia o hub; os seguintes só se conectam a ele.

Os tempos ficam em `src/hub.rs`:

| Constante | Valor | O que é |
| --- | --- | --- |
| `HEARTBEAT` | 2 s | Sem novidade, a instância manda um `heartbeat` |
| `STALE_AFTER` | 6 s | Sem sinal (3 heartbeats perdidos), a instância fica `stale` |
| `GONE_KEEP` | 30 s | Quanto tempo uma instância `gone` continua na lista |
| `IDLE_EXIT` | 5 s | Sem nenhuma conexão por esse tempo, o hub sai |

## 3. Lado da instância (`Reporter`)

```
Reporter::start():                     # thread própria
    loop:
        conn = conectar(endpoint)
        se falhou:
            spawn_desacoplado("terra __hub")
            tenta de novo com backoff: 50 ms, 100 ms, 200 ms… (até 800 ms) por até 3 s
        envia hello (com o estado COMPLETO de agora)
        aguarda welcome (ou reject → trata a versão)
        loop:
            pega o estado da fila (timeout 2 s)
            nada chegou → envia heartbeat
            a escrita falhou → sai do loop interno e reconecta (a cada 2 s)

# API que o resto do terra usa — NUNCA bloqueia:
Reporter::set_status(status)  → se mudou, põe na fila e volta
drop(Reporter)                → põe o bye na fila e espera até 300 ms ele sair
```

A regra de ouro: **o trabalho do terra nunca espera o monitoramento**. O
`App` calcula o estado a cada tick (100 ms) e chama `set_status`, que só
compara com o último e põe na fila. A thread junta os estados que chegaram
e manda só o mais novo; com o hub fora do ar, os antigos são descartados e o
terra segue normalmente.

O estado de uma instância (`Status`):

| Campo | De onde vem |
| --- | --- |
| `state` | `running` se algum chat tem o agente trabalhando; senão `idle` |
| `task` | O que o agente está fazendo (ex.: `Pensando...`, `Usando Bash...`), do primeiro chat que está rodando |
| `chats` | Os chats abertos: o principal e os do `Alt+N` |
| `agents` | Os **sub agentes**: os chats além do principal (hoje os do `Alt+N`; depois, os chats delegados) |

**Spawn desacoplado**: o hub é iniciado com `std::env::current_exe() __hub`,
com a entrada e as saídas no nada. No Unix, numa sessão nova (`setsid` no
`pre_exec`), então fechar o terminal ou apertar `Ctrl+C` não derruba o hub.
No Windows, com `DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP`. Uma thread
recolhe o processo se ele sair antes do terra (sem zumbi).

## 4. Protocolo (NDJSON: um JSON por linha)

```jsonc
// instância → hub
{"t":"hello","proto":1,"role":"instance","id":"7f3a…","pid":4812,
 "app":"0.9.0","cwd":"/proj/x","workspace":"x","args":["cli"],"started":1730120000,
 "status":{"state":"idle","task":null,"chats":1,"agents":0}}
{"t":"heartbeat"}
{"t":"status","state":"running","task":"Pensando...","chats":2,"agents":1}
{"t":"bye","reason":"exit"}

// hub → instância
{"t":"welcome","proto":1,"hub":"0.9.0"}
{"t":"reject","reason":"proto_mismatch","action":"restart_hub"}

// monitor → hub
{"t":"hello","proto":1,"role":"monitor"}
{"t":"subscribe","topic":"instances"}
{"t":"bye"}

// hub → monitor
{"t":"welcome","proto":1,"hub":"0.9.0"}
{"t":"snapshot","instances":[ … ]}
{"t":"instance_added","instance":{ … }}
{"t":"instance_updated","id":"7f3a…","changes":{"status":{ … },"last_seen":1730120042}}
{"t":"instance_removed","id":"7f3a…","reason":"exit"}
```

- O `id` é um id aleatório de 128 bits (hex) gerado ao abrir, **não o PID**, porque o SO reaproveita PIDs.
- O status vai por **push quando muda**, sempre o estado inteiro; o `heartbeat` é só um "estou vivo" vazio.
- O hub acrescenta ao objeto da instância: `health` (`alive`, `stale`, `gone`), `last_seen` (segundos Unix do último sinal) e `reason` (`exit`, `crash` ou `null`).
- `changes` traz só os campos que mudaram; o monitor os copia por cima do objeto.
- O `workspace` é o nome salvo do diretório ou, sem um, o nome da pasta (o mesmo das configurações do workspace). É por ele que a lista é separada.
- O `bye` do monitor faz o hub fechar a conexão: no Windows, o pipe não tem tempo limite de leitura, e é assim que a thread do monitor termina quando a aba é fechada.

**Versões**: um hub e uma instância com `proto` diferentes não conversam, e o
mais novo vence. Com a instância mais nova, o hub responde
`{"action":"restart_hub"}` e sai; a instância reconecta e inicia um hub da
versão dela. Com a instância mais antiga, o hub responde
`{"action":"upgrade_instance"}` e ela segue sem o hub.

## 5. Modelo de estado no hub

Cada instância tem um destes estados (`health`):

- **alive**: está conectada e mandando sinais.
- **stale**: não manda nada há mais de 6 segundos (3 heartbeats perdidos). Provavelmente travou. Volta a `alive` no próximo sinal.
- **gone**: a conexão fechou. O motivo vem do `bye` (`exit`) ou fica `crash` se não houver `bye`. O registro fica visível por 30 segundos e depois é removido (`instance_removed`).

O detalhe importante: **o fechamento da conexão é o sinal principal de
morte**. Quando um processo morre, até com `kill -9`, o SO fecha o socket na
hora. O heartbeat existe para pegar o caso que o socket não pega: o processo
vivo, mas travado.

Uma instância que reconecta (o hub caiu e voltou) manda o `hello` com o mesmo
`id` e substitui o registro antigo. Cada registro guarda a conexão dona dele,
para o fim da conexão antiga não marcar a nova como `gone`.

## 6. Display

O monitor é só mais um cliente, com `role: monitor`: faz o `subscribe`,
recebe o `snapshot` e aplica os deltas. Se o hub não existir, o monitor
também o inicia.

- **`terra status [--json]`**: pega o snapshot, imprime e sai. Serve para scripts e testa o protocolo inteiro. Sem terminais, mostra `Nenhuma instância ativa.` (o hub que ele iniciou fecha 5 s depois).

  ```
  $ terra status
  terraforme-cli  ·  1 instância ativa
    265a8949  pid 258049  alive  parado    1 chat  aberto há 3s  sinal há 0s

  tmp  ·  1 instância ativa
    882cf04c  pid 258050  alive  parado    1 chat  aberto há 2s  sinal há 0s
  ```

- **Aba Orquestrador** (no `terra cli`): a lista ao vivo, separada por workspace, com o resumo de cada um (`3 instâncias ativas + 2 sub agentes`) e, por instância, o id curto, o PID, o estado, a tarefa, os chats, há quanto tempo está aberta e o último sinal. A instância do próprio terminal tem `← este`. O monitor só é assinado na primeira vez que a aba abre; as instâncias `stale` aparecem em vermelho e as `gone`, apagadas.

## Testes

- `hub::daemon::tests`: sobe um hub numa pasta temporária, conecta um monitor e uma instância e confere a entrada, a mudança de estado, o `snapshot`, o `bye` (`gone` com `exit`) e a saída sozinho sem conexões (levando o lock).
- `hub::protocol::tests` e `hub::view::tests`: as mensagens, o agrupamento por workspace e os textos.

Para ver no terminal:

```bash
cargo build
./target/debug/terra cli            # em dois terminais, em pastas diferentes
./target/debug/terra status         # num terceiro
tail -f ~/.terraforme/hub.log
```

## Próximos passos

- **Controlar** os terminais pelo hub (mensagens hub → instância, ex.: focar, encerrar um chat, abrir um chat delegado).
- Os chats delegados de verdade, com as configurações da seção Configurações da aba.
- No Windows, restringir a DACL do pipe ao usuário (hoje vale a DACL padrão do named pipe).
