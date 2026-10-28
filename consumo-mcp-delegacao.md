# Consumo do MCP do terra ao delegar chats

Por que os chats abertos pela ferramenta `chat` do MCP do terra (o
orquestrador) gastam tanto do limite da conta, e o que fazer para gastar menos.

Análise de 2026-10-28, com os transcripts reais das duas rodadas de delegação
do dia (`~/.claude/projects/-home-augusto-kaizen-terraforme-terraforme-cli/`),
cada uma com 6 chats delegados. O uso de cada mensagem foi contado uma vez só.

## Os números

| Lote | Quem | Leitura de cache | Criação de cache | Saída |
|---|---|---|---|---|
| 11:10 (Sonnet) | 6 delegados | **18,7M** | 607k | 152k |
| 11:10 | orquestrador (20× `wait`) | 1,9M | 95k | 12k |
| 04:11 (Opus) | 6 delegados | 4,3M | 426k | 55k |
| 04:11 | orquestrador (16× `decide`, 13× `wait`) | 4,5M | 137k | 34k |

- No lote das 11:10, os chats delegados fizeram cerca de **90% do gasto**.
- Os dois chats mais caros leram 6,4M e 7,3M tokens de cache, em 60 e 65
  chamadas à API.

## Os motivos, do maior para o menor

### 1. Cada chat delegado já começa com ~29,5k tokens

- Os 12 chats começaram com um contexto inicial de 29,5k a 29,8k tokens,
  antes de fazer qualquer coisa.
- O chat delegado é um `claude -p` com **todo o ambiente do Claude Code do
  usuário** (`cli_args`, `src/agent/adapters/claude.rs`): todos os servidores
  MCP (ai-memory com ~25 ferramentas, git, Claude Docs, postman, o próprio
  terra), a lista de todas as skills e plugins, o CLAUDE.md e os hooks.
- Com extensões que têm hooks (ai-memory, rtk), `hook_settings` **também liga
  os hooks do usuário**. Com isso os textos do SessionStart (o
  "EXTREMELY_IMPORTANT" do Postman e o perfil do ai-memory) entram em todo
  chat delegado.
- Esse prefixo é **reenviado em toda chamada**. Em 65 chamadas, só ele já
  soma ~1,9M tokens lidos.

### 2. O chat delegado começa do zero e reexplora o repositório

- O chat recebe só o texto da tarefa (`ChatRequest::prompt`, `src/mcp.rs`),
  sem nada do contexto do orquestrador.
- Por isso ele lê o código de novo: 57 a 61 chamadas de `Bash` nos dois chats
  mais caros. Cada saída de comando (cargo, grep, testes) fica no contexto e é
  **relida em todas as chamadas seguintes**: o custo cresce de forma quase
  quadrática com o número de chamadas.
- Com 6 chats em paralelo na mesma base, a exploração se repete 6 vezes.

### 3. Os chats começam juntos e não aproveitam o cache um do outro

- Os 6 chats do lote das 11:10 começaram no mesmo instante (11:10:18.044 a
  .419). Quando todos sobem juntos, nenhum acha o prefixo no cache e **cada um
  paga a criação de cache inteira** (58k a 167k por chat). A criação de cache
  custa mais que a entrada normal.

### 4. Esforço `high` nos chats delegados

- O `~/.terraforme/config.json` tem `"mcp": { "model": "sonnet", "effort":
  "high" }`. O esforço alto deixa a saída maior (até 54k tokens de saída num
  chat) e faz o agente usar mais ferramentas.

### 5. O orquestrador gasta com o ciclo de `wait` e `decide`

- O `wait` devolve `running` a cada 120 s por padrão (`WAIT_DEFAULT`,
  `src/mcp/server.rs`). A cada retorno, o orquestrador relê o próprio
  contexto, que é grande: foram 20 `wait` no lote das 11:10.
- No lote das 04:11 (antes do `spawn_mode: yolo`), cada pedido de permissão
  custou um turno inteiro do orquestrador. Foram **16 `decide`, que levaram a
  4,5M lidos**, mais do que os 6 delegados juntos.

### 6. O chat delegado também recebe o MCP do terra

- `delegated()` (`src/tui/app.rs`) só troca a URL e mantém o servidor `terra`.
  As ferramentas `spaces`, `chat`, `wait` e `decide` entram no prompt de todo
  delegado, que gasta tokens com elas e pode, inclusive, delegar de novo.

## O que reduziria mais

1. **Ambiente enxuto para os delegados**: rodar com `--strict-mcp-config` e só
   o MCP que importa, sem skills e plugins, e com `disableAllHooks`, ou pelo
   menos sem os hooks do usuário. Isso deve levar o prefixo de ~30k para
   poucos milhares de tokens por chamada.
2. **Tirar o servidor `terra` dos chats delegados**, ou deixar só a ferramenta
   `permission`.
3. **Esforço `medium` ou `low`** e Sonnet ou Haiku para os delegados, no
   `/model` da aba Orquestrador.
4. **Começar os chats um depois do outro**, com alguns segundos de intervalo,
   para os seguintes aproveitarem o cache do primeiro.
5. **Mandar mais contexto na tarefa** (arquivos e caminhos exatos) para o
   delegado não reexplorar o repositório, e usar `timeout_seconds: 600` no
   `wait` para o orquestrador acordar menos vezes.

## O que mudou na v0.8.24

Os itens 1, 2, 3 e 6 entraram no terra: no Claude, o chat delegado roda
enxuto (veja o CHANGELOG). O prompt inicial medido no `claude` 2.1.308, com
`responda só: ok`:

| Combinação | Prompt inicial |
|---|---|
| Ambiente completo (como era) | 27,4k |
| Enxuto, com o ai-memory, sem `ToolSearch` | 33,0k |
| Enxuto, com o ai-memory e o `ToolSearch` (como ficou) | **11,1k** |
| Enxuto, sem MCP | 8,7k |

- Sem o `ToolSearch` no `--tools`, o `claude` não adia os schemas das
  ferramentas MCP, e só o ai-memory põe uns 24k no prompt. Por isso ele vai
  sempre na lista.
- Uma comunicação por arquivo em `~/.terraforme` não sairia mais barata. O
  delegado não gasta tokens para falar com o terra: a permissão é chamada pelo
  próprio `claude` (`--permission-prompt-tool`), e a resposta sai do stream.
  Com um arquivo, o agente precisaria de chamadas de `Read`/`Write`, e cada uma
  relê o contexto inteiro. O arquivo em `~/.terraforme/delegados/` guarda só a
  configuração dos MCP do delegado, para os segredos não irem na linha de
  comando.
- Sem os settings do usuário, as regras de permissão dele (`allow`) também
  não valem no delegado. Fora do Yolo, ele pode pedir mais permissões a quem
  delegou, e cada pedido custa um turno do orquestrador (item 5).
