---
name: terraforme-doc
description: >
  Instancia um agente de documentação dentro do terminal do terraforme.
  Use quando o usuário pedir uma documentação ou tarefa simples isolada
  (README, referência de API, guia, comentários de módulo, CHANGELOG,
  resumo de código) durante o `terraforme cli`: em vez de resolver no meio
  da conversa, repasse a tarefa a uma instância dedicada e siga com o que
  estava fazendo. Não use para tarefas que exigem continuidade com o
  contexto atual nem para documentos que o próprio usuário quer ver
  escritos passo a passo.
---

# Agente de documentação do terraforme

Quando o usuário pedir uma documentação (ex.: "faça a doc do módulo X"),
não resolva na conversa atual. Instancie o agente de documentação: o terminal
do terraforme abre uma nova sub-aba do agente já com a tarefa, e ela fica
aberta até o usuário a fechar (`Shift+F12`) — é uma instância, não um
sub-agente interno.

## Como instanciar

Grave o pedido (arquivo único: um pedido de cada vez, com um único comando):

```shell
printf '{"instructions":"%s"}\n' 'INSTRUÇÕES' > "$HOME/.config/terraforme/doc-request.json"
```

Regras do conteúdo de `INSTRUÇÕES`:

- É o prompt completo que o agente de documentação vai receber, em português
  e autocontido: o que documentar (arquivos, módulos, caminhos), o formato
  (ex.: "README em Markdown, seções: instalação, uso, requisitos") e onde
  gravar o resultado (ex.: `docs/api.md`).
- Escape `'` como `'\''` dentro do `printf` e `"` e `\` como `\"` e `\\`
  dentro do JSON; quebra de linha vira `\n` (o terminal digita o `Alt+Enter`
  por você ao montar o prompt).
- Não dependa do contexto desta conversa: a instância não o herda. Copie
  para as instruções tudo o que ela precisar saber.
- Uma tarefa por pedido; para uma segunda documentação, espere a instância
  terminar (ou peça ao usuário para fechá-la com `Shift+F12`).

## Depois de instanciar

1. Avise o usuário: "abri a instância de documentação (sub-aba `📄 doc`);
   vou seguir com a conversa aqui enquanto ela trabalha".
2. Siga com o resto da conversa normalmente.
3. Quando o usuário disser que o resultado está bom, diga que ele pode
   fechar a instância com `Shift+F12` — ou deixá-la aberta para ajustes.

## Depuração

- Se o comando falhar por pasta inexistente, não crie a pasta manualmente:
  informe o erro ao usuário e escreva a documentação na própria conversa.
- Se a instância não abrir alguns segundos depois do pedido (o `terraforme
  cli` precisa estar rodando para assisti-la), resolva a documentação na
  própria conversa e avise o usuário.
