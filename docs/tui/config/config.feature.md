# Feature: aba Configurações

Formulário que edita o `config.toml` (e as preferências do space) de dentro do
`terraforme cli`, gravando a cada alteração. Implementação: `Form` em
`crates/terraforme-cli/src/term.rs`.

## Comportamento

- Abre com `Ctrl+Shift+F4` ou clique em `Configurações`; é sempre a última aba.
- O título mostra o caminho do `config.toml`; embaixo do formulário aparecem o resultado da
  última alteração (`✔`/`✘`) e onde cada campo grava.
- Teclas: `↑↓`/`j k` escolhem o campo, `←→`/`Espaço`/`Enter` alternam o valor, dígitos (ou
  `Enter`) editam números (`Enter` grava, `Esc` cancela), `Del`/`d` volta ao padrão, `o` abre a UI
  web, `r` relê o arquivo, `Esc`/`q` voltam ao agente. Clique escolhe o campo.

### Layout

```text
── Agente ──────────────────────────────────────
   O agente escolhido no Router abre uma instância dele ao sair do campo ou da aba
 › Router                      Claude Code
   Modo                        custom (RTK + AI Jail)  marque as extensões abaixo com Espaço
     [x] AI Jail                                       sandbox do sistema
     [x] RTK                                           economia de tokens
     [ ] AI Memory                                     memória entre sessões
   Conta Claude Code           default
   Middleware (Experimental)   não  chat do terraforme no lugar do terminal (Claude, Antigravity, OpenCode)

── Orçamento ───────────────────────────────────
   Limites de tokens (aba Consumo, quando o plano não informa os dele)
   A cada 5 horas              20.000.000
   Tokens diário               96.000.000 (calculado)
   Tokens em 7 dias            672.000.000 (calculado)

── Plugins · config global ─────────────────────
   UI                          online · http://127.0.0.1:<porta>/ui
   Mascote                     ativo
   MCP                         online · porta <porta>

── Avançado ────────────────────────────────────
   Capturar o mouse            sim
   Copiar ao selecionar        sim
   Carência ao sair (s)        5
   Timeout do subagente (s)    300
   Subagentes simultâneos      3
```

### Campos

| Seção | Campo | Valores | Grava em |
| --- | --- | --- | --- |
| Agente | `Router` | agentes implementados | `agent` do space ou `provider` do `config.toml` |
| Agente | `Modo` | `eco`, `high`, `security`, `custom` (`CliMode`) | `mode` + `jail` do space ou do `config.toml` |
| Agente | `[ ] AI Jail` / `RTK` / `AI Memory` | caixas, só com `custom` | idem |
| Agente | `Conta <agente>` | `default` + contas do `claude.toml` | `account` do space (só Claude Code, só dentro de space) |
| Agente | `Middleware (Experimental)` | `sim` / `não` | `[tui] middleware` (sempre no `config.toml`) |
| Orçamento | `A cada 5 horas` | número (com máscara de milhar ao digitar) | `[budget] five_hours` e `week` |
| Orçamento | `Tokens diário`, `Tokens em 7 dias` | calculados: 5h × 24/5 e × 168/5 (`budget_for`) | não editáveis |
| Plugins | `UI` | `online · <url>` / `offline` | `[mcp] ui` |
| Plugins | `Mascote` | `ativo` / `inativo` (`só no Linux` fora dele) | `[tui] pet`; abre/fecha a janela na hora |
| Plugins | `MCP` | `online · porta N` / `offline` | nada: alternar sobe o daemon se estiver fora do ar |
| Avançado | `Capturar o mouse`, `Copiar ao selecionar` | `sim` / `não` | `[tui] mouse`, `copy_on_select` (valem na hora) |
| Avançado | `Carência ao sair (s)`, `Timeout do subagente (s)`, `Subagentes simultâneos` | número | `[mcp] grace_seconds`, `subagent_timeout_seconds`, `max_simultaneous_subagents` |

- Dentro de um space salvo, Router, Modo e Conta gravam nas preferências dele
  (`<root>/<space>/preferences.toml`); `Del` remove o override e volta a valer o `config.toml`.
- O preset `security` e a caixa `AI Jail` só aparecem onde há AI Jail (não no Windows).
- Escolher `custom` não grava nada: só mostra as caixas, que partem da combinação atual.
- Clicar na linha `UI` ou `o` abre a UI web no navegador (`platform::open_url`).

### Troca de Router

- A troca grava na hora, mas só tem efeito ao sair do campo ou da aba (`Form::router`,
  `Form::leave_router`, checado em `Form::key`, `App::click_form` e `App::select`).
- Então a TUI resolve a sessão de novo (`Options::relaunch`) e abre uma instância do novo agente;
  as instâncias abertas continuam rodando com o agente delas.
- Mudanças de modo e conta marcam `App::stale` e valem nas próximas instâncias (`⌃⌥N`, ou `⌃⌥R`
  para reiniciar a aberta).

## Como funciona

- `Form::fields()` monta a lista na ordem da tela; `Field::section()` a divide nas seções
  (`Section`: Agente / Orçamento / Plugins · config global / Avançado).
- `Form::effective()` aplica as preferências do space sobre o `config.toml`, como
  `Choice::resolve` faz no lançamento.
- Cada alteração chama `Form::save` (`config.toml`) ou `Form::save_prefs` (`preferences.toml`;
  preferências vazias apagam o arquivo).

## Limitações

- O campo `MCP` não desliga o daemon: ele fecha sozinho quando o último terminal sai.
- Os valores do `[mcp]` em Avançado valem para o próximo daemon iniciado (são passados como
  argumentos por `mcp::start`).
