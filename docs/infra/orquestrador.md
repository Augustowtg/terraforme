# Daemon orquestrador (hub)

O orquestrador é um processo só por usuário (o **hub**) que acompanha todos os
terminais do terra abertos na máquina. Ele é a base para monitorar e,
depois, controlar os terminais e os sub agentes. Hoje (v0.10.0), o hub:

- sobe sozinho quando o primeiro `terra cli` abre e fecha sozinho quando o último fecha;
- recebe de cada terminal o que ele está fazendo (parado ou rodando, a tarefa do agente, os chats abertos);
- separa os terminais **por workspace** na aba **Orquestrador** e no `terra status`.

```
 Orquestrador

 Workspaces
   terraforme-cli  ·  3 instâncias ativas + 2 sub agentes
     ● Pensando...                  │ 3 sessões  │ 5 min
     ● parado                       │ 1 sessão   │ 2 min
     ● Usando Bash...               │ 1 sessão   │ 1 min
   smart-mecanica  ·  1 instância ativa
     ● parado                       │ 1 sessão   │ 40 s

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
| `src/hub/tests.rs` | Testes com hubs em processos próprios: `kill -9`, dez janelas juntas, cliente lento |

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
No Windows, o pipe é criado com uma DACL só para o usuário do hub e para o
sistema (`D:P(A;;GA;;;<SID do usuário>)(A;;GA;;;SY)`), desde a v0.10.0.

Ao lado do lock ficam:

- o **`hub.log`**, porque o hub não tem terminal. Passando de 1 MB, ele vira `hub.log.1` (o anterior sai) e recomeça;
- o **`hub.guard`**, a trava que elege o hub (veja o ciclo de vida).

Com a variável `TERRAFORME_HUB_DIR`, o socket, o lock, o guard e o log ficam
todos nessa pasta. O hub iniciado por um cliente com ela a recebe também.
É assim que os testes sobem hubs de verdade sem tocar no do usuário.

O transporte é o crate [`interprocess`](https://crates.io/crates/interprocess)
(`local_socket`): socket Unix no Linux e no macOS, named pipe no Windows, com
a mesma API síncrona.

## 2. Ciclo de vida do hub

```
serve():
    pasta 0700 (Unix)
    trava o hub.guard (try_lock)
    outro processo tem a trava → outro hub está vivo → sai(0)
    se conecta no endpoint → um hub de antes da trava (v0.9) está vivo → sai(0)
    apaga o socket que sobrou (Unix): com a trava, ele só pode ser de um hub morto
    bind(endpoint) (no Windows, com a DACL do usuário)
    grava o hub.lock de forma atômica (hub.lock.tmp + rename):
        { pid, start_time, endpoint, proto: 1, versao_app }
    log em hub.log
    loop (accept não bloqueante, a cada 50 ms):
        aceita conexão → uma thread por cliente
        a cada 1 s: marca stale e tira as gone antigas
        0 conexões por 5 s → apaga o lock (se ainda é dele) → sai
```

- **Corrida de hubs**: dois terras abrindo juntos podem iniciar dois hubs, mas só um consegue a trava do `hub.guard`; o outro sai. O SO solta a trava quando o processo morre, até num `kill -9`. Antes da v0.10.0, quem decidia era o `bind`. Depois de um `kill -9`, as janelas reconectando juntas iniciavam vários hubs: cada um achava o socket órfão, apagava o do outro e ficava vivo, e as janelas se dividiam entre hubs que não se viam. O teste `hub::tests` com o `kill -9` pegou isso.
- **Fecha com o último terminal**: o pedido foi que o hub feche assim que todos os terminais fecharem. Por isso a espera é de **5 s** sem nenhuma conexão (e não os 10 min do rascunho): o intervalo só cobre quem fecha um terra e abre outro em seguida. A aba Orquestrador e o `terra status` também contam como conexão enquanto estão abertos.
- **Um hub só**: o primeiro terra que abre inicia o hub; os seguintes só se conectam a ele.

Os tempos ficam em `src/hub.rs`:

| Constante | Valor | O que é |
| --- | --- | --- |
| `HEARTBEAT` | 2 s | Sem novidade, a instância manda um `heartbeat` |
| `STALE_AFTER` | 6 s | Sem sinal (3 heartbeats perdidos), a instância fica `stale` |
| `GONE_KEEP` | 30 s | Quanto tempo uma instância `gone` continua na lista |
| `IDLE_EXIT` | 5 s | Sem nenhuma conexão por esse tempo, o hub sai |
| `MAX_LINE` | 1 MB | A maior linha do protocolo; uma maior derruba a conexão |
| `OUTBOX` | 256 | Mensagens na fila de saída de cada cliente; cheia, ele é desconectado |
| `LOG_MAX` | 1 MB | O tamanho em que o `hub.log` vira `hub.log.1` |

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
{"t":"status","state":"running","task":"Pensando...","chats":2,"agents":1,
 "tabs":[{"id":"9c1e…","title":"Principal","kind":"manual","task":"Pensando..."},
         {"id":"41d0…","title":"Chat 2","kind":"manual","task":null}]}
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
- Cada instância é uma **janela** (um `terra cli`). Em `tabs` vão as **abas com agente** dela: o chat principal e os do `Alt+N`, cada um com um id aleatório criado com o chat, o título e o que o agente faz (`task`, `null` quando parado). O `kind` é sempre `manual` por enquanto: o usuário abriu a aba e conversa nela.
- **Uma linha tem no máximo 1 MB.** Quem recebe uma linha maior fecha a conexão, e quem envia não chega a mandá-la.
- **O hub nunca escreve direto num cliente.** Cada conexão tem uma fila de saída de 256 mensagens e uma thread que escreve. Um cliente que não lê enche a fila e é desconectado (`cliente lento` no log), sem segurar o hub nem os outros. No Unix, a conexão fecha com um `shutdown`; no Windows, com o `DisconnectNamedPipe`.
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

- **`terra status [--json]`**: pega o snapshot, imprime e sai. Embaixo de cada janela vêm as abas com agente (`Principal    parado`). No JSON, elas ficam em `status.tabs`. Serve para scripts e testa o protocolo inteiro. Sem terminais, mostra `Nenhuma instância ativa.` (o hub que ele iniciou fecha 5 s depois).

  ```
  $ terra status
  terraforme-cli  ·  1 instância ativa
    265a8949  pid 258049  alive  parado    1 chat  aberto há 3s  sinal há 0s

  tmp  ·  1 instância ativa
    882cf04c  pid 258050  alive  parado    1 chat  aberto há 2s  sinal há 0s
  ```

- **Aba Orquestrador** (no `terra cli`): a lista ao vivo, separada por workspace, com o resumo de cada um (`3 instâncias ativas + 2 sub agentes`) e, por instância, uma linha de tabela: a bolinha da saúde (verde `alive`, laranja `stale`, vermelha `gone`), a tarefa, as sessões (os chats) e há quanto tempo está aberta (ex.: `● parado │ 1 sessão │ 10 min`), sem o id e o PID (v0.9.1). O monitor só é assinado na primeira vez que a aba abre.

## Testes

- `hub::daemon::tests`: sobe um hub numa pasta temporária, conecta um monitor e uma instância e confere a entrada, a mudança de estado, o `snapshot`, o `bye` (`gone` com `exit`) e a saída sozinho sem conexões (levando o lock).
- `hub::protocol::tests` e `hub::view::tests`: as mensagens, o limite de 1 MB nos dois sentidos, o agrupamento por workspace e os textos.
- `hub::daemon::tests::the_log_turns_into_log_1_past_the_limit`: a rotação do `hub.log`.
- `hub::tests`: hubs de verdade, em processos próprios. O hub é o próprio executável de testes rodando só o teste ignorado `hub_process`, com a pasta do teste em `TERRAFORME_HUB_DIR`:
  - **`kill -9` no hub** com três janelas abertas: todas reconectam ao hub novo, que nasce de uma delas, e reenviam o estado, até o que mudou com o hub fora do ar;
  - **dez janelas no mesmo segundo**: dez hubs disputam e só um sobe (uma linha de início no `hub.log`);
  - **cliente que não lê**: é desconectado, e o monitor que lê recebe as 5000 mudanças até a última.

O CI roda os testes no Linux, no Windows e no macOS.

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
