# Documento de Requisitos — Loja Online

> Exemplo preenchido a partir de uma aplicação fictícia de comércio eletrônico. Mostra o tom, o nível de detalhe e o único jeito em que o código aparece: como correspondência de tipo ("Pedido é `Order`").

## Controle de versões

| Versão | Data | Autor | Mudanças |
|---|---|---|---|
| 1.0.0 | 2026-09-02 | Equipe de Vendas | Versão inicial com o contexto Vendas (RF-001 a RF-004, RN-001 a RN-005) |
| 1.1.0 | 2026-10-16 | Equipe de Vendas | Inclusão do contexto Pagamentos (RF-005, RN-006) e do requisito de auditoria (RNF-003) |

## 1. Visão geral

- **Propósito:** permite que clientes montem pedidos de produtos do catálogo, paguem com cartão e acompanhem a entrega.
- **Escopo:** contextos de Vendas e Pagamentos.
- **Versão do documento:** 1.1.0
- **Versão da aplicação analisada:** 2.3.0
- **Documentos relacionados:** Política de cancelamento e reembolso (`docs/politicas/cancelamento-e-reembolso.md`)

## 2. Contextos delimitados

| Contexto | Responsabilidade | Relação com outros contextos |
|---|---|---|
| Vendas | Montagem, confirmação e cancelamento de pedidos | Pede a cobrança a Pagamentos; reage à aprovação do pagamento |
| Pagamentos | Cobrança e reembolso junto ao meio de pagamento | Recebe pedidos de cobrança de Vendas; avisa quando o pagamento é aprovado ou recusado |

## 3. Linguagem ubíqua

| Termo de negócio | Definição | Termo no código | Contexto |
|---|---|---|---|
| Cliente | Pessoa que compra na loja; pode estar ativa ou bloqueada | `Customer` | Vendas |
| Pedido | Conjunto de produtos que um cliente quer comprar, com valor total e situação | `Order` | Vendas |
| Item do pedido | Um produto do pedido com quantidade e preço unitário | `OrderItem` | Vendas |
| Situação do pedido | Etapa em que o pedido está: rascunho, confirmado, pago, enviado ou cancelado | `OrderStatus` | Vendas |
| Confirmar pedido | Encerrar a montagem do pedido e enviá-lo para cobrança | — | Vendas |
| Valor | Quantia em dinheiro com moeda, sempre com duas casas decimais | `Money` | Compartilhado |
| Cobrança | Tentativa de receber o valor do pedido no cartão do cliente | `Payment` | Pagamentos |
| Pagamento aprovado | Fato de a cobrança ter sido aceita pelo meio de pagamento | `PaymentApproved` | Pagamentos |
| Pagamento recusado | Fato de a cobrança ter sido recusada pelo meio de pagamento | `PaymentDeclined` | Pagamentos |
| Reembolso | Devolução do valor pago ao cliente | `Refund` | Pagamentos |

## 4. Modelo de domínio

### 4.1 Vendas

**Agregados**

- **Pedido:** concentra os itens, o valor total e a situação. Toda alteração em itens passa pelo pedido.

**Entidades**

- **Item do pedido:** pertence a um pedido; guarda o produto, a quantidade e o preço unitário no momento em que foi adicionado.
- **Cliente:** quem faz o pedido; pode estar ativo ou bloqueado.

**Objetos de valor**

- **Valor:** quantia e moeda; não aceita valores negativos; dois valores só podem ser somados se tiverem a mesma moeda.

**Ciclo de vida**

- Rascunho → Confirmado → Pago → Enviado. Rascunho, Confirmado e Pago podem ir para Cancelado. Enviado e Cancelado são finais.

**Eventos de domínio**

- **Pedido confirmado:** acontece quando o cliente confirma o pedido; Pagamentos reage iniciando a cobrança.
- **Pedido cancelado:** acontece quando o pedido é cancelado; Pagamentos reage reembolsando, se já houve pagamento.

### 4.2 Pagamentos

**Agregados**

- **Cobrança:** representa uma tentativa de receber o valor de um pedido; guarda o valor, o resultado e o código da transação no meio de pagamento.

**Entidades**

- **Reembolso:** pertence a uma cobrança aprovada; registra o valor devolvido e a data.

**Objetos de valor**

- **Valor:** o mesmo conceito usado em Vendas.

**Ciclo de vida**

- Pendente → Aprovada ou Recusada. Uma cobrança aprovada pode ser reembolsada.

**Eventos de domínio**

- **Pagamento aprovado:** acontece quando o meio de pagamento aceita a cobrança; Vendas reage marcando o pedido como pago.
- **Pagamento recusado:** acontece quando o meio de pagamento recusa a cobrança; Vendas reage devolvendo o pedido para rascunho.

## 5. Requisitos funcionais

### RF-001 — Montar pedido

- **Descrição:** O cliente pode adicionar produtos do catálogo a um pedido em rascunho, informando a quantidade de cada um.
- **Ator:** Cliente
- **Contexto:** Vendas
- **Regras relacionadas:** RN-001, RN-002
- **Critérios de aceite:**
  - Dado um pedido em rascunho, quando o cliente adiciona um produto com quantidade 2, então o item aparece no pedido e o valor total é recalculado.
  - Dado um pedido com 50 itens, quando o cliente tenta adicionar mais um, então o sistema recusa e informa o limite.

### RF-002 — Confirmar pedido

- **Descrição:** O cliente pode confirmar um pedido em rascunho, que passa a confirmado e segue para cobrança.
- **Ator:** Cliente
- **Contexto:** Vendas
- **Regras relacionadas:** RN-003, RN-004
- **Critérios de aceite:**
  - Dado um pedido em rascunho com pelo menos um item, quando o cliente confirma, então o pedido fica confirmado e a cobrança é iniciada.
  - Dado um pedido sem itens, quando o cliente tenta confirmar, então o sistema recusa e informa que o pedido está vazio.
  - Dado um cliente bloqueado, quando ele tenta confirmar um pedido, então o sistema recusa a confirmação.

### RF-003 — Cancelar pedido

- **Descrição:** O cliente pode cancelar um pedido que ainda não foi enviado. Se o pedido já estiver pago, o valor é reembolsado.
- **Ator:** Cliente
- **Contexto:** Vendas
- **Regras relacionadas:** RN-005, RN-006
- **Critérios de aceite:**
  - Dado um pedido pago, quando o cliente cancela, então o pedido fica cancelado e o reembolso é solicitado.
  - Dado um pedido enviado, quando o cliente tenta cancelar, então o sistema recusa e informa que o pedido já foi enviado.
- **Referências:** Política de cancelamento e reembolso (`docs/politicas/cancelamento-e-reembolso.md`)

### RF-004 — Registrar resultado da cobrança

- **Descrição:** O sistema deve marcar o pedido como pago quando o pagamento é aprovado e devolvê-lo para rascunho quando é recusado.
- **Ator:** Sistema
- **Contexto:** Vendas
- **Regras relacionadas:** Nenhuma
- **Critérios de aceite:**
  - Dado um pedido confirmado, quando o pagamento é aprovado, então o pedido passa a pago e o cliente recebe o comprovante por e-mail.
  - Dado um pedido confirmado, quando o pagamento é recusado, então o pedido volta para rascunho e o cliente é avisado.

### RF-005 — Reembolsar cobrança

- **Descrição:** O sistema deve devolver ao cliente o valor de uma cobrança aprovada quando o pedido correspondente é cancelado.
- **Ator:** Sistema
- **Contexto:** Pagamentos
- **Regras relacionadas:** RN-006
- **Critérios de aceite:**
  - Dado uma cobrança aprovada há 3 dias, quando o pedido é cancelado, então o reembolso é feito no meio de pagamento e registrado.
  - Dado uma cobrança aprovada há 10 dias, quando o pedido é cancelado, então o reembolso não é feito automaticamente e o caso vai para análise.
- **Referências:** Política de cancelamento e reembolso (`docs/politicas/cancelamento-e-reembolso.md`)

## 6. Regras de negócio

### RN-001 — Quantidade mínima por item

- **Regra:** A quantidade de cada item do pedido deve ser de pelo menos 1.
- **Contexto:** Vendas
- **Consequência quando violada:** o item não é adicionado e o cliente é informado.

### RN-002 — Limite de itens por pedido

- **Regra:** Um pedido pode ter no máximo 50 itens.
- **Contexto:** Vendas
- **Consequência quando violada:** o item não é adicionado e o cliente é informado do limite.

### RN-003 — Pedido vazio não pode ser confirmado

- **Regra:** Somente pedidos com pelo menos um item podem ser confirmados.
- **Contexto:** Vendas
- **Consequência quando violada:** a confirmação é recusada.

### RN-004 — Cliente bloqueado não confirma pedidos

- **Regra:** Um cliente bloqueado não pode confirmar pedidos.
- **Contexto:** Vendas
- **Consequência quando violada:** a confirmação é recusada sem informar o motivo do bloqueio (inferido).

### RN-005 — Pedido enviado não pode ser cancelado

- **Regra:** Um pedido enviado não pode mais ser cancelado.
- **Contexto:** Vendas
- **Consequência quando violada:** o cancelamento é recusado.

### RN-006 — Prazo para reembolso automático

- **Regra:** O reembolso automático só é feito em até 7 dias após a aprovação do pagamento.
- **Contexto:** Pagamentos
- **Consequência quando violada:** o pedido é cancelado, mas o reembolso fica pendente de análise manual.
- **Referências:** Política de cancelamento e reembolso (`docs/politicas/cancelamento-e-reembolso.md`)

## 7. Requisitos não funcionais

- **RNF-001 — Segurança:** somente clientes autenticados podem montar, confirmar ou cancelar os próprios pedidos.
- **RNF-002 — Disponibilidade:** falhas temporárias na comunicação com o meio de pagamento são tentadas de novo até 3 vezes antes de a cobrança ser considerada recusada.
- **RNF-003 — Auditoria:** toda mudança de situação do pedido é registrada com data, hora e responsável.

## 8. Integrações externas

| Sistema externo | Para que é usado | Direção | Contexto |
|---|---|---|---|
| Meio de pagamento | Cobrar e reembolsar no cartão do cliente | Ambos | Pagamentos |
| Serviço de e-mail | Enviar comprovantes e avisos ao cliente | Envia | Vendas |

## 9. Lacunas e dúvidas

- A regra RN-004 não mostra se o cliente deve saber por que foi bloqueado. O sistema só recusa a confirmação. Pergunta: o cliente deve ser informado do motivo?
- O prazo de 7 dias da RN-006 é fixo e a política não diz se vale para todos os produtos. Pergunta: o prazo depende da categoria do produto?
- Não há tratamento para pedido pago que nunca é enviado. Pergunta: existe prazo máximo para o envio?

## 10. Rastreabilidade

| Requisito | Correspondência no código |
|---|---|
| RF-001 | Pedido é `Order`; Item do pedido é `OrderItem` |
| RF-002 | Pedido é `Order`; Cliente é `Customer`; Situação do pedido é `OrderStatus` |
| RF-003 | Pedido é `Order`; Situação do pedido é `OrderStatus`; Reembolso é `Refund` |
| RF-004 | Pagamento aprovado é `PaymentApproved`; Pagamento recusado é `PaymentDeclined`; Pedido é `Order` |
| RF-005 | Cobrança é `Payment`; Reembolso é `Refund` |
| RN-001 | Item do pedido é `OrderItem` |
| RN-002 | Pedido é `Order` |
| RN-003 | Pedido é `Order` |
| RN-004 | Cliente é `Customer` |
| RN-005 | Situação do pedido é `OrderStatus` |
| RN-006 | Reembolso é `Refund`; Cobrança é `Payment` |
