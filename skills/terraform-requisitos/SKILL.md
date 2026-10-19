---
name: terraform-requisitos
description: Levanta e documenta os requisitos de uma aplicação existente a partir do código-fonte, com foco em DDD (contextos delimitados, agregados, entidades, objetos de valor, eventos e regras de negócio), traduzindo nomes e lógica do código para linguagem de negócio (linguagem ubíqua). Registra cada achado numa memória JSON em ~/.terraforme/{projeto}/requisitos.json e gera dela o documento .md e o PDF, com glossário, requisitos funcionais, regras de negócio, critérios de aceite, controle de versões e rastreabilidade por tipo ("Pedido é Order"), sem citar arquivos de código. Use quando o usuário pedir para "levantar os requisitos", "extrair requisitos do código", "documentar as regras de negócio", "mapear o domínio", "criar a linguagem ubíqua", "engenharia reversa de requisitos" ou "documento de requisitos DDD". Não use para especificar uma feature nova que ainda não existe no código nem para documentação técnica de API.
metadata:
  author: augusto
  version: 1.3.0
---

# Requisitos com DDD a partir do código

Lê o código de uma aplicação, identifica o modelo de domínio e escreve um documento de requisitos em Markdown que uma pessoa de negócio consiga ler sem conhecer o código.

## Importante

- **Nunca invente requisitos.** Tudo no documento precisa ter origem no código. O que for dedução sem evidência direta recebe a marca "(inferido)" e também entra em "Lacunas e dúvidas".
- **Linguagem de negócio, não de código.** Escreva "O cliente pode cancelar um pedido", nunca "O método `cancel()` da classe `Order`".
- **Nunca cite arquivos, pastas, linhas, métodos ou constantes do código.** O código só aparece como correspondência de tipo, no formato "Pedido é `Order`", em dois lugares: na coluna "Termo no código" do glossário e na seção "Rastreabilidade". O campo "Referências" só pode apontar para outras documentações (políticas, manuais, especificações).
- **Versione o documento.** Todo documento tem "Controle de versões". IDs (RF, RN, RNF) nunca são renumerados nem reaproveitados.
- **Glossário primeiro.** Monte a linguagem ubíqua antes de escrever os requisitos e use sempre o mesmo termo para o mesmo conceito.
- **Sem detalhes técnicos nos requisitos funcionais.** Frameworks, tabelas, endpoints e bibliotecas vão em "Requisitos não funcionais" ou "Integrações externas", e só quando expressam uma necessidade do negócio.
- **A memória JSON é a fonte da verdade.** Tudo o que for identificado entra em `~/.terraforme/<projeto>/requisitos.json` pelo `scripts/memoria.py`, uma parte por vez (seção + id + campo). O `.md` e o PDF são sempre gerados do JSON. Nunca edite o JSON nem o `.md` à mão: uma edição manual no `.md` some na próxima geração.
- Escreva no idioma do usuário (português por padrão), traduzindo os nomes em inglês do código.

## Memória do levantamento

Todos os comandos usam o script abaixo. A referência completa dos campos e comandos está em `references/esquema-json.md`, e um exemplo preenchido em `exemplo/requisitos.json`.

O bloco a seguir funciona em qualquer agente (Claude Code, OpenCode…): ele descobre onde a skill está instalada e aponta `$M` para o script. Rode-o uma vez antes dos demais comandos e reutilize o mesmo shell.

```bash
# Localiza a pasta da skill instalada (global ou no projeto), sem depender do agente.
SKILL="$(for d in \
  "$HOME/.claude/skills/terraform-requisitos" \
  "$HOME/.config/opencode/skills/terraform-requisitos" \
  "$HOME/.agents/skills/terraform-requisitos" \
  ".claude/skills/terraform-requisitos" \
  ".opencode/skills/terraform-requisitos" \
  ".agents/skills/terraform-requisitos"; do
  [ -f "$d/SKILL.md" ] && printf '%s' "$d" && break
done)"
M="python3 \"$SKILL/scripts/memoria.py\""
$M iniciar <projeto> --titulo "<Nome da aplicação>" --autor "<autor>"   # cria a pasta e o JSON na versão 1.0.0
$M inserir <projeto> <secao> --json '{...}'                             # achado novo; imprime o id criado
$M editar  <projeto> <secao> <id> --campo <campo> --valor <valor> [--anexar]
$M ler     <projeto> [<secao> [<id>]] [--campo <campo>]
$M buscar  <projeto> "<texto>" [--secao <secao>]
$M gerar   <projeto> [--md <caminho.md>] [--pdf]
```

Use como `<projeto>` o nome do repositório analisado (ex.: `loja-online`). Antes de inserir, use `buscar` ou `ler` para ver se o item já existe: termos, contextos e ids repetidos são recusados. Leia e altere só a parte que precisa, sem carregar o JSON inteiro quando ele estiver grande.

## Etapa 1: Delimitar o escopo

Confirme com o usuário, se não estiver claro:
- **Abrangência:** o repositório inteiro ou um módulo/contexto específico.
- **Destino:** por padrão, `docs/requisitos.md` no repositório analisado (passado em `gerar --md`). Em aplicações grandes, um projeto de memória por contexto (`loja-vendas`, `loja-pagamentos`), cada um gerando seu arquivo em `docs/requisitos/`.
- **Memória existente:** rode `$M caminho <projeto>` e `$M ler <projeto> documento`. Se a memória já existir, continue nela em vez de começar do zero. Se não existir, rode `$M iniciar`. Se houver só um documento `.md` antigo, sem memória, leia-o e registre o conteúdo no JSON mantendo os mesmos IDs (passe `"id"` no `inserir`, em ordem crescente).
- **Outras documentações:** pergunte se há políticas, manuais ou especificações que possam ser citados em "Referências".

## Etapa 2: Reconhecimento

Mapeie a estrutura antes de ler arquivo por arquivo: linguagem, framework e onde o domínio vive (pastas como `domain`, `models`, `entities`, `services`, `usecases`, `handlers`, `modules`). Em repositórios grandes, delegue a varredura a um subagente de busca e peça só o mapa das pastas e dos arquivos de domínio.

## Etapa 3: Identificar os blocos do domínio

Procure, nesta ordem:
1. **Contextos delimitados:** módulos, pacotes, serviços ou schemas com vocabulário próprio.
2. **Agregados, entidades e objetos de valor:** classes com identidade, classes imutáveis comparadas por valor e qual delas controla as outras.
3. **Ações (casos de uso):** controllers, handlers de comando, métodos públicos de serviços de aplicação, jobs agendados.
4. **Regras de negócio:** validações, cláusulas de guarda, exceções de domínio, máquinas de estado (enums de status), constraints de banco e checagens de permissão.
5. **Eventos de domínio:** eventos publicados, listeners, mensagens em filas.
6. **Integrações e requisitos não funcionais:** clientes HTTP, filas, autenticação, auditoria, retentativas, cache, limites de taxa.

Registre cada achado na memória assim que identificá-lo, já em linguagem de negócio (Etapa 4): contextos em `contextos`, termos em `glossario` (com `tipo_codigo`), blocos do domínio em `modelo_dominio`, ações em `rf`, regras em `rn`, necessidades técnicas em `rnf`, sistemas externos em `integracoes` e dúvidas em `lacunas`. Quando um achado posterior completa um anterior, edite só o campo afetado, por exemplo `--campo regras --valor '["RN-003"]'` ou `--campo criterios --valor "Dado ..." --anexar`.

Caminhos e linhas de código servem só para você conferir o trabalho e não entram na memória; a rastreabilidade registra apenas os termos do glossário, e o tipo vem de lá ("Pedido é `Order`").

## Etapa 4: Traduzir para linguagem de negócio

Consulte `references/traducao-codigo-negocio.md` para converter cada padrão de código em frase de negócio (por exemplo, exceção de guarda em regra de negócio, enum de status em ciclo de vida, evento em "quando X acontece, Y"). Insira primeiro os termos do glossário e só depois os requisitos e regras, usando os termos dele em `rastreabilidade`.

## Etapa 5: Gerar o documento

Com os achados registrados, gere o `.md` a partir da memória:

```bash
$M gerar <projeto> --md <repositorio>/docs/requisitos.md
```

O documento segue a estrutura de `references/modelo-documento.md`, com todas as seções; as que ficaram vazias saem como "Não identificado". Para ver o nível de detalhe e o tom esperados, leia `exemplo/exemplo-documento.md`, que é gerado de `exemplo/requisitos.json` (o PDF dele está em `exemplo/exemplo-documento.pdf`).

O script numera RF-001, RN-001 e os demais ids sozinho e nunca reaproveita um id. Todo RF precisa de critérios de aceite no formato Dado/Quando/Então e deve citar as RN relacionadas em `regras`. Todo RF e RN deve ter `rastreabilidade` com os termos do glossário que o sustentam. Uma memória nova começa na versão 1.0.0.

## Etapa 6: Validar

O `gerar` recusa a memória enquanto houver ligação quebrada (RN citada que não existe, termo da rastreabilidade fora do glossário ou sem tipo no código). Use `$M verificar <projeto>` para ver esses erros antes. Depois de escrever o `.md`, o `gerar` roda o validador e mostra o resultado. Para validar um `.md` avulso:

```bash
python3 "$SKILL/scripts/validar_documento.py" <caminho-do-documento.md>
```

O validador confere as seções obrigatórias, o controle de versões (formato, ordem e igualdade com a "Versão do documento"), os IDs duplicados ou citados sem definição, os critérios de aceite e a rastreabilidade (formato "Termo é `Tipo`", termo presente no glossário e mesmo tipo). Acusa como erro qualquer referência a arquivo ou pasta de código e qualquer método ou caminho no lugar de um tipo, e avisa sobre termos de código que vazaram para o texto de negócio. Corrija todo `ERRO` e cada `AVISO` editando o campo correspondente na memória (`buscar` encontra o item pelo texto) e gere de novo. Se um `AVISO` deve ficar, justifique ao usuário por quê.

## Etapa 6b: Atualizar o documento

Para qualquer mudança pedida depois da primeira entrega, o fluxo é o mesmo: localize a parte com `buscar` ou `ler`, altere só ela com `editar`, `inserir` ou `remover`, registre a versão e gere de novo o `.md` e o PDF.

```bash
$M editar  <projeto> rn RN-002 --campo regra --valor "Um pedido pode ter no máximo 80 itens."
$M versao  <projeto> --tipo maior --mudancas "RN-002 passa a permitir 80 itens"
$M gerar   <projeto> --md docs/requisitos.md --pdf
```

Escolha o tipo da versão pelo efeito da mudança: `maior` para item removido ou com significado alterado, `menor` para item novo, `correcao` para ajuste de redação. Registre a versão antes de um `remover`: RF, RN e RNF removidos continuam no documento com "(removido na versão X.Y.Z)", marcados com a última versão.

## Etapa 7: Revisar com o usuário

Entregue o caminho do arquivo e um resumo curto: quantos contextos, RF e RN, e a lista de itens em "Lacunas e dúvidas". Peça ao usuário para confirmar ou corrigir as inferências; regras de negócio costumam ter motivos que o código não mostra.

## Etapa 8: Exportar para PDF (quando pedido)

Se o usuário pedir o documento em PDF, gere junto com o `.md`, sempre a partir da memória:

```bash
$M gerar <projeto> --md docs/requisitos.md --pdf
```

O PDF sai ao lado do `.md`. Por baixo, o `gerar` chama o script abaixo, que também pode converter um `.md` avulso:

```bash
uv run "$SKILL/scripts/gerar_pdf.py" <caminho-do-documento.md> [-o saida.pdf] [--sem-sumario]
```

O `uv run` instala as dependências (reportlab e markdown-it-py) num ambiente isolado. Sem `uv`, use `pip install reportlab markdown-it-py` e rode com `python3`. O PDF tem sumário clicável, cabeçalho com a versão do documento e numeração "Página X de Y". Para mudar o PDF, edite a memória e gere de novo.

## Exemplos

### Exemplo 1: repositório inteiro
Usuário diz: "Levante os requisitos desta aplicação com DDD."
Ações:
1. Confirmar escopo (repositório inteiro) e destino (`docs/requisitos.md`); `$M iniciar loja-online --titulo "Loja Online"`.
2. Mapear contextos, agregados, ações, regras e eventos, inserindo cada achado na memória assim que identificado.
3. `$M gerar loja-online --md docs/requisitos.md` e corrigir na memória o que o validador apontar.
Resultado: memória e documento na versão 1.0.0 com glossário, RF, RN e rastreabilidade por tipo, mais a lista de dúvidas para o usuário.

### Exemplo 2: um contexto específico
Usuário diz: "Documente as regras de negócio do módulo de pagamentos."
Ações:
1. Limitar a análise ao módulo e às integrações que ele usa.
2. Registrar as relações com outros contextos sem detalhá-los, num projeto de memória próprio (`loja-pagamentos`).
Resultado: `docs/requisitos/pagamentos.md` gerado da memória e validado.

### Exemplo 3: mudança num documento já entregue
Usuário diz: "O limite agora é 80 itens por pedido; atualize o documento e o PDF."
Ações:
1. `$M buscar loja-online "50 itens"` para achar a RN-002 e os critérios que citam o limite.
2. `editar` só esses campos, `versao --tipo maior` com a mudança, `gerar --md docs/requisitos.md --pdf`.
Resultado: versão 2.0.0 com a regra nova e o PDF atualizado.

## Solução de problemas

### O `gerar` recusa a memória
Causa: uma ligação está quebrada, como RN citada que não existe ou termo da rastreabilidade fora do glossário ou sem `tipo_codigo`.
Solução: rode `$M verificar <projeto>`, insira o termo ou a regra que falta (ou corrija o nome), e gere de novo.

### O `inserir` recusa o item
Causa: campo desconhecido, campo obrigatório vazio, termo ou contexto repetido, ou `tipo_codigo` que não é nome de tipo.
Solução: confira os campos em `references/esquema-json.md`. Se o item já existe, use `editar` em vez de `inserir`.

### O código não segue DDD (MVC, scripts, CRUD)
Causa: o domínio está espalhado por controllers, models e consultas SQL.
Solução: trate cada grupo de tabelas e telas relacionadas como um contexto candidato, procure regras em validações e consultas, e registre em "Lacunas e dúvidas" que os contextos foram inferidos.

### O validador aponta muitos termos de código no texto
Causa: frases descrevendo a implementação em vez do comportamento.
Solução: substitua cada identificador pelo termo do glossário e reescreva a frase do ponto de vista de quem usa o sistema. Se o termo não existir no glossário, adicione-o.

### O validador acusa referência a arquivo de código
Causa: um caminho ou linha de código entrou no documento (em "Referências", "Visão geral" ou "Rastreabilidade").
Solução: remova o caminho. Se a informação for importante, registre o tipo correspondente na rastreabilidade ou cite a documentação que descreve a regra.

### Nomes do código são ambíguos ou genéricos (Manager, Helper, data, process)
Causa: o código não usa a linguagem do negócio.
Solução: descreva o comportamento observado, proponha um termo de negócio marcado "(inferido)" e leve a dúvida ao usuário.
