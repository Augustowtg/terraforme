# Provedores: conexões do terraforme com o Claude

Este documento mapeia todos os pontos em que o terraforme fala com o provedor de agente (hoje, só o **Claude**, pelo CLI `claude`), para servir de guia na adição de novos provedores. Cada seção termina com **Para um provedor novo**: o que implementar e o que hoje está preso ao Claude.

> Mapeamento feito na v0.8.23. As referências `arquivo:linha` são dessa versão e podem andar com o código.
>
> Desde a v0.8.29 o terra **não tem mais um MCP próprio** nem chats delegados: o comando `terra mcp`, o `src/mcp/`, o `permission_tool` e a delegação saíram. As seções que ainda os descrevem ficam como histórico do que existia até a v0.8.28; hoje o Orquestrador só guarda as configurações (`src/tui/orchestrator.rs`).

## Mapa geral

| Área | O terra usa | Onde o Claude entra |
| --- | --- | --- |
| [Contrato e execução](#contrato-do-provedor-e-execução) | `ChatAdapter`, `Session`, `Request`, `AgentEvent` (`src/agent/adapter.rs`, `src/agent/event.rs`) | `src/agent/adapters/claude.rs` + `claude/stream.rs`: um `claude -p` em stream-json por mensagem |
| [Comandos `/` e menções `@`](#comandos--e-menções-) | `Catalog`, `CommandRoute`, `ModelMenu` (`src/tui/chat/`) | `claude/catalog.rs` (`initialize` do Agent SDK) e `claude/interactive.rs` (comandos do modo interativo) |
| [Consumo](#consumo) | `load_usage`, `load_daily`, eventos `Context`/`Done` | `claude/usage.rs` (`/api/oauth/usage`) e `claude/daily.rs` (`projects/**/*.jsonl`) |
| [Extensões, MCP e skills](#extensões-mcp-e-skills) | `Contribution` (hooks, MCPs, env) e `mcp_servers`/`skills` | `--settings`, `--mcp-config`, env; `claude/mcp.rs` e `claude/skills.rs` |
| [Orquestrador](#orquestrador) | `OrchestratorSettings` (`src/settings.rs`) e a aba `src/tui/orchestrator.rs` | por enquanto, só configurações salvas; não há delegação |
| [Configurações, contas e login](#configurações-contas-e-login) | `AgentSettings`, `OrchestratorSettings`, `accounts`, `login` | `CLAUDE_CONFIG_DIR`, pastas `~/.claude*`, `claude auth login` |

## Checklist para um provedor novo

**Mínimo para conversar:**
1. Criar o módulo em `src/agent/adapters/` e registrá-lo em `adapters::all()` (`src/agent/adapters.rs:11`).
2. Implementar `name`, `is_available` e `start`. A `Session` precisa:
   - devolver eventos sem bloquear em `poll`;
   - terminar com `Done` ou `Failed`;
   - emitir `Init` com um `session_id` que possa ser retomado;
   - interromper a resposta no `Drop`.
3. Traduzir `PermissionMode` nos dois sentidos. No Yolo, o adapter aprova tudo sozinho.
4. Emitir `PermissionRequest` e implementar `respond`.
5. Emitir `Context`, `Compacted` e os tokens do `Done`.

**Para ter paridade com o Claude:**
- `load_catalog` (comandos, skills, modelos, esforços e modelo padrão), `command_route` + `interactive` (interface original), `login`, `model_label` e `models`.
- `load_usage` e `load_daily` (aba Consumo e barra de consumo).
- `accounts`, `configured_mode` e `skills` (Config).
- Traduzir a `Contribution`: hooks, servidores MCP stdio/HTTP com `${VAR}` nos headers, e env.

**Acoplamentos ao Claude fora do adapter, para resolver antes ou junto:**
- **Escolha do provedor:**
  - não há troca de provedor: só se usa `adapters::default()`, e o router não troca (`src/tui/chat.rs:1356`);
  - a aba Orquestrador só exibe o provedor;
  - `AgentSettings` e `OrchestratorSettings` não guardam de qual provedor são a conta, o modelo e o esforço.
- **Hooks e extensões:**
  - `Hook.event` e `matcher` usam os nomes do Claude Code;
  - o RTK usa `rtk hook claude`;
  - a ai-memory usa `--agent claude-code` / `--client claude-code`;
  - `contribution()` não recebe o provedor ativo.
- **Prompt e comandos:**
  - o `/compact` manda o texto literal `/compact` ao agente;
  - as menções saem como `@caminho` e `/skill` dentro do prompt;
  - `INTERFACES` intercepta `clear`, `compact`, `model`, `login` e `usage`.
- **Login:**
  - os marcadores `visit: `, `Invalid code` e `Paste code here if prompted >` estão em `src/tui/login.rs`;
  - as palavras `authenticate`, `oauth` e `log in again` estão em `hint_login`.
- **Valores fixos:**
  - `DEFAULT_CONTEXT_WINDOW = 200_000`;
  - os rótulos de esforço (`low` … `max`) e o preferido `high`;
  - a janela "Sessão (5 h)".
- **Instaladores:** não instalam nem detectam o CLI do provedor. Hoje só existe a dica do `unavailable_hint`.

## Contrato do provedor e execução

O chat só conhece o contrato de `src/agent.rs`: `ChatAdapter`, `Session`, `Request`, `AgentEvent` e `PermissionMode` (`src/agent.rs:1-19`). Cada provedor é um adapter. Hoje só existe um, o Claude CLI (`ClaudeAdapter`).

### Registro e escolha do provedor

| Função | Onde | O que faz |
|---|---|---|
| `adapters::all()` | `src/agent/adapters.rs:11` | Lista os provedores em ordem de preferência. Hoje é só `ClaudeAdapter::detect()`. |
| `adapters::default()` | `src/agent/adapters.rs:18` | Pega o primeiro que responde `is_available()`. Se nenhum responde, pega o primeiro da lista, e o chat fica Desconectado com a dica dele. Tudo isso passa por `guard::guarded`. |
| `Chat::new` / `with_adapter` | `src/tui/chat.rs:206`, `src/tui/chat.rs:210` | O chat guarda um `Option<Box<dyn ChatAdapter>>` e escolhe o status inicial pelo `is_available()`. |

Ainda não dá para trocar de provedor na tela. O botão do router mostra só `name()` (`src/tui/chat.rs:1356`, TODO).

### O trait `ChatAdapter` (`src/agent/adapter.rs:210-303`)

| Método | O que faz | Implementação padrão | Como o Claude implementa (`src/agent/adapters/claude.rs`) |
|---|---|---|---|
| `name()` | Nome no título do input, nos rótulos de origem (ex.: `Claude · usuário`) e nas mensagens de erro | — (obrigatório) | `"Claude"` (`:75`) |
| `is_available()` | `false` deixa o chat Desconectado | — (obrigatório) | Diz se o `claude` foi achado no `PATH`. No Windows, procura `claude.exe` ou `claude.cmd` (`find_claude`, `:288`). |
| `unavailable_hint()` | Texto mostrado quando o provedor não está disponível | `"{name} não está disponível."` | `NOT_FOUND`, com o link de instalação (`:39`) |
| `configured_mode(cwd, account)` | Modo de permissão com que o chat começa | `None` | Lê `permissions.defaultMode` na ordem `.claude/settings.local.json` → `.claude/settings.json` → `<config_dir>/settings.json` (`:466-502`) e traduz com `parse_mode` |
| `accounts()` | Contas que aparecem na Config | vazio | Default e as pastas `~/.claude*` com credenciais (`accounts::discover`, `:93`) |
| `skills(cwd, account)` | Skills do provedor, mostradas na Config | vazio | Pasta da conta, `~/.agents/skills` e `.claude/skills` do `cwd` e das pastas acima dele (`:117`) |
| `command_route(name)` | Diz onde roda uma `/` que o terra não implementa | `CommandRoute::Agent` | `BILLING` → `Blocked`; comandos que só existem no modo interativo → `Original`; o resto → `Agent` (`:154`) |
| `models()` | Apelidos de modelo na Config | vazio | `opus`, `sonnet`, `haiku` (`:239`) |
| `model_label(id)` | Nome do modelo para a tela | o próprio `id` | `claude-opus-5-5` → `Opus - 5.5` (`display_model`, `:685`) |
| `load_catalog(account)` | Carrega comandos e modelos em segundo plano (`Receiver<Result<Catalog>>`) | `None` | Manda o `initialize` do protocolo do Agent SDK, sem chamar a API (`catalog::load`, `:210`) |
| `load_usage(account)` | Consumo da conta (aba Consumo) | `None` | Endpoint de uso do Claude (`usage::load`, `:217`) |
| `load_daily(account, days)` | Tokens por dia e por hora | `None` | Lê os transcripts na pasta da conta (`daily::load`, `:226`) |
| `login(account)` | Comando interativo de login | `None` | `claude auth login [--email …]` com `CLAUDE_CONFIG_DIR` (`:134`) |
| `interactive(cwd, Interactive)` | Abre a interface original do provedor | `None` | O `claude` interativo, descrito mais abaixo (`:168`) |
| `start(Request)` | Começa uma resposta e devolve `Box<dyn Session>`. No Yolo, o adapter tem que aprovar tudo. | — (obrigatório) | `Run::start`: um processo `claude -p` por mensagem (`:255`, `:523`) |

`Session` (`src/agent/adapter.rs:306-314`):
- `poll()`: devolve, sem bloquear, os eventos que chegaram desde a última chamada. A resposta termina com `Done` ou `Failed`.
- `respond(id, allow)`: responde a um `PermissionRequest`. O padrão não faz nada.
- Fazer drop da sessão interrompe a resposta. No Claude, isso mata o processo (`claude.rs:672`).

### `Request` (`src/agent/adapter.rs:35-60`) e como cada campo vira argumento do `claude`

| Campo | Significado | No `claude` |
|---|---|---|
| `prompt` | Texto do usuário | Uma linha JSON no stdin: `{"type":"user","message":{"role":"user","content":…}}` (`user_message`, `:409`) |
| `mode` | `PermissionMode` | `--permission-mode` via `cli_mode` (tabela de modos abaixo) |
| `session_id` | `session_id` do último `Init` | `--resume <id>` |
| `model` / `effort` | Escolhidos na Config ou no `/model` | `--model`, `--effort` |
| `account` | `AccountOption::id` | Variável `CLAUDE_CONFIG_DIR=<pasta da conta>` |
| `subagent_model` | Modelo dos sub agentes | Variável `CLAUDE_CODE_SUBAGENT_MODEL` (`SUBAGENT_MODEL_ENV`, `:42`) |
| `extensions` (`Contribution`) | Hooks, MCPs e env das extensões | `--settings` (`hook_settings`, `:351`; sem hooks vai `{"disableAllHooks":true}`), `--mcp-config` (`mcp_args`, `:372`), e as variáveis `extensions.env` |
| `allowed_tools` | Regras liberadas só nesta mensagem (vêm de `Blocked`) | `--allowedTools <regra>…`, uma regra por argumento (`:547`) |

A linha de comando fixa sai de `cli_args` (`claude.rs:311`):

```
claude -p --input-format stream-json --output-format stream-json --verbose \
  --include-partial-messages --permission-mode <modo> --settings <json> \
  [--permission-prompt-tool stdio] [--model m] [--effort e] [--resume id]
```

O ambiente do usuário continua ligado: não vão `--strict-mcp-config` nem `--disable-slash-commands`. No Windows, um `.cmd` roda por `cmd /C` (`command`, `:650`).

### Modos de permissão (`src/agent/mode.rs`)

| `PermissionMode` | `-p` (`cli_mode`, `claude.rs:265`) | Interativo (`claude.rs:174`) | Volta do `init` (`parse_mode`, `:277`) |
|---|---|---|---|
| Agente | `auto` | `auto` | `auto` |
| Yolo | `manual`, e o terra aprova cada pedido | `bypassPermissions` | nunca vem do provedor |
| Manual | `manual` | `manual` | `manual` / `default` |
| AcceptEdits | `acceptEdits` | `acceptEdits` | `acceptEdits` |
| Plan | `plan` | `plan` | `plan` |

Um modo que o terra não conhece (ex.: `dontAsk`) vira `ReportedMode::Unknown`, e um modo ausente vira `Missing` (`src/agent/event.rs:8`). O chat sincroniza o modo mostrado com o que o agente informou (`sync_mode`, `src/tui/chat.rs`). No Yolo o modo não é sincronizado.

Os pedidos de permissão seguem três caminhos (`Run::start`, `claude.rs:528-555`):
- **Yolo**: `--permission-prompt-tool stdio`. A thread de leitura aprova cada `control_request/can_use_tool` na hora.
- **Outros modos**: `stdio`. Cada pedido vira `AgentEvent::PermissionRequest`, e a resposta volta por `Session::respond`, que manda `control_response` com `{"behavior":"allow","updatedInput":…}` ou `deny` (`permission_response`, `:434`).

Qualquer outro `control_request` recebe uma resposta de erro (`unsupported_response`, `:450`).

### Fluxo de uma mensagem

1. **Envio**: `Chat::start` (`src/tui/chat.rs:882`) monta o `Request` com sessão, modelo, esforço, conta e extensões, e chama `adapter.start`. O resultado vai para `self.run`, e o status passa a "Pensando".
2. **Processo**: `Run::start` (`claude.rs:523`) cria o processo com stdin, stdout e stderr em pipe. Uma thread grava o `user_message` e lê o stdout linha a linha (`:583-624`):
   - os pedidos `can_use_tool` são aprovados (Yolo) ou guardados em `pending` e viram `PermissionRequest`;
   - os outros `control_request` recebem erro;
   - o resto passa por `StreamParser::parse`;
   - no `Done`, o stdin fecha, e é isso que encerra o `claude`;
   - se o stdout acabar sem `Done`, a thread manda `Failed` com as 3 últimas linhas do stderr.
3. **Tradução**: `StreamParser` (`src/agent/adapters/claude/stream.rs:16-145`) converte o stream-json em eventos:

| Linha do stream-json | `AgentEvent` (`src/agent/event.rs:17-72`) |
|---|---|
| `system/init` | `Init { session_id, model, permission_mode }` |
| `system/compact_boundary` | `Compacted { tokens: post_tokens }` |
| `system/permission_denied` com `decision_reason_type=classifier` | `Blocked { name, detail, reason, allow }`. `allow` é a regra do `--allowedTools`: `Bash(cmd)`, `WebFetch(domain:host)` ou `Edit(//caminho)` (`allow_rule`, `:234`). |
| `stream_event` `content_block_start` (text) / `text_delta` | `TextStart` / `TextDelta` |
| `assistant` | `Context { tokens, window: None }`, mais `Text` (se o texto não veio em trechos) e `ToolUse { name, detail }` |
| `user` com `tool_result` | `ToolResult { is_error, summary }` (primeira linha, até 80 caracteres) |
| `result` | `Context` com `window` (do `modelUsage[..].contextWindow`), depois `Done { is_error, message, tokens, cache_tokens, denied }` |

   Em `tokens` entram só `input` e `output`; o cache fica em `cache_tokens`.
4. **Tela**: a cada tick, `Chat::tick` chama `session.poll()` (`src/tui/chat.rs:1172`), e `Chat::apply` (`:997-1090`) atualiza o histórico, o status, o modo, a barra de contexto e a fila de permissões (`prompts`).
   - Quando o usuário responde a um pedido `Ask`, o chat chama `run.respond`.
   - Quando permite um `Blocked`, a regra vai para `self.allowed`. Depois do `Done`, `resume_allowed` manda uma mensagem nova com `allowed_tools` (`:948-975`).
   - Se a mensagem de erro fala em autenticação, o chat sugere `/login` (`hint_login`, `:651`).
   - A auto compactação manda o texto `/compact` como prompt (`:809`).
5. **Cancelar**: `Chat::cancel` faz `self.run = None`. O drop de `Run` mata o processo.

### Interface original (`src/agent/adapters/claude/interactive.rs`)

- `COMMANDS` (`:29`) lista os comandos que só existem no `claude` interativo e que o `initialize` do `-p` não informa (tirados do Claude Code 2.1.308). Eles completam a lista da `/`.
- `opens_original` (`:24`) decide se um comando está nessa lista. `BILLING` (`:14`) tem os comandos de pagamento e plano, que o terra não mostra nem roda.
- Quando `command_route` devolve `Original`, `Chat::route_to_provider` (`src/tui/chat.rs:526-570`) monta um `Interactive` (`src/agent/adapter.rs:182`) com conta, modelo, esforço, sub agente, modo, extensões, `session_id` e o comando. Depois guarda o `Command` de `adapter.interactive()` em `pending_original`.
- O loop principal roda esse comando fora da tela (`src/tui.rs:182`, `run_original` em `:115`). Na volta, `finish_original` recarrega o catálogo.
- No Claude, o comando fica `claude [--permission-mode …] [--model] [--effort] [--settings] [--mcp-config] [--resume <sessão>] <comando>`, com `CLAUDE_CODE_SUBAGENT_MODEL` e `CLAUDE_CONFIG_DIR` (`claude.rs:168-206`). No Yolo vai `bypassPermissions`, porque fora da tela o terra não tem como aprovar os pedidos.

### Guard: proteção contra provedor quebrado (`src/agent/guard.rs`)

| Peça | Onde | Efeito |
|---|---|---|
| `guarded(make)` | `:86` | Envolve o adapter em `Guarded`. Se criar o adapter já dá panic, devolve `Broken(reason)`: indisponível, com a dica "O provedor falhou: …". |
| `Guarded` | `:94-177` | Cada método roda dentro de `catch` (`catch_unwind`). Um panic vira o valor padrão do método (`false`, vazio, `None`, o próprio id). Em `start`, vira `Err("{name} falhou: …")`. |
| `GuardedSession` | `:180-195` | Um panic em `poll` vira `AgentEvent::Failed`. Em `respond`, o panic é ignorado. |
| `install_hook` / `take_panics` | `:34`, `:69` | Instalado em `src/tui.rs:60`. Só um panic na thread principal fora de `catch` derruba a tela. Os panics das threads de fundo vão para `PANICS`, que `App::tick` lê (`src/tui/app.rs:849`) e mostra com `chat.provider_failed`, encerrando a resposta em andamento. |

### Para um provedor novo

O que precisa ser implementado nesta área:
1. Um módulo em `src/agent/adapters/`, registrado em `all()` (`src/agent/adapters.rs:11`).
2. Os métodos obrigatórios `name`, `is_available` e `start`. Os outros têm padrão e podem ficar sem implementação no começo.
3. Uma `Session` que:
   - devolva eventos sem bloquear em `poll`;
   - termine sempre com `Done` ou `Failed`;
   - emita `Init` com um `session_id` que o próprio provedor aceite de volta em `Request::session_id` para continuar a conversa;
   - interrompa a resposta no `Drop`.
4. Uma tradução própria de `PermissionMode` nos dois sentidos. O Yolo é do terra: o adapter tem que aprovar sozinho tudo o que o agente pedir.
5. Permissões: emitir `PermissionRequest` e implementar `respond`. Se o provedor bloqueia ferramentas sem perguntar, emitir `Blocked` com uma regra `allow` que ele aceite em `allowed_tools`.
6. Uso: emitir `Context` e `Compacted` quando o provedor informar, e preencher `tokens`, `cache_tokens` e `denied` no `Done`.
7. Traduzir `Contribution` (hooks, MCPs e env das extensões), `subagent_model`, `effort` e `account` para o mecanismo do provedor, ou ignorá-los.
8. Opcionais: `interactive` + `command_route`, `login`, `load_catalog`, `load_usage`, `load_daily`, `accounts`, `skills`, `configured_mode`, `models`, `model_label`.

O que hoje é específico do Claude fora do adapter:
- **Hooks das extensões**: `Hook::event` usa os nomes de evento do Claude Code (ex.: `PreToolUse`, `src/extension/contract.rs:25`). A extensão RTK registra `rtk hook claude` (`src/extension/extensions/rtk.rs:68`), e a ai-memory gera os hooks e o MCP com `--agent claude-code` / `--client claude-code` (`src/extension/extensions/ai_memory.rs:41-42`).
- **`/compact`**: a compactação, manual ou automática, manda o texto `/compact` como prompt e conta que o agente compacte (`src/tui/chat.rs:809`). `INTERFACES` e `TERRA_COMMANDS` partem do princípio de que existem `/compact` e `/login` do Claude (`src/tui/chat/commands.rs:15-22`).
- **Login**: `src/tui/login.rs:19-23` procura na saída do `claude auth login` o texto `visit: `, `Invalid code` e o prompt `Paste code here if prompted >`. `hint_login` procura as palavras `authenticate`, `oauth` e `log in again` nas mensagens de erro (`src/tui/chat.rs:651`).
- **Contas e config**: a conta é o nome de uma pasta `.claude-*` (`src/settings.rs:13`), e `settings.rs:43` fala do `defaultMode` do Claude.
- **Interface original**: o pseudo-terminal foi ajustado para o `claude` interativo (`src/tui/term.rs:2`, `:105`).
- **Aba Orquestrador**: as configurações dos chats delegados ficam em `src/tui/orchestrator.rs` (por enquanto, só salvas).
- **Troca de provedor**: ainda não existe. Só o `default()` é usado (`src/tui/chat.rs:206`), e o router não troca de provedor (`src/tui/chat.rs:1356`).

## Comandos / e menções @

### De onde vem o catálogo

O catálogo (`Catalog`: `commands`, `models`, `default_model`, em `src/agent/adapter.rs:106`) é pedido ao provedor por `ChatAdapter::load_catalog(account)` (`src/agent/adapter.rs:264`). Ele roda em segundo plano e devolve um `Receiver<Result<Catalog, String>>`. Se o provedor não implementar, o padrão é `None`: o chat fica sem lista de comandos e sem modelos no `/model`.

**No Claude** (`src/agent/adapters/claude.rs:210` → `src/agent/adapters/claude/catalog.rs`):
- Uma thread sobe o `claude -p --input-format stream-json --output-format stream-json --verbose --strict-mcp-config --settings {"disableAllHooks":true}`, com `CLAUDE_CONFIG_DIR` apontando para a pasta da conta (`catalog.rs:35-56`).
- Ela manda um `control_request` `initialize` do protocolo do Agent SDK (`catalog.rs:58-65`) e lê o stdout até achar o `control_response` com `request_id = "terra-catalog"`. Depois mata o processo (`catalog.rs:67-78`). Não chama a API nem gasta tokens; a resposta leva uns 1 s.
- O que `parse` lê da resposta (`catalog.rs:82-133`):

| Campo do `initialize` | Vira | Regras |
|---|---|---|
| `commands[].name/description` | `CommandInfo` | `skill = builtin != true`. Descarta nomes vazios, os que começam com `__` e os de pagamento e plano (`interactive::BILLING`). |
| `interactive::COMMANDS` (lista fixa, tirada do Claude Code 2.1.308) | `CommandInfo { skill: false }` | São os comandos do modo interativo que o `-p` não informa (`/help`, `/status`, `/resume`, `/login`…). Só entram se o `claude` não os trouxe (`catalog.rs:94-103`, `claude/interactive.rs:29`). |
| `models[]` com `value == "default"` | `default_model = resolvedModel` | O "default" não vira um modelo da lista. |
| demais `models[]` | `ModelInfo` | `id = value`, `label = displayName`, `description`, `resolved = resolvedModel`, `efforts = supportedEffortLevels`. |

**Cache.** Não há cache em disco. O catálogo fica em memória no `Chat` (`catalog` e `catalog_loading`, `src/tui/chat.rs:151-153`) e vale para a conta atual. Ele é recarregado quando:
- o chat é criado (`chat.rs:222`);
- a conta muda (`chat.rs:236-239`);
- um login termina bem (`chat.rs:641`);
- o usuário volta da interface original, porque um `/plugin` pode ter instalado skills (`chat.rs:591`).

`poll_catalog` (`chat.rs:472-489`) recebe o resultado. Se der erro, fica um `Catalog::default()` vazio e o chat continua funcionando. O `guard` protege a chamada contra panic (`src/agent/guard.rs:141`).

**O que o terra acrescenta** (`src/tui/chat/commands.rs`):
- `INTERFACES = clear, compact, model, login, usage` (`commands.rs:17`): são os comandos com interface própria do terra.
- `TERRA_COMMANDS` (`commands.rs:20`): só o `/login`. Ele entra via `with_terra` (`commands.rs:32`) quando o agente não o informa; se o agente informar, fica a versão dele. No Claude ele já vem pelo `interactive::COMMANDS`.
- Filtro da lista (`commands.rs:47`): primeiro o começo do nome, depois parte do nome, depois parte da descrição; os que têm interface vêm antes. Os sem interface aparecem esmaecidos com "· padrão do router" (`commands.rs:119-122`).
- A Config também usa `catalog.commands` com `skill == true` para listar as skills embutidas e de plugin (`src/tui/app.rs:510`).

### Roteamento de um comando

O caminho de entrada é `Outcome::Command(name)` → `Chat::run_command` (`chat.rs:445`, `chat.rs:493`). Quem digita `/x` e dá Enter passa por `send` (`chat.rs:844-862`).

| Comando | Para onde vai | Onde |
|---|---|---|
| `/clear` | terra: conversa nova (zera sessão, histórico e contexto) | `chat.rs:495`, `chat.rs:777` |
| `/compact` | terra decide quando; manda o prompt `"/compact"` ao agente na sessão atual. Também roda sozinho pela auto compactação. | `chat.rs:795-811`, `chat.rs:1281` |
| `/login` | terra: roda `adapter.login(account)` fora da tela (no Claude, `claude auth login --email …`) e depois recarrega o catálogo | `chat.rs:497`, `chat.rs:597`, `claude.rs:142` |
| `/usage` | terra: abre a aba Consumo | `chat.rs:499` |
| `/model` | terra: abre o `ModelMenu` | `chat.rs:500-507` |
| outro, com o agente respondendo | aviso "Espere o agente terminar…" | `chat.rs:510` |
| outro | `route_to_provider`, que consulta `adapter.command_route(name)` | `chat.rs:526-572` |

`CommandRoute` (`adapter.rs:199`). O padrão do trait é sempre `Agent` (`adapter.rs:248`):
- **`Agent`**: o texto `/nome …` vai como prompt para o agente, que responde no chat.
- **`Original`**: monta `Interactive { account, model, effort, subagent_model, mode, extensions, session_id, command }`, chama `adapter.interactive(cwd, …)` e guarda em `pending_original`. A tela principal roda isso fora do terra (`chat.rs:544-570`). No Claude é o `claude` interativo com `--resume <sessão>` e o comando como argumento (`claude.rs:168-206`).
- **`Blocked`**: não roda e avisa "pagamento e plano ficam no {provedor}" (`chat.rs:537`).

Regra do Claude (`claude.rs:154-162`): nomes em `BILLING` → `Blocked`; nomes em `interactive::COMMANDS` → `Original` (`claude/interactive.rs:24`); o resto (embutidos do `-p` e skills) → `Agent`.

Detalhes do input:
- A `/` só abre a lista no começo do texto (`input.rs:269`).
- Enter num comando de interface roda a interface.
- Tab num comando que não é skill nem tem interface completa `/nome ` para o usuário digitar os argumentos (`input.rs:468-479`, `input.rs:530`).
- Uma skill escolhida na lista vira um badge `/nome` no input, em vez de rodar na hora (`input.rs:514-520`).
- `/model opus` digitado com argumentos não casa com `has_interface`, então vai para `route_to_provider`/agente (`chat.rs:849-862`).

### Menções `@`

- Digitar `@` abre a lista, menos logo depois de um caractere alfanumérico, como num e-mail (`input.rs:264-273`).
- O primeiro `@` começa a observar o diretório atual com `FileIndex::watch` (`input.rs:547-550`). O `FileIndex` lista arquivos e pastas respeitando o `.gitignore`, esconde os ocultos, vai até 50 000 entradas e se atualiza via `notify`, ou a cada 2 s se não houver observador (`src/files.rs:1-60`).
- A lista não depende do provedor: é o sistema de arquivos local.
- Filtro (`mention.rs:119-140`): começo do nome do arquivo > parte do nome > parte do caminho > letras na ordem. Mostra até 200 resultados; sem consulta, os caminhos mais rasos vêm primeiro.
- O item escolhido vira um badge. No buffer ele é um caractere da área de uso privado do Unicode (`mention.rs:22-40`), que aponta para um `Mention` (`mention.rs:44`).

O que cada badge manda ao agente (`mention::prompt_text`, `mention.rs:83`, chamado por `Input::text`, `input.rs:136`):

| Badge | Na tela | Vai para o agente |
|---|---|---|
| `Mention::File(path)` | `@/src/main.rs` | `@src/main.rs` (sintaxe de arquivo do Claude Code) |
| `Mention::Skill(name)` | `/docs` | `/docs`, em qualquer ponto do texto |
| `Mention::Paste` | `[Pasted text #1 +N lines]` | o texto colado inteiro (acima de 100 caracteres, `PASTE_LIMIT`) |

### Menu do `/model`

Fica em `src/tui/chat/model_menu.rs`. A tela é do terra; os modelos e os níveis de esforço vêm de `catalog.models`.

- **Itens.** Primeiro "Default (\<modelo padrão\>)", depois `N. <label> - <description>`, com até 5 visíveis por vez (`model_menu.rs:140`, `model_menu.rs:311-326`). O modelo padrão é o salvo na Config ou o `catalog.default_model` (`chat.rs:306`). Os ids casam por `id` ou por `resolved` (`model_menu.rs:397`).
- **Esforço.** ←/→ percorrem o `ModelInfo::efforts` do modelo em foco (`model_menu.rs:241`). `fit_effort` (`model_menu.rs:165`) garante um nível que o modelo aceite: o atual, senão `high` (`PREFERRED_EFFORT`), senão o último da lista. Os rótulos em português saem de `effort_label` (`low→baixo`, `medium→médio`, `high→alto`, `xhigh→muito alto`, `max→máximo`, `model_menu.rs:402`).
- **Alvos** (`Target`, `model_menu.rs:40`):

| Alvo | Quem abre | Efeito | Esforço |
|---|---|---|---|
| `Session` | `/model` (`chat.rs:504`) | `session_model` e `session_effort` desta conversa (`chat.rs:685-693`) | sim |
| `Default` | Enter no Default do menu da conversa (`model_menu.rs:213`) | `default_choice`, que vai para a Config via `take_default_choice` (`chat.rs:694-702`, `chat.rs:713`). Esc volta ao menu da conversa. | sim |
| `Subagent` | Config (`src/tui/config.rs:276`, `ModelMenu::subagents`) | modelo dos sub agentes; no Claude vai em `CLAUDE_CODE_SUBAGENT_MODEL` (`claude.rs:42`). Default é "o agente escolhe" (`Outcome::Unset`). | não |
| `Delegated` | aba Orquestrador (`src/tui/orchestrator.rs`, `ModelMenu::delegated`) | modelo e esforço dos chats delegados, salvos no `config.json` global | sim |

- **Para o provedor.** O modelo e o esforço vão no `Request.model`/`effort`: primeiro os da sessão, senão os da Config (`chat.rs:886-899`). No Claude viram `--model` e `--effort` (`claude.rs:336-339`), e também na interface original (`claude.rs:180-185`).
- **Rótulos.** `Chat::model_label_of` (`chat.rs:734-748`): se o `ModelInfo` tem `resolved`, usa `adapter.model_label(resolved)`; senão, o `label` do catálogo; senão, o próprio id. No Claude, `display_model` transforma `claude-opus-5-5` em `Opus - 5.5` e tira as datas do id (`claude.rs:249`, `claude.rs:685-702`). O padrão do trait devolve o id (`adapter.rs:258`).
- **Antes do catálogo chegar.** O menu mostra "Carregando os modelos do agente..."; com o catálogo vazio, "Nenhum modelo informado pelo agente." (`model_menu.rs:299-309`). A Config usa `adapter.models()` como reserva (no Claude, os apelidos `opus`/`sonnet`/`haiku`, `claude.rs:239`; uso em `chat.rs:758-774`).

### Para um provedor novo

**O que implementar no `ChatAdapter`:**
- `load_catalog(account)`: devolver `Catalog` com:
  - `commands`, marcando `skill: true` para skills, comandos do usuário e de plugins;
  - `models`, com `id` (o que o provedor aceita), `label`, `description`, `resolved` (se o id for apelido) e `efforts` (do menor para o maior; vazio se não houver esforço);
  - `default_model`.

  Rodar em thread, sem gastar tokens, e por conta.
- `command_route(name)`: dizer o que é `Agent`, `Original` ou `Blocked`. Sem isso, tudo vai para o agente como prompt.
- `interactive(cwd, Interactive)`: obrigatório se houver rotas `Original`. Deve abrir a interface original na sessão (`session_id`) com o comando.
- `login(account)`: para o `/login` do terra. Sem ele, o comando não tem o que rodar.
- `model_label(id)`: nome amigável do modelo resolvido.
- `models()`: lista de reserva para a Config enquanto o catálogo não chega.
- `start(Request)`: aplicar `model`, `effort` e `subagent_model`.

**O que hoje é específico do Claude e precisa ser revisto:**
- O protocolo `initialize` do Agent SDK e os campos `commands`/`builtin`/`models`/`value`/`resolvedModel`/`displayName`/`supportedEffortLevels` (`catalog.rs`).
- As listas fixas `interactive::COMMANDS` e `interactive::BILLING`, que valem para uma versão específica do Claude Code (2.1.308).
- O id `"default"` que marca o modelo padrão (`catalog.rs:24`).
- O `/compact` do terra manda o texto literal `"/compact"` ao agente (`chat.rs:810`). O provedor precisa entender isso, ou o terra precisa de um gancho próprio para compactar.
- As menções saem como `@caminho` e `/skill` dentro do prompt (`mention.rs:83`). O provedor precisa entender essa sintaxe, ou falta um gancho no adapter para traduzir.
- `INTERFACES`/`TERRA_COMMANDS` usam nomes fixos (`clear`, `compact`, `model`, `login`, `usage`). Se o provedor tiver um comando com o mesmo nome e outra função, o terra intercepta.
- Os rótulos de esforço (`effort_label`) e o esforço preferido `high` presumem os níveis do Claude (`low`, `medium`, `high`, `xhigh`, `max`). Níveis com outros nomes aparecem crus.
- O modelo dos sub agentes vai por `CLAUDE_CODE_SUBAGENT_MODEL`, e a mensagem de `Blocked` fala em "pagamento e plano".

## Consumo

O terra mostra o consumo do provedor em três lugares: a **aba Consumo** (os limites da conta e o gráfico por dia/hora), a **barra de consumo** do chat (quanto resta da janela de 5 h) e os **badges/barras do chat** (tokens da sessão e tamanho do contexto). Os dois primeiros saem de funções do `ChatAdapter` que rodam em segundo plano. O terceiro sai dos eventos do stream.

### Contrato (`src/agent/adapter.rs`)

| Item | Onde | O que é |
|---|---|---|
| `UsageWindow` | `src/agent/adapter.rs:116` | `label`, `percent` (0–100 usado), `resets_at` (Unix s, opcional), `detail` (texto no lugar do reset), `session` (marca a janela da barra do chat) |
| `Usage` | `src/agent/adapter.rs:133` | `plan` opcional e `windows`; `session()` acha a janela com `session = true` (`:141`) |
| `HourTokens` / `DailyTokens` | `src/agent/adapter.rs:148`, `:157` | tokens (entrada+saída) e cache (criação+leitura) por dia, com 24 horas no fuso local; `DailyTokens::empty` (`:170`) |
| `fn load_usage(account)` | `src/agent/adapter.rs:270` | `Option<Receiver<Result<Usage, String>>>`; o padrão é `None` (sem consumo) |
| `fn load_daily(account, days)` | `src/agent/adapter.rs:292` | `Option<Receiver<Result<Vec<DailyTokens>, String>>>`: `days` dias até hoje, o mais antigo primeiro, dias vazios com 0; o padrão é `None` |
| `AgentEvent::Context { tokens, window }` | `src/agent/event.rs:56` | tamanho do contexto na última chamada e o limite do modelo, quando o provedor informa |
| `AgentEvent::Compacted { tokens }` | `src/agent/event.rs:59` | a conversa foi compactada (tamanho depois, se o provedor informa) |
| `AgentEvent::Done { tokens, cache_tokens, .. }` | `src/agent/event.rs:61` | tokens da resposta, com o cache separado |

O `guard` envolve `load_usage` e `load_daily` em `catch`: se o adapter der pânico, o resultado vira `None` (`src/agent/guard.rs:145`, `:159`).

### De onde o Claude tira o consumo

**Conta → pasta.** As duas funções resolvem a pasta da conta com `account_dir` (`src/agent/adapters/claude.rs:61`, que valida `~/.claude-<nome>` por `accounts::config_dir` em `src/agent/adapters/claude/accounts.rs:106`). Sem conta escolhida, usam `default_config_dir()`: `$CLAUDE_CONFIG_DIR` ou `~/.claude` (`src/agent/adapters/claude.rs:481`). Se a conta não existe, o resultado é `None` (`src/agent/adapters/claude.rs:217`, `:226`).

**Limites (`load_usage` → `usage::load`, `src/agent/adapters/claude/usage.rs:40`):**
- **Endpoint:** `GET https://api.anthropic.com/api/oauth/usage` com o header `anthropic-beta: oauth-2025-04-20` (`src/agent/adapters/claude/usage.rs:24-25`). Não chama o modelo nem gasta tokens.
- **Credencial:** `<pasta>/.credentials.json` → `claudeAiOauth.accessToken`. O plano vem de `subscriptionType`, com a inicial maiúscula (`src/agent/adapters/claude/usage.rs:178`).
- **Chamada:** feita pelo `curl` com `--max-time 10`. Os headers vão pelo stdin (`-H @-`) para o token não aparecer no `ps` (`src/agent/adapters/claude/usage.rs:125`).
- **Status HTTP:**
  - 200: faz o parse;
  - 429: espera de 60 s, que dobra a cada 429 seguido, até 15 min;
  - 401/403: "o login do Claude expirou… use /login";
  - outros: o erro é repassado à tela (`src/agent/adapters/claude/usage.rs:162-169`).
- **Cache por pasta de conta:** a resposta vale 50 s (`FRESH`), então a aba e a barra, que pedem juntas, recebem a mesma resposta (`src/agent/adapters/claude/usage.rs:50-102`).
- **Parse (`src/agent/adapters/claude/usage.rs:203`):**
  - As chaves `five_hour`, `seven_day`, `seven_day_opus` e `seven_day_sonnet` viram janelas, nesta ordem, com `utilization` e `resets_at` em ISO 8601. Só `five_hour` tem `session = true` (`src/agent/adapters/claude/usage.rs:31-36`, `:218`).
  - Janelas `null` são ignoradas.
  - `extra_usage` só entra se `is_enabled`. Vira "Crédito extra (mensal)", com `detail` em dinheiro (`src/agent/adapters/claude/usage.rs:228`).

**Por dia (`load_daily` → `daily::load`, `src/agent/adapters/claude/daily.rs:31`):**
- **Fonte:** os transcripts do Claude Code em `<pasta da conta>/projects/**/*.jsonl`, em qualquer profundidade, inclusive `subagents/` (`src/agent/adapters/claude/daily.rs:146`). As conversas que o terra roda pelo `claude` também entram.
- **Arquivos lidos:** os que não mudaram desde antes do primeiro dia são pulados (pelo `mtime`, `src/agent/adapters/claude/daily.rs:49-59`).
- **Linhas lidas:** só as do tipo `assistant` com `message.usage` (`src/agent/adapters/claude/daily.rs:112`).
- **Sem contagem dupla:** cada resposta conta uma vez por `message.id:requestId` (`src/agent/adapters/claude/daily.rs:119-126`).
- **Tokens:** `input_tokens + output_tokens`.
- **Cache:** `cache_creation_input_tokens + cache_read_input_tokens` (`src/agent/adapters/claude/daily.rs:27-28`).
- **Dia e hora:** pelo `timestamp`, no fuso local.
- **Erros:** essa função nunca devolve `Err`.

### Tokens da sessão e contexto (stream do `claude -p`)

O `StreamParser` (`src/agent/adapters/claude/stream.rs`) emite os eventos assim:

| Evento | Origem no stream | Linha |
|---|---|---|
| `Context { tokens, window: None }` | `usage` de cada mensagem `assistant` | `src/agent/adapters/claude/stream.rs:87` |
| `Context { tokens, window }` | no `result`: a última de `usage.iterations` (ou o próprio `usage`); `window` = `modelUsage[<modelo do init>].contextWindow`, ou o maior dos modelos usados; não sai depois de uma compactação | `src/agent/adapters/claude/stream.rs:129-162` |
| `Compacted { tokens }` | `system/compact_boundary`, `compact_metadata.post_tokens` | `src/agent/adapters/claude/stream.rs:46` |
| `Done { tokens, cache_tokens }` | `usage` do `result`: entrada+saída, e o cache à parte | `src/agent/adapters/claude/stream.rs:185-211` |

`context_tokens` soma só `input_tokens + output_tokens`, sem o cache (`src/agent/adapters/claude/stream.rs:149`).

**Ponto de atenção:** com o prompt caching, a maior parte do contexto vem em `cache_read_input_tokens`. Por isso a barra de contexto mostra um tamanho bem menor que o contexto real; no teste, 83 tokens contra cerca de 1.100 de cache (`src/agent/adapters/claude/stream.rs:381-401`). Isso é de propósito, mas vale revisar.

**Como o chat usa (`src/tui/chat.rs`):**
- **Eventos:**
  - `Context` grava `context` e, quando vem, `context_window` (`src/tui/chat.rs:1042`);
  - `Compacted` troca o `context` e desarma a auto compactação (`src/tui/chat.rs:1049`);
  - `Done` soma em `tokens_used` e `cache_tokens_used` (`src/tui/chat.rs:1058-1066`).
- **Badge "Consumo: N Tokens":** mostra `tokens_used`, sem o cache (`src/tui/chat.rs:1259`).
- **Consumo geral ao lado do "Sair":** a soma de todos os chats, inclusive os fechados (`src/tui/app.rs:1021`).
- **MCP:** `chat_stats` expõe `tokens` e `cache_tokens` (`src/tui/app.rs:1176`).
- **Janela de contexto:** `context_window()` usa a informada ou `DEFAULT_CONTEXT_WINDOW = 200_000` (`src/tui/chat.rs:61`, `:1267`).
- **Barra de contexto:** `context / compact_tokens` (`src/tui/chat.rs:1295`). O ponto de compactar é `limit (Config ou janela) × compact_at%` (`src/settings.rs:136-144`). Ao chegar nele, a auto compactação roda (`src/tui/chat.rs:1281`).

### Barra de consumo (janela de 5 h)

- **`SessionUsage` (`src/tui/usage.rs:602`):** chama só `load_usage` e guarda `usage.session()` (`src/tui/usage.rs:656`). Com erro, mantém a última janela.
- **Quando recarrega:**
  - na primeira vez e a cada `STALE` = 60 s (`src/tui/usage.rs:43`);
  - quando a conta muda;
  - quando o `resets_at` passa (`reset_since`, `src/tui/usage.rs:680`).
- **Quem chama:** `App::tick`, só com `settings.usage.visible`; a janela é repassada a todos os chats (`src/tui/app.rs:864-875`).
- **Desenho:** `usage_bar` mostra quanto **resta** (`100 − percent`), com o texto de `session_label`, por exemplo `58% · reseta 15:00` (`src/tui/chat.rs:1313`, `src/tui/usage.rs:688`). Ela fica à esquerda da barra de contexto (`src/tui/chat.rs:1390`).
- **Sem janela `session`, a barra não aparece.**

### Aba Consumo (`src/tui/usage.rs`)

- **Carga:** `UsageView` só carrega com a aba aberta (`src/tui/app.rs:858-863`). `refresh` chama `load_usage(account)` e `load_daily(account, 14)` (`src/tui/usage.rs:131-156`), só se `is_available()`.
- **Atualização:**
  - a cada 60 s;
  - ao trocar de conta;
  - quando uma janela zera;
  - com `r`, na hora.
- **Invalidação:** `invalidate()` depois de login, da interface original ou do terminal (`src/tui/app.rs:597-608`, `:659`).
- **Mensagens na tela (`src/tui/usage.rs:462-525`):**

  | Caso | O que aparece |
  |---|---|
  | Route | `Claude · <conta> · <plano>` |
  | Cada janela | barra com o que resta, "Reseta dd-mm-aaaa HH:MM · zera em…" ou o `detail` |
  | Lista de janelas vazia | "O provedor não informou limites para esta conta." |
  | `load_usage` = `None` | "Este provedor não informa consumo." |
  | Erro | texto em vermelho |

- **Gráfico "Consumo por dia":** vem do `load_daily`. Clicar num dia mostra as horas dele; `c` ou os botões alternam sem/com cache (`src/tui/usage.rs:259`). Se `load_daily` der `None` ou erro, o gráfico some e as barras ficam (`src/tui/usage.rs:168-182`).
- **Atalho:** o `/usage` do chat abre a aba (`src/tui/chat.rs:499`).

**No Antigravity** (`src/agent/adapters/antigravity/daily.rs`, `stream.rs`), medido no `agy` 1.4.8:

- **Onde o `agy` guarda o uso:** em `~/.gemini/antigravity-cli/conversations/<id>.db` (SQLite com WAL; tabelas `steps`, `gen_metadata`, `executor_metadata`...). As colunas são protobuf sem esquema público e sem datas legíveis; o `gen_metadata` traz os contadores de cada chamada e a janela do modelo, mas só por campos numerados que a Google pode mudar. O `conversation_summaries.db` tem só título, contagem de passos e hora. O terra **não** lê esses bancos.
- **Tokens por dia:** o terra registra o que ele mesmo vê. O parser do stream junta o `usage` de cada `agent_response` terminado (`StreamParser::take_spends`), e a thread da resposta grava na hora uma linha em `~/.terraforme/antigravity-usage.jsonl` (`at` em Unix s, `conversation`, `step`, `model`, `tokens` = entrada + saída, `cache` = leitura de cache). `load_daily` soma o arquivo por dia e hora no fuso local, como o do Claude, contando uma vez cada (conversa, passo). Não entra o uso fora do terra (a interface original do `agy`) nem o anterior ao arquivo. Sem conta: o `account` é ignorado.
- **Janela de contexto:** o stream e o `agy models` não a informam. O `Context` do Antigravity leva `window` de uma tabela por família do modelo (`context_window`): `gemini-*` 256k, `claude-*` 160k, `gpt-oss*` 80k, que são os limites que o `agy` grava nas conversas. Fora da tabela, `window` é `None` e vale o padrão de 200k.

### Para um provedor novo

**O que implementar (tudo é opcional):**
1. **Tokens e contexto no stream** (mínimo recomendado):
   - emitir `Done { tokens, cache_tokens }` no fim de cada resposta;
   - emitir `Context { tokens, window }` a cada chamada ao modelo, com `window` quando souber;
   - emitir `Compacted` se o provedor compactar.
   
   Sem `Context`, a barra de contexto não aparece e a auto compactação nunca dispara. Sem `window`, o limite fica em 200k.
2. **`load_usage(account)`:** devolver um canal com `Usage` e rodar numa thread, porque a tela faz `try_recv` a cada tick.
   - Marcar `session = true` na janela que deve ir para a barra do chat.
   - Pôr `resets_at` para a tela recarregar quando a janela zerar.
   - Fazer o próprio cache ou backoff: a aba e a barra pedem a cada 60 s, e juntas.
3. **`load_daily(account, days)`:** devolver exatamente `days` itens, o mais antigo primeiro, com zeros nos dias sem uso e horas no fuso local.
4. **`login(account)`:** opcional, para a mensagem de credencial expirada ter saída.

**Sem consumo:** as funções padrão devolvem `None`. A aba mostra "Este provedor não informa consumo.", sem gráfico; a barra de consumo some. Os badges de tokens e a barra de contexto continuam funcionando se o stream emitir `Done` e `Context`.

**O que hoje é específico do Claude:**
- O endpoint `/api/oauth/usage`, o header beta e o `.credentials.json` (`claudeAiOauth`).
- O `curl` como cliente HTTP.
- As chaves `five_hour` e `seven_day*`, os rótulos em português e o `extra_usage`.
- O formato dos transcripts `projects/**/*.jsonl` e a contagem única por `message.id:requestId`.
- As pastas de conta `~/.claude-<nome>` e o `CLAUDE_CONFIG_DIR`.
- Os campos do stream: `usage.iterations`, `modelUsage.contextWindow`, `compact_boundary.post_tokens`.
- Nos textos da tela:
  - o rótulo "Sessão (5 h)" e a ideia de uma janela de sessão (`UsageWindow.session`);
  - a mensagem de 429 ("o Claude limitou…");
  - as palavras que `hint_login` procura para sugerir o `/login` (`authenticate`, `oauth`, `log in again`, `src/tui/chat.rs:651`).
- O `DEFAULT_CONTEXT_WINDOW = 200_000`, tirado dos modelos Claude (`src/tui/chat.rs:61`).

## Extensões, MCP e skills

### Contrato de extensão

Cada extensão é uma estratégia que implementa o trait `Extension` (`src/extension/contract.rs:107`). O terra conhece só esse contrato. Para registrar uma extensão nova basta incluí-la em `extensions::all()` (`src/extension/extensions.rs:25`).

| Peça | Onde | O que é |
|---|---|---|
| `Extension` | `contract.rs:107-130` | Métodos `id` (o que fica salvo no `config.json`), `name`, `description`, `homepage`, `status`, `contribution()` e `load_report()` (opcional, para a aba Extensões) |
| `Status` | `contract.rs:11` | `Installed { version }` ou `Missing` |
| `Contribution` | `contract.rs:52-58` | `hooks: Vec<Hook>`, `mcp_servers: Vec<McpServer>`, `env: Vec<(String,String)>`. `merge()` concatena as listas (`:62`) |
| `Hook` | `contract.rs:29-33` | `event` + `matcher` + `command`. **O formato é o do Claude Code** (ex.: `PreToolUse`, matcher `Bash`) |
| `McpServer` | `contract.rs:38-47` | stdio (`command`, `args`) ou HTTP (`url`, `headers`). Os headers podem citar `${VAR}` de `Contribution::env` |
| `Report`/`Stat` | `contract.rs:71-104` | Relatório que a aba Extensões mostra (`src/tui/extensions.rs`) |
| `Registry` | `src/extension/registry.rs:7-43` | `detect()` cria todas as extensões. `contribution(enabled)` junta, na ordem do registro, só as ativas **e** instaladas; a extensão que falha vira aviso |
| Helpers | `src/extension/extensions.rs:33-91` | `find_program` (procura no PATH), `run` (timeout de 10 s), `version` (`--version`), `first_json` (pula comentários) |

### Da extensão ao processo do agente

1. A Config muda o modo de uma extensão (inativa → ativa nesta conta → usar sempre): `handle_extension_key` faz `settings.toggle(id)` (`src/tui/config.rs`). Os ids ficam em `ExtensionSettings.enabled` (da conta) e `ExtensionSettings.always`, no banco (`Db::extension_settings` / `Db::set_extension`, `src/db.rs`), por conta do agente (`App::extension_account`).
2. `App::apply_extensions` (`src/tui/app.rs`) chama `registry.contribution(&extensions.active())`.
   - O resultado vai para cada chat com `chat.set_extensions` (`src/tui/chat.rs:351`).
3. O chat passa a Contribution em `Request.extensions` (`src/agent/adapter.rs:52`, `src/tui/chat.rs:900`). A interface original recebe a mesma coisa em `Interactive.extensions` (`adapter.rs:190`, `app.rs:629`).


### Tradução para o Claude (`src/agent/adapters/claude.rs`)

| Contribution | Chat (`Run::start`, `:523-559`) | Interface original (`interactive`, `:168-206`) |
|---|---|---|
| `hooks` | Vai sempre `--settings <json>` (`cli_args`, `:329`). `hook_settings` (`:351-368`) agrupa por evento: `{"hooks":{evento:[{matcher, hooks:[{type:"command",command}]}]}}`. **Sem nenhum hook, vai `{"disableAllHooks":true}`**, o que desliga também os hooks do usuário | `--settings` só quando há hooks (`:190-192`). Sem eles, os hooks do usuário continuam ligados |
| `mcp_servers` | `mcp_args` (`:372-406`) gera `--mcp-config '{"mcpServers":{...}}'`: `type:"stdio"` (command/args) ou `type:"http"` (url e, se houver, headers). Os servidores somam aos do usuário: não se usa `--strict-mcp-config` | Igual (`:193`) |
| `env` | `cmd.envs(...)` (`:541`). O `claude` resolve os `${NOME}` dos headers | Igual (`:194`) |
| conta | `CLAUDE_CONFIG_DIR=<pasta da conta>` (`:557`) | Igual (`:202`) |

O terra **não grava arquivos** do Claude. Tudo vai por flags e variáveis de ambiente de cada processo.

### RTK e ai-memory

| Extensão | Detecção | O que injeta no Claude | Relatório |
|---|---|---|---|
| **RTK** (`src/extension/extensions/rtk.rs`) | `rtk` no PATH (`:29`) | Um hook fixo, `PreToolUse` com matcher `Bash` e comando `rtk hook claude` (`:63-73`). Ele reescreve o comando do Bash (`git status` → `rtk git status`). O argumento `claude` é específico do provedor | `rtk gain --all --format json`, com gráfico de 14 dias (`:75-133`) |
| **ai-memory** (`src/extension/extensions/ai_memory.rs`) | `ai-memory` no PATH (`:29`) | Roda uma vez (num `OnceCell`) `ai-memory install-hooks --agent claude-code` e `install-mcp --client claude-code` (`:39-48`). `parse_hooks` (`:95-118`) lê o bloco `hooks` no formato dos settings do Claude Code: SessionStart, prompts, ferramentas, Stop etc. `parse_mcp` (`:122-143`) lê o `mcpServers` sugerido para o `.claude.json` (stdio, ex.: `ai-memory mcp-bridge`). A configuração do Claude não é alterada (não usa `--apply`) | `ai-memory status --json` (`:146-188`) |

### Como a Config lê as skills do provedor

O trait `ChatAdapter` tem o método opcional `skills(cwd, account) -> Vec<ProviderSkill>` (`src/agent/adapter.rs`), que por padrão devolve vazio. O tipo está em `adapter.rs: `ProviderSkill { name, description, scope, path }`.

**MCP do Claude** (`claude.rs:102-112` → `src/agent/adapters/claude/mcp.rs:14-63`):

| Escopo | Fonte |
|---|---|
| `usuário` | `mcpServers` do `.claude.json` da conta. Sem conta, `$CLAUDE_CONFIG_DIR/.claude.json` ou `~/.claude.json` (`claude.rs:474-478`) |
| `local` | `projects["<cwd>"].mcpServers` do mesmo `.claude.json` |
| `projeto` | `mcpServers` do `<cwd>/.mcp.json` |

`App::mcp_servers` (`src/tui/app.rs:528-561`) junta essa lista com os servidores da Contribution (origem `terraforme`). Um servidor que aparece nos dois fica numa linha só, com as duas origens.

**Skills do Claude** (`claude.rs:117-130` → `src/agent/adapters/claude/skills.rs:25-59`), nesta ordem, removendo nomes repetidos:

| Escopo | Pasta |
|---|---|
| `usuário` | `<config>/skills/*/SKILL.md` |
| `projeto` | `.claude/skills` do `cwd` e de cada pasta acima, até o home (`:32-34`) |
| `agentes` | `~/.agents/skills` |
| `claude.ai` | `<config>/skills/synced/<organizationUuid>_<accountUuid>`. O par vem de `oauthAccount` no `.claude.json` (`:63-69`) |
| `plugin` | `<config>/plugins/installed_plugins.json`: plugins com `scope:"user"` ou `projectPath == cwd` (`:93-117`) e plugins sincronizados em `<config>/plugins/synced/<bucket>/*/.claude-plugin/plugin.json` (`:73-89`). Em todos, lê `<installPath>/skills`. O nome fica `plugin:skill` |

`name` e `description` vêm do frontmatter do `SKILL.md` (`:147-168`). Sem frontmatter, o nome é o da pasta.

**Catálogo:** `App::skills` (`app.rs:496-523`) acrescenta os comandos do catálogo com `skill == true`, com origem `<provedor> · catálogo`. Para o Claude, `skill` é verdadeiro para todo comando com `builtin != true` na resposta do `initialize` (`src/agent/adapters/claude/catalog.rs:82-93`).

### Para um provedor novo

**O que implementar no adapter:**
- [ ] Traduzir a `Contribution` no `start` e no `interactive`:
  - `hooks` → mecanismo de hooks do provedor. Se ele não tiver, ignore os hooks e avise.
  - `mcp_servers` → configuração MCP do provedor (flag, arquivo temporário ou variável de ambiente), em stdio e HTTP com headers.
  - `env` → variáveis do processo. Um segredo nunca vai pela linha de comando.
- [ ] Garantir que `${VAR}` nos headers seja resolvido. O `claude` faz isso sozinho; outro provedor pode exigir que o adapter substitua.
- [ ] Permissão delegada: um equivalente a `--permission-prompt-tool mcp__terra__permission`, ou outra forma de rotear os pedidos para o MCP do terra.
- [ ] `mcp_servers()` e `skills()` lendo os arquivos do provedor (escopos usuário, local, projeto e plugins), para a Config. São opcionais: sem eles, a lista fica vazia.
- [ ] No catálogo, marcar `CommandInfo.skill` para a Config listar as skills que só o provedor conhece.

**O que hoje é específico do Claude:**
- **`Hook.event`/`matcher`** usam os nomes de evento e ferramenta do Claude Code (`contract.rs:25-27`). Outro provedor precisa de um mapa de eventos ou de um contrato neutro.
- **RTK**: o comando `rtk hook claude` e o matcher `Bash` estão fixos no código (`rtk.rs:65-69`).
- **ai-memory**: `--agent claude-code` e `--client claude-code` estão fixos (`ai_memory.rs:41-42`), e os parsers esperam o formato de settings e `.claude.json` do Claude. Para outro provedor, a extensão precisa saber o provedor ativo, e hoje `contribution()` não recebe nenhum parâmetro.
- **`disableAllHooks`** quando não há hooks é uma política do adapter do Claude (`claude.rs:351-354`): no chat, os hooks do usuário ficam desligados.
- **Tudo em `claude/mcp.rs` e `claude/skills.rs`**: `.claude.json`, `.mcp.json`, `.claude/skills`, `~/.agents/skills`, buckets do claude.ai, `installed_plugins.json`, `.claude-plugin/plugin.json` e o nome `plugin:skill`.
- **`CLAUDE_CONFIG_DIR`**: é a variável que seleciona a conta, e muda de onde vêm MCP, skills e settings.
- **Prefixo `mcp__<server>__<tool>`** da ferramenta de permissão (`claude.rs:553`).

### Antigravity (`agy`): extensões e MCP

O `agy` não tem flag nem variável de ambiente para MCP ou hooks por processo (nem para trocar a pasta de configuração: só o `HOME`, que quebraria o `git`, o `ssh` e as ferramentas do agente). O que ele tem é a descoberta de plugins em `<pasta>/.agents/plugins/<nome>/` de cada pasta do `--add-dir`, com `plugin.json`, `mcp_config.json` e `hooks.json`. É por aí que o terra entrega a Contribution, sem tocar no `~/.gemini` nem no projeto (`src/agent/adapters/antigravity/plugin.rs`).

| Contribution | O que o adapter faz |
|---|---|
| `mcp_servers` | `Overlay::create` grava o plugin `terraforme` numa pasta temporária (0700, arquivo 0600) e o `start` e o `interactive` passam `--add-dir <pasta>`. stdio vira `command`/`args`; HTTP vira `url`/`headers`. Um servidor com o nome de um do `~/.gemini/config/mcp_config.json` não é repetido |
| `${VAR}` nos headers | O `agy` não resolve (manda o texto literal). O `plugin::expand` troca pelo `env` da Contribution e, na falta, pelo ambiente do terra |
| `env` | `cmd.envs(...)`, como no Claude |
| `hooks` | **Não vão.** São do Claude Code (evento, matcher `Bash`, `rtk hook claude`); o `agy` manda `toolCall.name = run_command`, `args.CommandLine` e espera `{"decision": ...}`. O `hooks.json` do plugin existe e funciona (named hooks: `{"nome": {"PreToolUse": [{"matcher", "hooks": [...]}]}}`; eventos `SessionStart`, `PreInvocation`, `PostInvocation` e `Stop` são listas planas), mas faltaria um mapa de payload |
| ciclo de vida | `Run` guarda o `Overlay` e o apaga ao soltar (depois de matar o processo). Na interface original, o `Command` não carrega a pasta: ela fica numa variável global e é apagada quando a próxima abre |

Como o `agy` expõe o MCP ao modelo:
- O servidor do plugin se chama `<plugin>_<servidor>` (`terraforme_terra`), e as ferramentas são chamadas pela ferramenta genérica `call_mcp_tool` (`ServerName`, `ToolName`, `Arguments`). Não existe o `mcp__terra__permission` do Claude.
- No `agy -p` no modo Manual (`request-review`), uma chamada MCP é **negada** (`user denied permission for mcp(...)`); um hook `PreToolUse` com `decision: allow` não muda isso. Nos modos `auto` e `accept-edits` ela passa.

**Permissões do Antigravity** (`antigravity/permission.rs`, `antigravity/channel.rs`): o `agy -p` não traz pedidos de permissão no stream e nega sozinho o que pediria. O terra liga o `--dangerously-skip-permissions` e põe um gancho `PreToolUse` (`terra agy-permission-hook`) no plugin temporário `terraforme-permission`; o gancho deixa passar as ferramentas que só leem (e as edições no Accept Edits) e pergunta o resto. Quem decide depende do chat:
- **Chat delegado** (Manual, Accept Edits e Agente): `--from <pid> --chat <n>`; o gancho pergunta a quem delegou pela ferramenta `permission` do MCP (`mcp::ask_permission`), e quem delegou responde com `decide`.
- **Chat normal, Manual e Accept Edits**: `--channel <arquivo>`; o `Channel` (`Run::gate`) abre um servidor TCP em `127.0.0.1` (porta qualquer) e grava a porta e um token aleatório de 32 bytes no arquivo `channel` do plugin (pasta 0700, arquivo 0600). O gancho lê o arquivo, manda `{token, tool, input}` e espera. Cada pedido vira um `AgentEvent::PermissionRequest { id, name, detail }` (o `detail` é o `CommandLine`, o caminho...), entregue pelo `Session::poll`, e o `Session::respond` do `Run` devolve a decisão ao gancho. A UI é a mesma do Claude (`Chat::apply` e `Chat::answer`, sem mudança).
- **Yolo**: o `agy` aprova tudo, sem gancho. **Plan** e **Agente** (chat normal): o `agy` decide sozinho pelo modo.

Negado, token errado, sem resposta em 10 minutos, canal fechado (chat interrompido) ou qualquer erro: a ferramenta não roda. Um pedido que o `agy` desistiu de esperar (o `timeout` de 11 minutos do plugin) continua na fila do chat até ser respondido; a resposta é ignorada.

RTK e ai-memory:
- **RTK**: `rtk hook` só tem `claude`, `cursor`, `gemini` (BeforeTool do Gemini CLI), `copilot`, `droid` e `vibe`; nenhum fala o protocolo do `agy`. Sem suporte.
- **ai-memory**: tem `--agent antigravity-cli` e `--client antigravity-cli`, que escrevem no `~/.gemini/config/hooks.json` e no `mcp_config.json`. Como o `agy` já lê esses arquivos, o caminho é o usuário rodar `ai-memory install-hooks --agent antigravity-cli --apply`; o terra repetir isso por processo duplicaria a captura. A extensão ainda monta a Contribution do `claude-code` (e roda o `install-hooks` à toa para o `agy`), mas só o servidor MCP chega ao `agy`, e sem repetir o do usuário.
- `Extension::contribution()` continua sem receber o provedor. Receber o id só vale quando houver um segundo provedor que aceite hooks do terra.

**Config do Antigravity** (`antigravity/config.rs`): `mcpServers` do `~/.gemini/config/mcp_config.json` (aceita comentários e vírgula sobrando, como o `agy`; `serverUrl` vale como `url`) e as skills de `~/.gemini/antigravity-cli/builtin/skills`, `~/.gemini/config/skills` e `.agents/skills` do `cwd` para cima. Não lê ainda os plugins do `agy` (`~/.gemini/config/plugins/*` e `.agents/plugins/*`, com MCP e skills próprios) nem o `skills.json`/`plugins.json`.

## Orquestrador

> **Removido na v0.8.29.** O MCP do terra e os chats delegados não existem mais; esta seção descreve o que existia até a v0.8.28 e fica como histórico. Hoje a aba Orquestrador só guarda as configurações (`OrchestratorSettings`, `src/tui/orchestrator.rs`).

O orquestrador é o **MCP do terra**: um servidor MCP (JSON-RPC 2.0 sobre HTTP) que roda uma vez por máquina. Ele lista os terra abertos e deixa um agente delegar tarefas a chats novos. Quem delega recebe a resposta final e também aprova ou nega os pedidos de permissão do chat delegado.

### Servidor

| Item | Valor | Ref. |
|---|---|---|
| Endpoint | `http://127.0.0.1:30303/mcp` (`PORT`, `PATH`), só na máquina local | `src/mcp.rs:40`, `src/mcp.rs:42`, `src/mcp.rs:49` |
| Processo | `terra mcp`, em processo próprio e desligado do terminal (`ensure_running` + `detach`). Se a versão for outra, é fechado e aberto de novo | `src/mcp.rs:297`, `src/mcp.rs:327`, `src/cli.rs:77`, `src/mcp/server.rs:71` |
| Ligar/desligar | `McpSettings.enabled` (`config.json` global, chave `mcp`). Ao desligar, um `terra/shutdown` fecha o MCP para todos os terra | `src/settings.rs:334`, `src/tui/app.rs:223`, `src/tui/app.rs:250` |
| Protocolo | `initialize`, `ping`, `tools/list`, `tools/call`; versão `2025-06-18`; respostas em `application/json`, sem SSE | `src/mcp/server.rs:58`, `src/mcp/server.rs:442` |
| Concorrência | As chamadas curtas são atendidas uma por vez. `wait` e `permission` ganham uma thread cada e esperam por um `Condvar` | `src/mcp/server.rs:344`, `src/mcp/server.rs:364` |
| Origem do pedido | Vem na query: `?from=<pid>` (agente de um terra) e `&chat=<id>` (chat delegado) | `src/mcp.rs:216`, `src/mcp.rs:223`, `src/mcp/server.rs:320` |

**Ferramentas** (`tools/list`, `src/mcp/server.rs:455`):

| Ferramenta | Quem usa | O que faz | Ref. |
|---|---|---|---|
| `spaces` | qualquer agente | Lista os terra vivos: pid, nome, pasta, se há sessão e os chats de cada um (tokens, cache, busy, id, reply) | `src/mcp/server.rs:460`, `src/mcp/server.rs:623` |
| `chat` | quem delega | Recebe `message` (obrigatório), `title`, `pid` ou `workspace`; sem destino, usa o terra de quem chamou. Guarda um `ChatRequest` e devolve o `chat_id` | `src/mcp/server.rs:187`, `src/mcp/server.rs:632` |
| `wait` | quem delega | Espera até `timeout_seconds` (padrão 120 s, máximo 600 s). Devolve `permission` (pedidos pendentes), `done` (com `reply`), `running` ou `closed` | `src/mcp/server.rs:546`, `src/mcp/server.rs:238` |
| `decide` | quem delega | Recebe `request_id`, `allow` e `message`. Só vale para pedidos de chats que o próprio chamador delegou (`parent == caller`) | `src/mcp/server.rs:262` |
| `permission` | chat delegado (interno) | É o `--permission-prompt-tool` do Claude. Bloqueia até a decisão, ou nega depois de 10 min | `src/mcp/server.rs:576`, `src/mcp/server.rs:61`, `src/mcp/server.rs:683` |

**Registro dos terra** (métodos `terra/*`, `src/mcp/server.rs:489-519`):
- Cada terra abre um `Link` (`src/mcp.rs:416`). A cada 2 s (`HEARTBEAT`), uma thread manda `terra/announce` com o `Instance`: pid, nome, pasta, se há sessão e os `chats` com o `ChatStats` de cada um (`src/mcp.rs:478`).
- A resposta do `announce` traz os `ChatRequest` pendentes daquele terra. Eles são marcados como `delivered` (`src/mcp/server.rs:491`).
- Um terra que fica 6 s sem avisar sai da lista (`EXPIRE`, `src/mcp/server.rs:52`). `terra/leave` é chamado no `Drop` do `Link` (`src/mcp.rs:469`).

**Token** (v0.8.23):
- `~/.terraforme/mcp.token`: 32 bytes aleatórios em hex, arquivo com modo 0600 (`src/mcp.rs:65-111`). O MCP lê o arquivo a cada pedido.
- O cliente interno (`post`) manda `Authorization: Bearer`. O agente também: `apply_extensions` (`src/tui/app.rs`) junta a `mcp::agent_server(url, token)` à Contribution, com o cabeçalho `Bearer ${TERRA_MCP_TOKEN}` e o token em `Contribution.env`, fora da linha de comando. Num chat delegado, `delegated()` troca só a URL.
- `handle_connection` (`src/mcp/server.rs`) chama `refusal` antes de olhar o corpo: sem o token (`401`), `Host` ou `Origin` que não sejam `127.0.0.1`/`localhost`/`[::1]` (`403`) ou `Content-Type` que não seja `application/json` (`415`). O corpo da recusa é um erro JSON-RPC.
- Um provedor novo só precisa levar o cabeçalho: o `${TERRA_MCP_TOKEN}` vem do `env` da Contribution (no `agy`, o plugin do overlay expande o valor, porque o `agy` não resolve `${VAR}`).

### Como um chat delegado é aberto

1. O agente chama `chat`. O MCP cria a `Delegation { parent, target, delivered }` e enfileira o `ChatRequest` para o terra alvo (`src/mcp/server.rs:208-217`).
2. Na próxima batida, o terra alvo recebe o pedido. `tick_mcp` chama `open_chat` para cada um (`src/tui/app.rs:910`).
3. `open_chat` (`src/tui/app.rs:290`) faz o seguinte:

| Aspecto | Origem | Ref. |
|---|---|---|
| Provedor | `McpSettings.provider` (`mcp.provider` no `config.json` global): `None` é o do chat do workspace; senão, o `id` de um adapter, escolhido na linha "Provedor" da aba (`←`/`→`). `delegated_settings()` troca `AgentSettings.provider` e zera conta, modelo, esforço e sub agentes (ids do outro provedor); `Chat::connect` usa `adapters::select` | `src/settings.rs:359`, `src/tui/app.rs` (`open_chat`, `delegated_settings`), `src/tui/mcp.rs` (`Setting::Provider`) |
| Modelo / esforço | `McpSettings.model` / `effort`, escolhidos no menu do `/model` da aba Orquestrador, com o catálogo do provedor dos delegados: o do chat do workspace ou, com outro provedor, o que o terra carrega para ele (`tui::mcp::Delegated`, `load_catalog` do adapter). `valid_choice` descarta o modelo/esforço que o catálogo não tem. Sem modelo, fica o padrão | `src/tui/app.rs` (`open_chat`, `sync_delegated`), `src/tui/mcp.rs` (`Delegated`, `valid_choice`) |
| Modo | `McpSettings.spawn_mode` (padrão Accept Edits) | `src/tui/app.rs:295`, `src/tui/chat.rs:290`, `src/settings.rs:355` |
| Extensões | As do chat do terra, com a URL do servidor `terra` trocada por `delegated_url(pid, id)` (`delegated()`) | `src/tui/app.rs:293`, `src/tui/app.rs:1199` |
| Prompt | `ChatRequest::prompt()` acrescenta o cabeçalho "[Tarefa delegada pelo agente do workspace …]" | `src/mcp.rs:274` |
| Aba | Título = `title`, ou "De {from}", ou "Chat N". Fica em segundo plano, a não ser com `focus_delegated` | `src/tui/app.rs:298-306` |
| Fim | Com `close_finished` (padrão ligado), a aba fecha quando o chat termina. O `ChatStats` com o `reply` vai para `finished` e continua sendo enviado ao MCP | `src/tui/app.rs:312`, `src/tui/app.rs:888` |

A resposta volta assim: `chat_stats` só manda `id` e `reply` nos chats delegados (`src/tui/app.rs:1176`). O `wait` devolve `done` quando `!busy && reply.is_some()` (`src/mcp/server.rs:247`).

### Permissões do chat delegado → quem delegou

1. `Chat::start` preenche `Request.permission_tool = Some(("terra", "permission"))` quando o chat tem `delegation` e o servidor `terra` está nas extensões (`src/tui/chat.rs:903-911`, `src/agent/adapter.rs:56`).
2. No Claude (`Run::start`, `src/agent/adapters/claude.rs:523`):
   - Se o modo não é Yolo e há `permission_tool`, o `--permission-prompt-tool stdio` sai (`src/agent/adapters/claude.rs:529`, `src/agent/adapters/claude.rs:332`) e entra `--permission-prompt-tool mcp__terra__permission` (`src/agent/adapters/claude.rs:550-554`).
   - No Yolo, o terra aprova tudo e não pergunta a ninguém.
3. O servidor `terra` chega ao `claude` por `--mcp-config` como `{"type":"http","url":…,"headers":…}`; o `claude` expande os `${VAR}` dos cabeçalhos (`src/agent/adapters/claude.rs:372-406`). O `env` da `Contribution` vai para o processo (`src/agent/adapters/claude.rs:541`).
4. O `claude` chama `permission` com `{tool_name, input, tool_use_id}`. O MCP usa o `chat` da URL para achar a delegação, guarda um `Permission` e bloqueia.
5. Quem delegou recebe o pedido no `wait` (`status: permission`) e responde com `decide`.
6. A resposta segue o formato do Claude: o texto `{"behavior":"allow","updatedInput":…}` ou `{"behavior":"deny","message":…}` (`src/mcp/server.rs:576-616`).

**No Antigravity** (`src/agent/adapters/antigravity/permission.rs`): o `agy` não tem `--permission-prompt-tool`, e no `-p` ele nega sozinho o que pede permissão ("a tool required the command permission that headless mode cannot prompt for"). O que ele tem é o gancho `PreToolUse` dos plugins (medido no `agy` 1.4.8):
- O `agy` roda o comando do `hooks.json` antes de cada ferramenta, com `{toolCall:{name,args},conversationId,…}` no stdin, e lê `{"decision":"allow"|"deny","reason":…}` no stdout. O `hooks.json` do plugin usa o mesmo formato do global: `{"<plugin>":{"PreToolUse":[{"matcher":"","hooks":[{"type":"command","command":…,"timeout":<s>}]}]}}`.
- `allow` do gancho **não** vence a negativa do modo headless: só `--dangerously-skip-permissions` faz a ferramenta rodar. Com ele, o gancho passa a ser o único porteiro: `deny` bloqueia (o agente vê "tool call denied by pre-tool hook: <motivo>") e `allow` executa. O `permissionOverrides` não liberou nada no `-p`.
- O `agy` mata o gancho em 30 s (e a ferramenta não roda); o `timeout` do `hooks.json` (segundos) muda isso.
- `Gate::create` monta o plugin `terraforme-permission` numa pasta temporária (`--add-dir`, 0700, apagada ao soltar) quando o chat é delegado (`Request.permission_tool`) e o modo não é Yolo nem Plan; o `Run` liga o `--dangerously-skip-permissions`. O comando do gancho é `terra agy-permission-hook --from <pid> --chat <id> [--accept-edits]` (subcomando oculto do `cli.rs`).
- `permission::hook` lê a chamada e deixa passar as ferramentas que só leem (`READ_ONLY`) e, no Accept Edits, as de edição (`EDITS`). O resto (`run_command`, navegador, sub agentes, MCP...) vai para `mcp::ask_permission`, que chama a ferramenta `permission` (`tool_name` = nome do `agy`, `input` = `args`) e espera até 11 min. Negado, sem resposta ou com o MCP fora do ar: `deny` com o motivo.
- Quem delegou vê o pedido no `wait` com o nome e os argumentos do `agy` (ex.: `run_command` com `CommandLine`) e responde com `decide`.

### Para um provedor novo

**O que implementar no adapter:**
- **Aceitar um servidor MCP HTTP** vindo de `Contribution.mcp_servers` (`url` + `headers`, com `${VAR}` apontando para `Contribution.env`), traduzido para a configuração MCP do provedor. É o equivalente a `mcp_args` (`src/agent/adapters/claude.rs:372`). Sem isso o agente não enxerga `spaces`/`chat`/`wait`/`decide`.
- **Passar `Contribution.env` ao processo do agente**, para o token (`TERRA_MCP_TOKEN`) não aparecer na linha de comando.
- **Um equivalente do `--permission-prompt-tool`:**
  - Ler `Request.permission_tool` (servidor, ferramenta) e fazer o provedor consultar essa ferramenta MCP antes de usar outra ferramenta.
  - Se o provedor não tiver esse gancho, o adapter mesmo pode interceptar o pedido de permissão e chamar `permission` por HTTP (`/mcp?from=<pid>&chat=<id>`): `mcp::ask_permission(pid, chat, tool, input)` faz isso, espera a decisão e devolve `Ok(())` ou o motivo da negativa. O Antigravity o chama de um gancho `PreToolUse` (`permission.rs`); um provedor com gancho parecido pode seguir o mesmo caminho (subcomando oculto do `terra` como comando do gancho).
  - Nesse caso, traduzir o formato de entrada `{tool_name, input}` e de saída `behavior/allow|deny`, `updatedInput`, `message`.
- **Respeitar o modo no chat delegado:** no Yolo, aprovar sem consultar. Nos outros modos, encaminhar a quem delegou, e não ao usuário (no Claude, isso é desligar o `stdio`).
- **Expor `name()`, `catalog()` e `default_model()`**, para a aba Orquestrador mostrar o provedor e o menu de modelo/esforço (`src/tui/app.rs:1190`). O `McpSettings.model` é um id do catálogo do provedor.
- **Sinais de fim de chat:** `is_busy()`, `last_reply()` e tokens/cache corretos no `Chat`. São eles que alimentam o `wait` (`done`).

**O que hoje é específico do Claude ou fixo:**

| Ponto | Situação atual | Ref. |
|---|---|---|
| Formato da ferramenta `permission` | Entrada e saída no formato do `--permission-prompt-tool` do Claude (texto JSON com `behavior`). O gancho do `agy` traduz para `decision`/`reason` e usa os nomes de ferramenta do `agy` | `src/mcp/server.rs:573-616`, `src/mcp/server.rs:683`, `src/agent/adapters/antigravity/permission.rs` |
| Nome da ferramenta para o provedor | `mcp__{server}__{tool}`, convenção do Claude | `src/agent/adapters/claude.rs:553` |
| Configuração MCP | `--mcp-config` JSON `mcpServers` / `type: http`, com `${VAR}` expandido pelo `claude` | `src/agent/adapters/claude.rs:372-406` |
| Provedor dos delegados | Resolvido: `McpSettings.provider` (ver acima). O chat do workspace e os delegados podem usar provedores diferentes | `src/settings.rs:359`, `src/tui/mcp.rs` |
| Modelo / esforço salvos | Ids do catálogo do provedor dos delegados. Trocar o provedor os limpa, e `valid_choice` descarta, ao abrir o chat, o que o catálogo não tem. Sem catálogo (ainda carregando), o id salvo vai como está | `src/settings.rs:347-351`, `src/tui/mcp.rs` |
| Modo dos delegados no `agy` | O `spawn_mode` vale como no Claude, mas o `agy` só tem `auto`, `accept-edits` e `plan`; Manual não passa `--mode`. Com o gancho, Manual/Accept Edits/Agente perguntam a quem delegou (o Accept Edits deixa as edições passarem); Yolo aprova tudo; Plan nega o que mudaria algo | `src/agent/adapters/antigravity.rs` (`mode_args`), `permission.rs` |

## Configurações, contas e login

### Onde fica a configuração

| Arquivo | Escopo | Conteúdo | Código |
|---|---|---|---|
| `~/.terraforme/workspace/<workspace>/config.json` | um por workspace | `agent` (`AgentSettings`), `extensions`, `workspace.ask_to_save` | `src/settings.rs:450`, `src/cli.rs:114` |
| `~/.terraforme/config.json` | um por máquina | `mcp` (`McpSettings`) | `src/settings.rs:443` |

- O app carrega os dois arquivos ao abrir (`src/tui/app.rs:203-216`). Se o arquivo do workspace não existir, vale `AgentSettings::default()`, em que tudo fica com o provedor (`src/settings.rs:230-247`).
- Cada mudança na aba Config chega em `handle_config_key` (`src/tui/app.rs:416`). Ela é gravada com `save_file`, que mantém as outras chaves (`src/settings.rs:274`, `src/settings.rs:430`), e aplicada a todos os chats com `Chat::apply_settings` (`src/tui/app.rs:564-572`, `src/tui/chat.rs:233`).
- Os valores dinâmicos de cada campo vêm de `App::choices()` (`src/tui/app.rs:463`): contas, modelos, catálogo, servidores MCP e skills. Tudo isso é pedido ao adapter (`ChatAdapter`, `src/agent/adapter.rs:210`).

### Campos que dependem do provedor

| Campo (rótulo na Config) | Chave no JSON | Valor salvo | De onde vêm as opções | Como chega ao provedor (Claude) |
|---|---|---|---|---|
| Conta | `agent.account` | id da conta (nome da pasta, ex.: `.claude-trabalho`); `null` é a Default | `adapter.accounts()` (`src/tui/app.rs:465`). A Config percorre as opções (`src/tui/config.rs:695-707`); sem contas, só a Default existe | `ClaudeAdapter::account_dir` resolve a pasta (`src/agent/adapters/claude.rs:62`), que vira `CLAUDE_CONFIG_DIR` no `start` (`claude.rs:558`), no `interactive` (`claude.rs:202`) e no `login` (`claude.rs:146`). Trocar de conta recarrega o catálogo e começa uma conversa nova (`src/tui/chat.rs:234-249`) |
| Modo inicial | `agent.start_mode` | `provider` ou a chave de um `PermissionMode` | `StartMode::ALL`, com "Do provedor" mais os 5 modos (`src/settings.rs:50`) | Ver o modo "Do provedor" abaixo. Um modo fixo vira `--permission-mode` (`cli_mode`, `claude.rs:265`) |
| Modelo | `agent.model` | id do modelo; `null` é o "Padrão" | `Chat::model_options()`: os modelos do catálogo ou, enquanto ele não chega, `adapter.models()` (`src/tui/chat.rs:758`). No Claude, os modelos fixos são `opus`, `sonnet` e `haiku` (`claude.rs:239`) | `--model` (`claude.rs:335`, `claude.rs:180`). Também é gravado pelo `/model` → "default" (`src/tui/app.rs:575-582`) |
| Esforço | `agent.effort` | nível; `null` é o do provedor | Não aparece na Config. É escolhido no menu do `/model` entre os `ModelInfo::efforts` do catálogo (`src/agent/adapter.rs:103`) | `--effort` (`claude.rs:338`, `claude.rs:183`). O chat usa o esforço da sessão ou, sem ele, o padrão (`src/tui/chat.rs:503`) |
| Sub agentes | `agent.subagent_model` | id do modelo; `null` deixa o agente decidir | Percorre os modelos ou abre, com Enter, o menu do `/model` sobre o catálogo (`src/tui/config.rs:274-279`, `src/tui/config.rs:208-216`) | Variável `CLAUDE_CODE_SUBAGENT_MODEL` (`claude.rs:42`, `claude.rs:186`, `claude.rs:542`) |
| Interface | `agent.interface` | `terraforme` ou `provider` | `Interface::ALL` (`src/settings.rs:91`) | Em `provider`, o terra abre `adapter.interactive(...)` com conta, modelo, esforço, sub agentes, modo e extensões (`src/tui/app.rs:613-636`, `claude.rs:168`) |
| MCP › Conectados | (não é salvo) | — | `adapter.mcp_servers(cwd, conta)` somado aos servidores das extensões e do MCP do terra (`src/tui/app.rs:528-561`). No Claude, vêm do `.claude.json` da conta (do usuário e o `projects[cwd]`) e do `.mcp.json` do cwd (`claude.rs:102`, `claude/mcp.rs:14-17`) | Só exibição: o próprio `claude` carrega esses servidores. Os das extensões vão por `mcp_args` (`claude.rs:372`) |
| Skills › Instaladas | (não é salvo) | — | `adapter.skills(cwd, conta)` mais as skills que só o catálogo informa (`src/tui/app.rs:496-523`). No Claude, vêm de `<pasta da conta>/skills`, `skills/synced/*`, plugins, `~/.agents/skills` e `.claude/skills` do cwd e das pastas acima (`claude.rs:117`, `claude/skills.rs:25-44`) | Só exibição |
| MCP › Modo dos chats, modelo e esforço dos delegados | `mcp.spawn_mode`, `mcp.model`, `mcp.effort` (no config global) | `StartMode` e ids do catálogo | `StartMode::ALL` (`src/tui/config.rs:243-252`); modelo e esforço vêm do `/model` da aba Orquestrador (`src/tui/mcp.rs:313-320`) | São aplicados ao chat delegado (`src/tui/app.rs:295-296`) |

Contexto, Consumo e Extensões não dependem do provedor. A única exceção é o limite "Do modelo", que usa `Chat::context_window()` (`src/tui/app.rs:472`).

### Modo inicial "Do provedor"

- `Chat::start_mode` (`src/tui/chat.rs:337-347`) chama `adapter.configured_mode(cwd, conta)`. Se o provedor responder Yolo ou nada, o chat começa em Agente.
- No Claude (`claude.rs:87`, `claude.rs:463-502`), o modo é o `permissions.defaultMode` do primeiro arquivo que o definir, nesta ordem:
  1. `<cwd>/.claude/settings.local.json`
  2. `<cwd>/.claude/settings.json`
  3. `<pasta da conta>/settings.json`
- A pasta da conta é `CLAUDE_CONFIG_DIR` ou `~/.claude` (`default_config_dir`, `claude.rs:481`).
- `parse_mode` faz o mapa: `auto`→Agente, `manual`/`default`→Manual, `acceptEdits`, `plan`. `bypassPermissions` não tem equivalente (`claude.rs:277`).
- Como cada conta tem seu próprio `settings.json`, trocar de conta recalcula o modo (`src/tui/chat.rs:250-255`).

### Contas do Claude (`src/agent/adapters/claude/accounts.rs`)

| Item | Regra | Linha |
|---|---|---|
| Conta Default | `id: None`: o terra não define `CLAUDE_CONFIG_DIR`. O rótulo usa o e-mail do `.claude.json` padrão, que é `$CLAUDE_CONFIG_DIR/.claude.json` ou `~/.claude.json` | `accounts.rs:57-63`, `claude.rs:474` |
| Demais contas | Pastas da home com nome `.claude` ou `.claude-<nome>` que tenham `.credentials.json` | `accounts.rs:19-20`, `accounts.rs:65-73`, `accounts.rs:111` |
| E-mail e organização | Vêm de `oauthAccount.emailAddress` e `oauthAccount.organizationName` do `<pasta>/.claude.json` | `accounts.rs:122-132` |
| Tipo | Sem organização, ou com organização terminando em `'s Organization`, a conta é "Pessoal"; senão é "Trabalho" (aparece com o nome da organização). Sem `.claude.json`, o rótulo é o nome da pasta | `accounts.rs:44-50`, `accounts.rs:74-86` |
| Ordem | Default primeiro, depois as pessoais e por fim as de trabalho | `accounts.rs:89-96` |
| Segurança | `config_dir` só aceita nomes de conta sem `/` nem `..` e que sejam pastas existentes. Uma conta salva que sumiu dá o erro "Conta do Claude não encontrada" | `accounts.rs:106-109`, `claude.rs:62-71` |

O rótulo exibido (`AccountOption.label`) aparece na Config e no título do chat (`src/tui/chat.rs:269`, `src/tui/app.rs:1108`).

### Como o `/login` funciona

1. O `/login` chama `Chat::start_login` (`src/tui/chat.rs:497`, `src/tui/chat.rs:597`), que pede `adapter.login(conta)`. Se o adapter devolver `None`, o chat mostra "Este provedor não tem login pelo terra."
2. No Claude, o comando é `claude auth login [--email <e-mail da conta>]` com `CLAUDE_CONFIG_DIR=<pasta>` quando a conta não é a Default (`claude.rs:134-150`).
3. O loop principal pega o comando com `take_login` e o roda fora da tela do terra (`src/tui.rs:178-180`, `src/tui.rs:103-112`). Ele passa por `login::run` (`src/tui/login.rs:32`), que:
   - mostra o link que vem depois de `visit: ` como hiperlink OSC 8 e o copia para a área de transferência (`login.rs:20`, `login.rs:163-175`, `login.rs:205`);
   - esconde o prompt do Claude, `Paste code here if prompted >` (`login.rs:23`), e lê o código com eco, sem espaços (`login.rs:84-111`);
   - pede o código de novo quando a saída traz `Invalid code` (`login.rs:21`, `login.rs:115-120`);
   - usa os tempos limite de 15 s para o link e 60 s para a resposta (`login.rs:25-27`).
4. No fim, `login_finished` avisa no chat, recarrega o catálogo e invalida o consumo (`src/tui/app.rs:605-609`, `src/tui/chat.rs:635-648`).
5. Quando uma resposta do agente falha com um texto que contém `authenticate`, `oauth` ou `log in again`, o chat sugere o `/login` (`src/tui/chat.rs:651-662`, chamado em `src/tui/chat.rs:1075` e `src/tui/chat.rs:1086`).

### Detecção do Claude e instaladores

- O terra procura o executável no `PATH` (`claude`; no Windows, `claude.exe` ou `claude.cmd`) em `find_claude` (`claude.rs:288-297`). A busca roda no `ClaudeAdapter::detect`, que também guarda a home (`claude.rs:54`).
- Sem o executável, `is_available()` é falso, o chat fica Desconectado e mostra o link de instalação (`claude.rs:39`, `src/tui/chat.rs:211-215`).
- Os adapters ficam registrados em `adapters::all()`, e `default()` escolhe o primeiro disponível (`src/agent/adapters.rs:11-24`).
- **Os instaladores (`install-linux.sh`, `install-win.sh`, `install.sh`, `install.ps1`) não detectam nem instalam o Claude.** O texto "claude" não aparece neles. Eles cuidam de:
  - o binário do terra;
  - a função `terra` nos shells (no Windows, também o PATH e os perfis do PowerShell);
  - as extensões RTK e ai-memory (`install-linux.sh:148-177`, `install-win.sh:164-179`);
  - a remoção do terraforme antigo.
- O único contato indireto é o `ai-memory setup --yes` (`install-linux.sh:175`, `install-win.sh:177`). Ele é da própria ferramenta ai-memory e "liga os agentes" por conta dela; o terra não controla o que é configurado.
- `src/files.rs` não tem relação com o provedor. É o índice de arquivos do workspace usado nas menções `@`.

### Para um provedor novo

**O que implementar no `ChatAdapter`** (`src/agent/adapter.rs:210-303`). Depois, registre o adapter em `adapters::all()` (`src/agent/adapters.rs:12`).

| Método | Obrigatório? | Efeito na Config, nas contas e no login |
|---|---|---|
| `name`, `is_available`, `start` | sim | Nome no título; estado Desconectado; envio das mensagens com `Request` (modelo, conta, esforço e sub agentes) |
| `unavailable_hint` | recomendado | Dica de instalação |
| `accounts` | opcional | Sem ele, só existe a Default. O `id` é opaco para o terra: é salvo em `agent.account` e volta em todos os métodos que recebem `account` |
| `configured_mode` | opcional | Sem ele, "Do provedor" começa em Agente |
| `models`, `model_label`, `load_catalog` | opcional | Modelos da Config e do `/model`. Os esforços vêm só de `ModelInfo::efforts`, no catálogo |
| `mcp_servers`, `skills` | opcional | Seções MCP e Skills (só exibição) |
| `login` | opcional | Sem ele, o `/login` mostra o `login_help` do adapter (ou "não tem login pelo terra") |
| `login_error_words` | opcional | Trechos de erro que fazem o chat sugerir o `/login`; vazio, nunca sugere |
| `interactive` | opcional | Sem ele, a Interface "Original do provedor" mostra um erro (`src/tui/app.rs:638-644`) |
| `load_usage`, `load_daily` | opcional | Consumo (fora desta seção) |

**O que hoje é específico do Claude e precisa ser revisto:**

- **Contas:** pastas `~/.claude*` com `.credentials.json`, a variável `CLAUDE_CONFIG_DIR`, o `oauthAccount` do `.claude.json` e a regra Pessoal/Trabalho pelo sufixo `'s Organization` (`accounts.rs`).
- **Login:** os textos fixos `URL_MARKER = "visit: "`, `INVALID_MARKER = "Invalid code"` e `PROMPT` (`src/tui/login.rs:20-23`). Eles moram no módulo genérico da TUI, então outro CLI com mensagens diferentes não funciona sem parametrizar o `login::run` (por exemplo, com marcadores vindos do adapter).
- **Dica de login:** as palavras vêm do adapter (`login_error_words`: Claude `authenticate`, `oauth`, `log in again`; Antigravity `not logged in`).
- **Modo:** o `permissions.defaultMode` e a ordem dos arquivos do Claude (`claude.rs:489`), mais o mapa de nomes `cli_mode` e `parse_mode`.
- **Sub agentes:** a variável `CLAUDE_CODE_SUBAGENT_MODEL` (`claude.rs:42`).
- **Configuração:** resolvido. `agent.provider` (e `mcp.provider`) guarda o provedor; `adapters::select` escolhe o adapter por ele, e a Config limpa conta, modelo, esforço e sub agentes ao trocar de provedor (`change` em `src/tui/config.rs`).
- **Textos de exemplo e documentação** que citam o Claude: `src/settings.rs:13` (`".claude-trabalho"`) e a doc de `StartMode::Provider` (`src/settings.rs:43`).
- **Instaladores:** se o provedor novo precisar de um CLI instalado, os instaladores não têm hoje nenhum passo para provedores. Seria preciso criar um, seguindo o modelo de `install_rtk`/`install_ai_memory`, ou deixar só a dica do `unavailable_hint`.

### Antigravity (`agy` 1.4.8): contas, login e instalação

Conferido no `agy` real:

- **Contas: não há.** O token fica no chaveiro do sistema (`keyringAuth`), não numa pasta. Com `HOME=/outra/pasta`, o `agy` cria um `.gemini` novo lá, mas entra na mesma conta do chaveiro (`authenticated via keyring`). `ANTIGRAVITY_GEMINI_DIR` não muda nada. Por isso o adapter não implementa `accounts()`: só existe a Default.
- **Login: sem comando.** `agy login` dá "unexpected argument"; os subcomandos são `agent(s)`, `changelog`, `help`, `install`, `mcp`, `mic-serve`, `models`, `plugin(s)`, `remote-control` e `update`. O `/login` só existe na interface dele. Por isso `login()` é `None`, e `login_help()` explica o caminho. Os marcadores de `src/tui/login.rs` (`URL_MARKER`, `INVALID_MARKER`, `PROMPT`) **não foram parametrizados**: sem comando de login para rodar, nenhum provedor os usaria, e o Claude é o único que passa por ali.
- **Instaladores: só a dica.** `install-linux.sh`, `install-win.sh`, `install.sh` e `install.ps1` não instalam o Claude; instalar o `agy` (Google, com login interativo no navegador) seria o mesmo caso, e o `agy` é opcional. Fica o `unavailable_hint` do adapter, com o link de instalação.
