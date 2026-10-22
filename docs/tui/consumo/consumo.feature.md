# Feature: aba Consumo

Consumo de tokens e limites de uso dos agentes abertos no `terraforme cli`, mais a economia do
RTK quando o modo o usa. Implementação em `crates/terraforme-cli/src/term.rs`.

## Comportamento

- Abre com `Ctrl+Shift+F2` ou clique em `Consumo` no menu (ver [menu](../tui.feature.md)).
- Sub-abas próprias (`USAGE_VIEWS`): **Limites**, **Tokens**, **Subagentes**, **Spaces**; com o
  RTK no modo, os relatórios dele vêm depois, agrupados como `RTK` (ex.: `Economia`).
- As sub-abas próprias recarregam sozinhas a cada 5s (`USAGE_REFRESH`) e mostram a instância
  em foco; os relatórios do RTK só recarregam com `r`.
- Teclas: `↑↓`/`j k`/`PgUp PgDn` rolam, `r` recarrega, `⌃⌥C` copia, `←→` trocam de aba,
  `Esc`/`q` voltam ao agente.

### Limites

Um bloco `Router <agente>` por agente com instância aberta, o da instância em foco primeiro
(`App::routers`, `format_routers`); instâncias do mesmo agente dividem os limites.

```text
Router claude

Sessão · 5h - zera em 2h30min - 21/10/2026 - 18:00
████████████████░░░░░░░░  65% restante

Semana · todos os modelos - zera em 3d 4h - 25/10/2026 - 09:00
████████████████████░░░░  83% restante

Fonte: limites do plano (api.anthropic.com, como o /usage) · atualiza a cada 60s

Router opencode
…
```

- Uma barra por limite: começa cheia em 100% e encurta com o consumo; verde acima de 50%,
  amarela até 20%, vermelha abaixo (`bar`).
- Título: nome do limite, `zera em <tempo> - <DD/MM/AAAA - HH:MM>` (hora local, `local_date`)
  quando o limite tem horário de reset, e `· <usado> de <máximo> tokens` quando há total de
  tokens (só no orçamento; a API do Claude informa só a porcentagem).
- Sem limites, aparece `Nenhum limite disponível.` com o comando para definir um orçamento.
- Fontes dos limites (`terraforme_core::limits`):

| Agente | Limites | Origem |
| --- | --- | --- |
| Claude Code | `Sessão · 5h`, `Semana · todos os modelos`, `Semana · Opus`, `Semana · Sonnet` (os que a API devolver) | `api.anthropic.com/api/oauth/usage`, cache de 60s |
| Antigravity | um bucket semanal por grupo de modelos | `agy -p "/usage" --output-format json` |
| OpenCode | só `Últimos 7 dias` (orçamento) | `[budget]` × `opencode stats --json` |
| Qualquer um, sem limites do plano | `Últimas 5h`, `Últimos 7 dias` | `[budget]` do `config.toml` × transcripts |

### Tokens

Tabela com Entrada, Saída, Raciocínio, Cache (grava), Cache (lê), Total e Respostas
(`format_usage`):

- Claude Code e Antigravity: `Sessão atual`, `Projeto · 24h`, `Todos · 5h`, `Todos · 24h`,
  `Todos · 7 dias` e a sessão atual por modelo.
- OpenCode: `Projeto · hoje`, `Todos · hoje`, `Todos · 7 dias`, o projeto por modelo e o custo
  (`format_opencode_usage`).

### Subagentes

A sessão atual dividida entre o agente principal e cada subagente, com o tipo e a descrição
(`format_agents`, só Claude Code). O OpenCode mostra só contagens (subagentes, sessões,
prompts dos últimos 7 dias); o Antigravity, um aviso de que não detalha.

### Spaces

Histórico do consumo por space × agente (`format_spaces`, `terraforme_core::history`, SQLite em
`<root>/usage.db`): Sessões, 24h, 7 dias, Total, Respostas. Não depende da instância em foco. A
mesma tabela está na [UI web](../webserver/webserver.feature.md).

### Consumo sempre à vista

Fora da aba, em qualquer tela do `terraforme cli` (só Claude Code por enquanto):

```text
 Terraforme  Agente (claude)  Consumo  …                           ⌃⌥K  Atalhos
 ⌃⌥1 claude 1  ⌃⌥2 claude 2  …                      Sessão terraforme  48.210 tokens
…
 Prompt atual  5.350 tokens · entrada 120 · saída 5.230                       claude 1  31.904 tokens
```

Esses contadores somam só entrada e saída (o raciocínio conta como saída), sem os tokens de
cache; o cache aparece apenas na sub-aba Tokens.

| Onde | O quê |
| --- | --- |
| Rodapé, à esquerda | Prompt atual: do último prompt digitado até agora (o em andamento ou o último), com os subagentes |
| Rodapé, à direita | Total da sessão da instância aberta |
| Canto superior direito | Total desta sessão do terraforme: todas as instâncias desde que ele abriu, inclusive as fechadas; nas outras abas, no título do quadro |

- Atualiza a cada 5s (`LIVE_EVERY`); vazio até a primeira leitura ou em agentes sem `live_usage`.
- Sem espaço na linha das sub-abas, a dica de atalhos sai antes do total; no rodapé, o prompt
  atual sai antes do total da instância.
- Uma sessão retomada conta inteira na instância, mas só o gasto desde a abertura conta no total
  do terraforme.

## Como funciona

- Rodapé e total: `App::poll_live` lê numa thread `Adapter::live_usage` de cada instância
  (Claude: `usage::live`, que lê só a transcript da sessão e as dos subagentes) e manda
  `Msg::Live`; `Live` traz a sessão, o gasto desde a abertura (`launched`, somado em
  `App::spent` por id de instância) e o prompt atual (`turn`: respostas desde a última linha
  `user` digitada, sem resultados de ferramenta, `isMeta` nem subagentes). Desenho em
  `App::draw_footer` e `App::spent_line`.
- A aba é uma entrada `TabKind::Content` do registro `TABS` com `refresh: true`; o conteúdo é
  carregado numa thread e chega por `Msg::Loaded`.
- `Adapter::usage` lê o consumo de cada agente; `limits::collect` converte o `Adapter::quota()`
  em `Limit`s ou cai no orçamento `[budget]`.
- O TUI grava o `usage.session` de cada instância no histórico a cada 60s (`RECORD_EVERY`), ao
  fechar/reiniciar a instância e ao sair (`App::record_usage`).

## Limitações

- O cache de 60s dos limites do plano (`limits::quotas`, `static PLAN`) é um só para todos os
  agentes: com dois routers que informam limites (ex.: Claude e Antigravity), o segundo pode
  mostrar os limites do primeiro até o cache expirar.
- OpenCode não tem janela de 5h, consumo da sessão atual nem tokens por subagente.
- O rodapé e o total do terraforme só existem no Claude Code; uma instância que fecha logo depois
  de uma leitura perde no total o que gastou nos últimos segundos (até 5s).
- Os números são tokens, não custo em dinheiro: o terraforme não tem tabela de preços por modelo.
