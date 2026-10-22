# Feature: Webserver

Parte de observabilidade do terraforme: uma página web local que mostra as instâncias abertas no
computador, as mensagens entre elas e o consumo por space. Servida pela
[engine](../engine/engine.feature.md) (`crates/terraforme-mcp/src/ui.rs` e `ui.html`).

## Comportamento

- Pode ficar **online** ou **offline** (`[mcp] ui`, padrão offline): ligue com
  `terraforme config --mcp-ui true` ou no campo `UI` da aba [Configurações](../config/config.feature.md),
  que mostra o endereço e abre o navegador com clique ou `o`.
- O `config.toml` é lido a cada pedido: ligar/desligar vale sem reiniciar a engine. Offline, as
  páginas respondem 404 com a dica de como ligar.
- Endereço: `http://127.0.0.1:<porta da engine>/ui` (também em `GET /`).

### Página principal: "Terraforme instâncias"

```text
Terraforme instâncias                 terraforme-mcp v1.2.3 · 3 instância(s) · atualiza a cada 2s

Instâncias abertas no computador
┌─────────────┬──────────────────────────────────────────┐
│ Space       │ Agentes                                  │
├─────────────┼──────────────────────────────────────────┤
│ terraforme  │ ● Agente 1 (Claude)  ● Agente 2 (Opencode)│
│ ~/kaizen/…  │                                          │
└─────────────┴──────────────────────────────────────────┘

Mensagens
10:42:01 · api/claude-1 → api/claude-2
  qual o formato do endpoint X?
[destino ▾] [mensagem…] [Enviar]

Consumo por space
Space | Agente | Sessões | 24h | 7 dias | Total | Respostas
```

- **Instâncias**: uma linha por space (ou pasta, fora de space, como `(fora de space)`), com os
  agentes numerados na ordem em que abriram (`Agente 1 (Claude)`). O ponto colorido mostra a
  atividade (parada, trabalhando, terminou, pede atenção); o tooltip traz apelido, pasta e hora
  de abertura. Clicar escolhe a instância como destino da mensagem. Atualiza a cada 2s.
- **Mensagens**: o histórico do MCP (até 500), incluindo erros de entrega. O formulário manda
  uma mensagem do usuário para uma instância; ela chega como prompt, com o pedido de responder
  com `send_message(to="web")`.
- **Consumo por space**: totais do histórico SQLite (`terraforme_core::history`), igual à
  sub-aba Spaces do [Consumo](../consumo/consumo.feature.md). Atualiza a cada 10s.

## Como funciona

| Rota | Uso |
| --- | --- |
| `GET /ui`, `GET /` | a página (`ui.html`, embutida com `include_str!`) |
| `GET /api/state` | versão, instâncias e mensagens do `Hub` |
| `GET /api/usage` | `History::totals` por space × agente |
| `POST /api/send` | `{to, text}` → `Hub::send` com remetente `web` |

- Todas exigem `Host` local (`127.0.0.1`/`localhost`, contra DNS rebinding) e, se houver,
  `Origin` local (`ui::local`); senão 403.
- `GET /pet/state` e `POST /pet/focus` servem a [janela flutuante](../../janela-flutuante.md) com
  a mesma checagem de origem, mas independem de `[mcp] ui`.

## Limitações

- Só leitura das instâncias: não abre nem fecha instâncias pela página.
- Sem autenticação além da checagem de origem local.
