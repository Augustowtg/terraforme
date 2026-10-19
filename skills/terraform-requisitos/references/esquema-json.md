# Esquema da memória JSON (`requisitos.json`)

A memória de cada projeto fica em `~/.terraforme/<projeto>/requisitos.json` e é a fonte da verdade do levantamento. O `.md` e o PDF são sempre gerados a partir dela pelo `scripts/memoria.py`. Nunca edite o JSON nem o `.md` à mão: use os comandos do script, que validam cada parte antes de gravar.

Exemplo completo: `exemplo/requisitos.json`. Ele gera exatamente o `exemplo/exemplo-documento.md` (um teste garante isso).

A raiz `~/.terraforme` pode ser trocada pela variável de ambiente `TERRAFORME_HOME`.

## Estrutura

```json
{
  "esquema": "terraforme-requisitos/1",
  "projeto": "loja-online",
  "atualizado_em": "2026-10-16T15:00:00",
  "documento": { ... },
  "versoes": [ ... ],
  "visao_geral": { ... },
  "contextos": [ ... ],
  "glossario": [ ... ],
  "modelo_dominio": [ ... ],
  "requisitos_funcionais": [ ... ],
  "regras_negocio": [ ... ],
  "requisitos_nao_funcionais": [ ... ],
  "integracoes": [ ... ],
  "lacunas": [ ... ],
  "contadores": { "RF": 5, "RN": 6, ... }
}
```

`esquema`, `projeto`, `atualizado_em` e `contadores` são mantidos pelo script. `contadores` guarda o maior número de id já usado por prefixo; é ele que impede reaproveitar um id depois de uma remoção.

## Seções de objeto único (editadas sem id)

| Seção | Campo | Obrigatório | Descrição |
|---|---|---|---|
| `documento` | `titulo` | sim | Nome da aplicação. Vira "Documento de Requisitos — <titulo>". |
| `documento` | `nota` | não | Texto opcional exibido como citação logo abaixo do título. |
| `visao_geral` | `proposito` | não | O que a aplicação faz para o negócio. |
| `visao_geral` | `escopo` | não | Aplicação inteira ou contextos analisados. |
| `visao_geral` | `versao_aplicacao` | não | Versão, release ou data do código analisado. |
| `visao_geral` | `documentos_relacionados` | não | Outras documentações usadas como fonte. Vazio vira "Nenhum". |

A "Versão do documento" da visão geral não é guardada: sai sempre da última linha de `versoes`, então as duas nunca divergem.

## Controle de versões (`versoes`)

Lista de `{ "versao", "data", "autor", "mudancas" }`, da mais antiga para a mais recente. Só muda pelo comando `versao`, que calcula o próximo número (`maior`, `menor` ou `correcao`).

## Seções de lista (itens com id)

O id é gerado pelo script no `inserir` quando não é informado. Campos fora da tabela são recusados.

| Seção (apelido) | Prefixo | Campos obrigatórios | Campos opcionais (padrão) |
|---|---|---|---|
| `contextos` (`ctx`) | `CTX` | `nome`, `responsabilidade` | `relacoes` (`""`) |
| `glossario` (`glo`) | `GLO` | `termo`, `definicao`, `contexto` | `tipo_codigo` (`null`) |
| `modelo_dominio` (`dominio`) | `DOM` | `contexto` | `agregados`, `entidades`, `objetos_valor`, `ciclo_vida`, `eventos` (`[]`) |
| `requisitos_funcionais` (`rf`) | `RF` | `titulo`, `descricao`, `ator`, `contexto`, `criterios` | `regras` (`[]`), `referencias` (`null`), `rastreabilidade` (`[]`), `removido_em` (`null`) |
| `regras_negocio` (`rn`) | `RN` | `titulo`, `regra`, `contexto`, `consequencia` | `referencias` (`null`), `rastreabilidade` (`[]`), `removido_em` (`null`) |
| `requisitos_nao_funcionais` (`rnf`) | `RNF` | `categoria`, `descricao` | `removido_em` (`null`) |
| `integracoes` (`int`) | `INT` | `sistema`, `finalidade`, `direcao`, `contexto` | — |
| `lacunas` (`lac`) | `LAC` | `descricao` | — |

Regras de formato conferidas a cada gravação:

- `glossario.tipo_codigo`: só o nome de um tipo (`Order`, `OrderStatus`), ou `null` quando o termo não tem tipo próprio. Métodos, atributos e caminhos são recusados.
- `glossario.termo`, `contextos.nome` e `modelo_dominio.contexto` são únicos (sem diferenciar maiúsculas e acentos).
- `requisitos_funcionais.criterios`: lista de textos não vazia, de preferência no formato Dado/Quando/Então.
- `requisitos_funcionais.regras`: lista de ids `RN-001`.
- `rastreabilidade` (RF e RN): lista de **termos do glossário**, não de tipos. O documento resolve o tipo pelo glossário e escreve "Pedido é `Order`". Por isso o glossário e a rastreabilidade nunca divergem.
- `modelo_dominio.agregados`, `entidades`, `objetos_valor` e `eventos`: listas de `{ "nome", "descricao" }`. `ciclo_vida`: lista de textos.
- `removido_em`: versão `X.Y.Z` em que o item foi removido. Preenchido pelo comando `remover`.

Item de requisito funcional:

```json
{
  "id": "RF-002",
  "titulo": "Confirmar pedido",
  "descricao": "O cliente pode confirmar um pedido em rascunho, que passa a confirmado e segue para cobrança.",
  "ator": "Cliente",
  "contexto": "Vendas",
  "regras": ["RN-003", "RN-004"],
  "criterios": [
    "Dado um pedido sem itens, quando o cliente tenta confirmar, então o sistema recusa e informa que o pedido está vazio."
  ],
  "referencias": null,
  "rastreabilidade": ["Pedido", "Cliente", "Situação do pedido"],
  "removido_em": null
}
```

## Comandos

Todos recebem o nome do projeto (letras, números, `.`, `-` e `_`). Abreviado como `M` abaixo:

```bash
# $SKILL é a pasta onde a skill está instalada (global ou no projeto); veja o SKILL.md.
M="python3 \"$SKILL/scripts/memoria.py\""
```

| Para | Comando |
|---|---|
| Criar a memória (versão 1.0.0) | `$M iniciar loja --titulo "Loja Online" --autor "Equipe de Vendas"` |
| Registrar um achado novo | `$M inserir loja glossario --json '{"termo": "Pedido", "definicao": "...", "contexto": "Vendas", "tipo_codigo": "Order"}'` (imprime o id criado) |
| Alterar um campo | `$M editar loja rf RF-002 --campo titulo --valor "Confirmar compra"` |
| Alterar um campo de lista ou nulo | `$M editar loja rf RF-002 --campo regras --valor '["RN-003"]'` (o valor é lido como JSON quando possível) |
| Acrescentar a uma lista | `$M editar loja rf RF-002 --campo criterios --valor "Dado ..., quando ..., então ..." --anexar` |
| Alterar vários campos | `$M editar loja rn RN-002 --json '{"regra": "...", "consequencia": "..."}'` |
| Alterar seção de objeto | `$M editar loja visao_geral --campo proposito --valor "..."` |
| Ler | `$M ler loja` · `$M ler loja rf` · `$M ler loja rf RF-002` · `$M ler loja rf RF-002 --campo criterios` |
| Pesquisar | `$M buscar loja "reembolso" [--secao rn] [--json]` |
| Registrar versão | `$M versao loja --tipo menor --mudancas "Inclusão do RF-006"` (imprime a nova versão) |
| Remover | `$M remover loja rf RF-004` (RF, RN e RNF ficam marcados como removidos na última versão; os demais saem de vez) |
| Conferir ligações | `$M verificar loja` |
| Gerar o documento | `$M gerar loja [--md docs/requisitos.md] [--pdf] [--sem-sumario]` |
| Carregar um JSON completo | `$M importar loja --arquivo requisitos.json [--substituir]` |
| Ver onde fica o JSON | `$M caminho loja` |

`gerar` recusa o JSON enquanto `verificar` acusar erro (RN citada que não existe, termo da rastreabilidade fora do glossário ou sem tipo, nenhum RF). Depois de escrever o `.md`, ele roda o `validar_documento.py` e mostra os erros e avisos. O `.md` sai por padrão em `~/.terraforme/<projeto>/requisitos.md` e o PDF ao lado dele (`requisitos.pdf`).

Saída: `0` quando deu certo, `1` com a mensagem `ERRO: ...` quando algo foi recusado. Nada é gravado quando um comando falha.
