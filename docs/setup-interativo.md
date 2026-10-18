# Configuração interativa de agentes e extensões

Esta página descreve o fluxo implementado por `terraforme extension --setup`.

## Objetivo

Permitir que a pessoa escolha os agentes instalados que quer configurar e, em seguida, quais extensões deseja integrar a eles. O terraforme deve executar os comandos e preparar os arquivos necessários, sem exigir que a pessoa descubra ou rode manualmente etapas específicas de cada agente.

## Fluxo interativo

1. Ao executar `terraforme extension --setup` sem informar uma extensão, procurar no `PATH` os agentes implementados: Claude Code (`claude`) e OpenCode (`opencode`). Mostrar somente os encontrados e permitir selecionar um ou mais, informando os números separados por vírgula (por exemplo, `1,2`).
2. Perguntar quais extensões configurar: RTK, AI Memory e/ou AI Jail, também por números separados por vírgula. RTK e AI Memory são configuradas para os agentes selecionados; AI Jail é uma configuração da máquina e pode ser escolhida mesmo sem agentes detectados. Enter seleciona nenhum item; `q` cancela sem fazer alterações.
3. Não mudar o agente padrão definido em `terraforme config` nem configurar integrações RTK/AI Memory para agentes que não foram selecionados.
4. Verificar se cada extensão escolhida está instalada. Se faltar alguma, informar qual e orientar a instalação com `terraforme extension <extensão> --install`; não instalar binários sem uma escolha explícita.
5. Executar automaticamente a configuração necessária e informar o resultado por extensão e agente. Uma falha deve identificar a etapa que falhou, sem ocultar configurações que já tenham sido concluídas.

O comando existente `terraforme extension <extensão> --setup` deve continuar disponível para refazer a configuração de uma extensão individual.

## Requisitos por integração

- **Claude Code + RTK:** executar a configuração do RTK para o Claude (`rtk init -g`).
- **Claude Code + AI Memory:** executar `ai-memory setup`, que prepara a integração com o Claude.
- **OpenCode + AI Memory:** executar `ai-memory install-hooks --agent opencode2 --apply` para preparar o plugin compatível com OpenCode 2.
- **OpenCode + RTK:** preparar o plugin compatível gerenciado pelo terraforme. Não usar como substituto o plugin legado de `rtk init --opencode`: o código atual documenta que ele usa o formato do OpenCode 1, incompatível com OpenCode 2.
- **AI Jail:** executar o setup existente uma vez, independentemente das extensões de agente: verificar a versão e, no Linux, `bwrap`; criar `~/.ai-jail` se não existir ou completar as seções de agentes ausentes. O comportamento atual completa as seções de todos os agentes implementados.

As etapas de RTK e AI Memory devem ser limitadas aos agentes escolhidos; o AI Jail segue o comportamento global descrito acima. Preservar configurações e dados existentes; comunicar claramente qualquer backup ou alteração inevitável. As regras de integração existentes nos adapters em `crates/terraforme-core/src/agent/` são a referência para os detalhes específicos de cada agente.

## Critérios de aceite

- `terraforme extension --setup` inicia o fluxo interativo sem exigir nome de extensão.
- A lista de agentes é detectada no computador e contém apenas Claude Code e/ou OpenCode quando seus executáveis estão disponíveis.
- A seleção de agentes acontece antes da seleção de extensões; ambas permitem escolher mais de uma opção. Sem agentes detectados, ainda é possível configurar apenas o AI Jail.
- Somente as integrações RTK/AI Memory selecionadas são configuradas para os agentes escolhidos. O AI Jail, se selecionado, roda uma vez como setup da máquina.
- Cancelar antes da execução não altera configurações; sucesso e falhas parciais são reportados claramente.
- As etapas específicas de AI Memory são executadas automaticamente para cada agente selecionado, com sucesso ou erro informado por etapa.
- A forma explícita `terraforme extension <extensão> --setup` e o setup separado do AI Jail continuam funcionando.
