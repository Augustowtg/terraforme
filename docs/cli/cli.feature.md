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

## Configurações 
Configurações do agente (conta, modo inicial, modelo e esforço padrão), de cada workspace.
1. Salvas em `~/.terraforme/workspace/[nome_da_workspace]/config.json`
2. O nome é o do workspace salvo para o diretório ou, sem um, o nome da pasta
3. Sem o arquivo, o workspace começa com as configurações globais
4. A tela mostra onde salva (Salvo em)

## Sair
Botão em vermelho, e quando apertar Esc ele redireciona para o botão de sair quando clicar enter ele mostra o box abaixo
1. box de confirmação (Sim) (Não) para confirmar saida ou não 