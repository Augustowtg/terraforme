# Documentação do terraforme

Índice da documentação: comece pelo guia de uso; os documentos de referência e as features detalham cada parte.

## Uso

- [Guia de uso](guia.md) — instalação, todos os comandos, o terminal com abas, atalhos e spaces.
- [Setup](setup.md) — `terraforme setup`, `terraforme extension <ext> --setup` e o que cada configuração altera.
- [Setup interativo](setup-interativo.md) — o fluxo de `terraforme extension --setup`, passo a passo.
- [Janela flutuante](janela-flutuante.md) — o mascote que avisa quando os agentes terminam (`terraforme pet`, Linux).

## Referência

- [Funcionalidades](funcionalidades.md) — catálogo de tudo o que o terraforme faz, com links para os docs dedicados.
- [Arquitetura](arquitetura.md) — crates, módulos e como modos, AI Jail, consumo e o daemon MCP funcionam por dentro.
- [Desenvolvimento](desenvolvimento.md) — compilar, testar, versões, commits, CI e releases.

## Features

- [MCP](MCP/mcp.feature.md) — instâncias que conversam, abrem e fecham outras e delegam tarefas.
- [TUI](tui/README.md) — índice das features do `terraforme cli`:
  - [Menu de abas](tui/tui.feature.md)
  - [Middleware (experimental)](tui/agente/agente.feature.md)
  - [Consumo](tui/consumo/consumo.feature.md)
  - [Configurações](tui/config/config.feature.md)
  - [Engine (daemon MCP)](tui/engine/engine.feature.md)
  - [UI web](tui/webserver/webserver.feature.md)
