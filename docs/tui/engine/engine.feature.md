# Feature: Engine

Processo em segundo plano que orquestra os terminais do `terraforme cli`. Hoje a engine é o
daemon `terraforme-mcp` (`crates/terraforme-mcp`): um só por usuário, compartilhado por todos os
terminais, que roda o [MCP](../../MCP/mcp.feature.md) e o [Webserver](../webserver/webserver.feature.md).

## Comportamento

- Ao abrir, o `terraforme cli` verifica se já há uma engine no ar; se houver, usa ela, senão
  sobe uma em segundo plano (`mcp::ensure_running`).
- A engine não depende do terminal que a abriu: fechar essa janela não a derruba.
- Ela encerra sozinha quando o último terminal sai (depois da carência `[mcp] grace_seconds`,
  padrão 5s) ou 30s após subir sem nenhum terminal registrado.
- `terraforme restart` mata todas as engines (inclusive órfãs de versões antigas) e sobe a nova;
  os terminais abertos se registram de novo sozinhos.
- Na aba [Configurações](../config/config.feature.md), o campo `MCP` mostra `online · porta N`
  ou `offline` e sobe a engine se ela estiver fora do ar.

## Como funciona

- **Lock file**: `<data dir>/terraforme/mcp.lock` (`$XDG_DATA_HOME` ou `~/.local/share` no Linux,
  `%LOCALAPPDATA%` no Windows) com `pid` e `port`. Uma engine só grava o lock se nenhuma outra
  saudável já o tem (`LockFile::claim`, senão encerra) e só apaga o seu (`LockFile::release`).
  Saudável = PID vivo e porta aceitando conexão (`mcp::is_healthy`).
- **Início**: `mcp::start` roda o `terraforme-mcp` ao lado do binário atual (ou do `PATH`) com
  `--grace-seconds`, `--subagent-timeout` e `--max-subagents`, desligado do terminal
  (`platform::detach`), e espera até 2s pelo lock. Escuta em `127.0.0.1` numa porta livre.
- **Registro**: cada terminal manda `POST /register` (id, PID e o endereço do seu servidor de
  controle) e um `POST /heartbeat` a cada 2s. Se o heartbeat falha ou volta 404 (engine nova que
  não o conhece), o terminal chama `ensure_running`, se registra de novo e reenvia as instâncias
  (`Msg::Reconnected`).
- **Watchdog**: a cada 5s a engine remove terminais com PID morto e as instâncias deles
  (`ClientRegistry::spawn_watchdog`, `Hub::drop_client`).
- **Encerramento**: `Ctrl+C`/`SIGTERM` ou o canal de shutdown param o servidor axum com
  graceful shutdown.

## Limitações

- Agentes que já estavam rodando quando a engine foi trocada continuam com a porta antiga no
  `--mcp-config` até serem reiniciados (`⌃⌥R`).
