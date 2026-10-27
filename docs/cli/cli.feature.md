# CLI 

Interface principal onde vai ter visibilidade dos agentes, consumo entre outros.

## Chat
Em construção 
### Chat historico
### Chat Input


## Consumo 
Mostra o consumo das LLMs usando os routes 
1. Route: o provedor e a conta do chat, com o plano (ex.: `Claude · Default · eu@gmail.com · Pro`)
2. Um progress bar de 0 a 100% por limite (Sessão de 5 h, Semana...), verde, amarelo a partir de 70% e vermelho a partir de 90%
3. Abaixo de cada barra: `Reseta DD-MM-YYYY HH:MM · zera em Xh Ym`
4. Crédito extra: se a conta tiver o crédito extra ativo, aparece a barra dele com o valor gasto no mês (`US$ 1,78 de US$ 40,00 usados no mês`); se não tiver, a barra não aparece
5. Gráfico Consumo por dia: ao lado das barras, com a altura da seção dos limites (embaixo em telas estreitas), os tokens de cada um dos últimos 14 dias, com o de hoje em destaque e o resumo (hoje, média por dia e total). Botões com o mouse: Sem cache (verde, padrão) e Com cache (vermelho). Clicar num dia troca o gráfico para as horas daquele dia, com o consumo do dia embaixo (total e pico); ‹ Dias ou Esc volta
6. Atualiza ao abrir a aba, a cada minuto com ela aberta e com `r`

## Extensões
Aplicações secundárias que melhoram o agente (ex.: [RTK](https://github.com/rtk-ai/rtk) e [ai-memory](https://github.com/akitaonrails/ai-memory)). Cada extensão é uma estratégia (padrão Strategy) registrada num Registry: diz se está instalada, o que acrescenta ao agente (hooks e servidores MCP) e o relatório desta aba.
1. Lista das extensões à esquerda (ativa, inativa ou não instalada); `↑`/`↓` escolhem
2. À direita, a versão, a descrição, o link e o relatório da escolhida: no RTK, os tokens economizados (total, economia média, comandos) e o gráfico dos tokens economizados por dia; no ai-memory, páginas, sessões, observações, tamanho do banco e o estado dos embeddings e do LLM
3. Atualiza ao abrir a aba, a cada minuto com ela aberta e com `r`
4. Ativar e desativar fica na Config (seção Extensões)

## Configurações 
Configurações do agente (conta, modo inicial, modelo e esforço padrão), de cada workspace.
1. Salvas em `~/.terraforme/workspace/[nome_da_workspace]/config.json`
2. O nome é o do workspace salvo para o diretório ou, sem um, o nome da pasta
3. O config.json é lido ao iniciar; sem ele, o workspace começa com o padrão do router (modo do provedor, modelo e esforço padrão, conta Default)
4. Interface: Agente Terraforme (chat e input do terra) ou Original do provedor (o `claude` interativo dentro da aba Chat, mantendo a barra de abas; aberto ao iniciar e com Enter, com as mesmas configurações; Ctrl+Q volta às abas; trocar de Interface não fecha o terminal se já foi enviada uma mensagem)
5. Seção Extensões, à direita do Agente (embaixo em telas estreitas): a lista das extensões no estilo da lista de modelos (`1. RTK - descrição`), paginada (5 por página, `←`/`→` trocam de página); `Espaço` (ou `Enter`) ativa ou desativa; `Shift+Tab` alterna entre o Agente e as Extensões. Salva em `extensions.enabled` no config.json do workspace

## Sair
Botão em vermelho, e quando apertar Esc ele redireciona para o botão de sair quando clicar enter ele mostra o box abaixo
1. box de confirmação (Sim) (Não) para confirmar saida ou não 
2. se o workspace não estiver salvo, pergunta se deseja salvar: (Sim) (Não) (Não, não perguntar novamente); o "não perguntar novamente" fica no config.json do workspace