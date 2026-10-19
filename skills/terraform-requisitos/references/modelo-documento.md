# Documento de Requisitos — [Nome da Aplicação]

> Modelo. Copie a estrutura, substitua o que está entre colchetes e remova estas notas. Todas as seções são obrigatórias; quando uma não se aplicar, escreva "Não identificado".
>
> O documento nunca cita arquivos, linhas, métodos ou constantes do código. O código só aparece como correspondência de tipo: na coluna "Termo no código" do glossário e na seção "Rastreabilidade", no formato "Pedido é `Order`". Referências externas só podem apontar para outras documentações.

## Controle de versões

| Versão | Data | Autor | Mudanças |
|---|---|---|---|
| 1.0.0 | [AAAA-MM-DD] | [autor ou equipe] | Versão inicial |

> Versão no formato MAIOR.MENOR.CORREÇÃO, com a mais recente na última linha:
> - **MAIOR:** requisito ou regra removido, ou com significado alterado.
> - **MENOR:** requisito, regra, contexto ou termo novo.
> - **CORREÇÃO:** ajuste de redação, sem mudar o significado.
>
> IDs nunca são reaproveitados nem renumerados. Um item removido continua no documento com o título "(removido na versão X.Y.Z)".

## 1. Visão geral

- **Propósito:** [o que a aplicação faz para o negócio, em uma ou duas frases]
- **Escopo:** [aplicação inteira ou contextos analisados]
- **Versão do documento:** [igual à última linha do controle de versões]
- **Versão da aplicação analisada:** [versão, release ou data]
- **Documentos relacionados:** [outras documentações usadas como fonte, ou "Nenhum"]

## 2. Contextos delimitados

| Contexto | Responsabilidade | Relação com outros contextos |
|---|---|---|
| [Nome do contexto] | [o que esta área do negócio resolve] | [ex.: recebe X de Y; publica Z para W] |

## 3. Linguagem ubíqua

| Termo de negócio | Definição | Termo no código | Contexto |
|---|---|---|---|
| [Termo] | [o que significa para o negócio] | `[NomeDoTipo]` | [Contexto] |
| [Ação ou termo sem tipo próprio] | [o que significa] | — | [Contexto] |

> "Termo no código" aceita apenas o nome de um tipo (classe, interface, enum, evento, registro). Métodos, atributos e constantes não entram; use "—".

## 4. Modelo de domínio

### 4.1 [Nome do contexto]

**Agregados**

- **[Nome do agregado]:** [o que representa e o que ele agrupa]

**Entidades**

- **[Nome da entidade]:** [o que representa; a qual agregado pertence]

**Objetos de valor**

- **[Nome]:** [informação que representa e suas regras de formato]

**Ciclo de vida**

- [Estado inicial] → [Estado] → [Estado final]; [quais transições são permitidas]

**Eventos de domínio**

- **[Nome do evento em linguagem de negócio]:** [quando acontece e quem reage a ele]

## 5. Requisitos funcionais

### RF-001 — [Título curto da ação]

- **Descrição:** [O ator pode / O sistema deve ...]
- **Ator:** [quem realiza: perfil de usuário, outro sistema ou "Sistema"]
- **Contexto:** [Contexto]
- **Regras relacionadas:** [RN-001, RN-002]
- **Critérios de aceite:**
  - Dado [situação inicial], quando [ação], então [resultado esperado].
  - Dado [situação que viola uma regra], quando [ação], então [o sistema recusa e informa o motivo].
- **Referências:** [opcional: outra documentação, ex. `docs/politicas/x.md`]

## 6. Regras de negócio

### RN-001 — [Título curto da regra]

- **Regra:** [enunciado em uma frase]
- **Contexto:** [Contexto]
- **Consequência quando violada:** [o que o sistema faz]
- **Referências:** [opcional: outra documentação]

## 7. Requisitos não funcionais

- **RNF-001 — [Categoria: segurança, desempenho, auditoria, disponibilidade]:** [necessidade em linguagem de negócio]

## 8. Integrações externas

| Sistema externo | Para que é usado | Direção | Contexto |
|---|---|---|---|
| [Nome] | [necessidade de negócio atendida] | [envia / recebe / ambos] | [Contexto] |

## 9. Lacunas e dúvidas

- [Item inferido ou ambíguo, por que há dúvida e a pergunta para o negócio]

## 10. Rastreabilidade

| Requisito | Correspondência no código |
|---|---|
| RF-001 | [Termo do glossário] é `[NomeDoTipo]`; [Termo] é `[NomeDoTipo]` |
| RN-001 | [Termo do glossário] é `[NomeDoTipo]` |

> Cada par usa um termo do glossário e o mesmo tipo que está na coluna "Termo no código".
