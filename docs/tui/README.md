# TUI do `terraforme cli`

Especificações das features da TUI (`crates/terraforme-cli`) e do daemon que ela usa. Visão
geral e uso no dia a dia: [guia](../guia.md) e [funcionalidades](../funcionalidades.md).

| Feature | O que cobre | Estado |
| --- | --- | --- |
| [Menu de abas](tui.feature.md) | menu do topo, atalhos de aba e sub-abas | implementado |
| [Agente / Middleware](agente/agente.feature.md) | chat do terraforme no lugar do terminal do agente | experimental (só Claude Code) |
| [Consumo](consumo/consumo.feature.md) | limites, tokens, subagentes e histórico por space | implementado |
| [Configurações](config/config.feature.md) | formulário do `config.toml` e das preferências do space | implementado |
| [Engine](engine/engine.feature.md) | daemon `terraforme-mcp` em segundo plano | implementado |
| [Webserver](webserver/webserver.feature.md) | UI web de observabilidade | implementado |
| [MCP](../MCP/mcp.feature.md) | ferramentas para as instâncias conversarem e delegarem | implementado (só Claude Code) |

Janela flutuante (mascote): [janela-flutuante](../janela-flutuante.md).
