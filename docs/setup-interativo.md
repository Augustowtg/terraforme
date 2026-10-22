# Setup interativo

Como funciona o `terraforme extension --setup`, para quem quer escolher à mão quais agentes e
extensões configurar. O setup automático e o que cada configuração altera estão em
[Setup](setup.md).

## Quando usar

- Configurar só alguns agentes ou só algumas extensões.
- Integrar um agente instalado depois do setup inicial.
- Refazer a integração por agente quando ela parou de funcionar (ex.: depois de atualizar o
  agente).

Diferente do `terraforme setup`, nada é instalado: as extensões escolhidas precisam estar
instaladas (`terraforme extension <ext> --install`). Exige terminal interativo.

## Fluxo

```bash
terraforme extension --setup
```

1. **Agentes** — mostra numerados os agentes implementados que estão no `PATH` (Claude Code,
   OpenCode, Antigravity). Sem nenhum, avisa e segue (ainda dá para configurar o AI Jail).
2. **Seleção de agentes** — números separados por vírgula ou espaço (ex.: `1,2`).
3. **Extensões** — RTK, AI Memory e AI Jail, escolhidas do mesmo jeito. Sem agentes selecionados,
   só o AI Jail aparece: ele é configuração da máquina e não depende de agente.
4. **Execução** — configura cada extensão escolhida e mostra o resultado por extensão e agente
   (`✔`, `ℹ` avisos, `⚠` alertas, `✘` falhas).

| Resposta | Efeito |
| --- | --- |
| `1,3` ou `1 3` | seleciona esses itens (repetidos são ignorados) |
| `Enter` ou `nenhum`/`nenhuma` | nenhum item |
| `q` ou `cancelar` | cancela sem alterar nada |

Número fora da lista ou texto inválido encerra com erro, sem alterar nada. Sem nenhuma extensão
selecionada, nada é alterado.

## Comportamento

- RTK e AI Memory são configurados **só** nos agentes selecionados; os outros não são tocados.
- O AI Jail roda uma vez, como setup da máquina, e completa as seções de todos os agentes que ele
  conhece no `~/.ai-jail` (Claude Code e OpenCode).
- O agente padrão do `terraforme config` não muda.
- Extensão não instalada: a etapa falha com `<extensão> não está instalado — use
  terraforme extension <ext> --install`.
- Uma falha não interrompe as demais: cada etapa é tentada e, no fim, o comando informa quantas
  falharam.
- O que cada integração executa (`rtk init -g`, `ai-memory setup`, `ai-memory install-hooks
  --agent opencode2 --apply`, plugins do terraforme) está em
  [O que cada configuração faz](setup.md#o-que-cada-configuração-faz).
- A integração de RTK e AI Memory com o Antigravity ainda não existe: selecioná-lo faz essas
  etapas falharem.

## Relação com os outros comandos

| Comando | Diferença |
| --- | --- |
| `terraforme setup` | sem perguntas; instala agentes e extensões que faltam e copia as skills ([Setup](setup.md#terraforme-setup)) |
| `terraforme extension <ext> --setup` | só o comando global de uma extensão, sem passar pelos agentes ([Setup](setup.md#reconfigurar-uma-extensão)) |
