# Tradução de código para linguagem de negócio

Use esta tabela para converter o que aparece no código em frases que uma pessoa de negócio entende. A coluna da direita mostra onde o achado entra no documento.

## Padrões de código e como lê-los

| No código | Como ler | Vai para |
|---|---|---|
| Módulo, pacote, microsserviço ou schema com vocabulário próprio | Uma área do negócio | Contextos delimitados |
| Classe com identificador (`id`) que muda ao longo do tempo | Um conceito do negócio com identidade própria | Modelo de domínio: entidade |
| Classe que controla outras e é a única porta de entrada para alterá-las | O conceito central de um processo | Modelo de domínio: agregado |
| Classe imutável comparada pelo valor (dinheiro, endereço, período, e-mail) | Uma informação com regras próprias de formato | Modelo de domínio: objeto de valor |
| Método público de agregado ou serviço de aplicação | Uma ação que alguém pode realizar | Requisito funcional |
| Controller, rota, handler de comando, tela | Quem realiza a ação e por qual canal | Ator do requisito funcional |
| Job agendado, cron, consumidor de fila | Uma ação que o sistema faz sozinho | Requisito funcional com ator "Sistema" |
| `if` que lança exceção, `assert`, guarda no início do método | Algo que não é permitido | Regra de negócio |
| Validação de campo (obrigatório, tamanho, formato, faixa) | Condição para a informação ser aceita | Regra de negócio |
| Enum de status e transições entre eles | O ciclo de vida do conceito | Modelo de domínio e regras de negócio |
| Cálculo (preço, desconto, juros, prazo) | Como um valor é obtido | Regra de negócio com a fórmula em palavras |
| Evento publicado (`...Created`, `...Approved`) | Um fato relevante que aconteceu | Eventos de domínio |
| Listener ou handler de evento | "Quando X acontece, o sistema faz Y" | Requisito funcional ou regra de negócio |
| Checagem de papel ou permissão | Quem pode fazer o quê | Regra de negócio ou requisito não funcional |
| Constraint de banco (único, não nulo, chave estrangeira) | Regra que o sistema garante sempre | Regra de negócio |
| Cliente HTTP, SDK de terceiro, fila externa | Dependência de outro sistema | Integrações externas |
| Middleware de autenticação, auditoria, cache, retentativa, limite de taxa | Qualidade esperada do sistema | Requisitos não funcionais |
| Repository, DTO, mapper, factory técnica, configuração | Detalhe de implementação | Não entra, a menos que revele uma regra |

## Regras de escrita

- **Sujeito é uma pessoa ou o sistema, nunca o código.** "O cliente pode...", "O sistema deve...", "Somente o administrador pode...".
- **Traduza os nomes.** `Order` vira "Pedido", `checkout` vira "finalização da compra", `isActive` vira "ativo". Registre a correspondência no glossário.
- **Expresse condições como regras.** `if (order.status == SHIPPED) throw` vira "Um pedido enviado não pode ser cancelado."
- **Expresse números com unidade e significado.** `MAX_ITEMS = 50` vira "Um pedido pode ter no máximo 50 itens."
- **Expresse fórmulas em palavras.** `total = sum(items.price * qty) - discount` vira "O valor total é a soma de preço vezes quantidade de cada item, menos o desconto."
- **Uma regra por RN.** Se um método tem três guardas, são três regras.
- **Marque o que é dedução.** Quando a intenção não está clara no código, escreva "(inferido)" e registre a dúvida.

## Exemplos de tradução

| Trecho de código | Em linguagem de negócio |
|---|---|
| `if (customer.isBlocked) throw new CustomerBlockedError()` | Um cliente bloqueado não pode fazer pedidos. |
| `@Min(1) quantity` | A quantidade de cada item deve ser de pelo menos 1. |
| `if (daysSince(order.paidAt) > 7) throw RefundWindowExpired` | O reembolso só pode ser pedido em até 7 dias após o pagamento. |
| `@Roles('admin') deleteProduct()` | Somente administradores podem remover produtos do catálogo. |
| `on(PaymentApproved) -> order.markAsPaid(); sendReceipt()` | Quando o pagamento é aprovado, o pedido passa a "pago" e o cliente recebe o comprovante. |
| `retry(3, backoff=exponential)` em chamada ao gateway | Falhas temporárias na comunicação com o meio de pagamento são tentadas de novo até 3 vezes. |
