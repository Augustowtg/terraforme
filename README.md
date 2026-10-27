# terraforme

CLI multiplataforma (Linux e Windows) para salvar e alternar entre diretórios de trabalho pelo terminal, com interface ratatui no tema Catppuccin Mocha.

O binário se chama `terra`. O dia a dia é feito pelo comando `terra`, uma função de shell instalada junto com ele.

## Instalação

É preciso ter o Rust instalado (`cargo`). Se não tiver, instale em <https://rustup.rs>.

Os instaladores mostram o banner TERRAFORME, a versão, uma barra de progresso com os passos e pedem confirmação antes de começar: escolha **SIM** ou **NÃO** com `←`/`→` e confirme com `Enter`. Para instalar sem a pergunta (ex.: fora de um terminal interativo), passe `-y`. Se um passo falhar, o instalador mostra o fim do log de compilação.

### Linux

```bash
./install-linux.sh
```

O script:

- compila em modo release;
- copia o binário para `~/.local/bin/terra`;
- adiciona a função `terra` ao `~/.bashrc`, ao `~/.zshrc` (se o zsh estiver instalado) e ao `~/.config/fish/config.fish` (se o fish estiver instalado).

O bloco fica entre `# >>> terraforme >>>` e `# <<< terraforme <<<`. Rodar o script de novo substitui o bloco em vez de duplicar.

`~/.local/bin` precisa estar no `PATH`; o script avisa se não estiver.

### Windows (Git Bash)

```bash
./install-win.sh
```

O script:

- compila o projeto;
- copia `terra.exe` para `~/.terraforme/bin`;
- adiciona ao `~/.bashrc` essa pasta no `PATH` e a função `terra`, com os mesmos marcadores;
- garante que o `~/.bash_profile` carregue o `~/.bashrc`.

Depois, feche e reabra o Git Bash. PowerShell e CMD não são suportados nesta versão.

## Uso

| Comando | O que faz |
| --- | --- |
| `terra --version` | Mostra a versão |
| `terra cli` | Abre a interface com as abas Chat, Consumo e Config e registra o diretório atual em **Recentes** |
| `terra workspace` | Lista Recentes e Salvos; `Enter` entra no diretório escolhido |
| `terra workspace --save` | Salva o diretório atual como workspace |
| `terra workspace --save --name <nome>` | Salva com um nome definido por você |

### Navegação no `terra cli`

| Tecla | Ação |
| --- | --- |
| `Tab` | Troca de aba (Chat, Consumo, Config) |
| `i` / `a` (aba Chat) | Entra no modo INSERT da caixa de mensagem |
| `Enter` (escrevendo) | Envia a mensagem ao Claude |
| `/` no começo do input | Abre a lista de comandos do agente (com descrição), filtrada pelo que vier depois da `/`. `Tab`/`Enter` executam; cada comando é uma interface do terra (hoje, `/clear`: conversa nova; `/compact`: o agente resume a conversa e libera o contexto; `/model`: modelo e esforço; `/login`: login na conta; `/usage`: aba Consumo). A lista tem todos os comandos do Claude: os embutidos, os do modo interativo (ex.: `/help`, `/status`, `/resume`) e as skills e plugins, e os que ainda não têm aparecem esmaecidos |
| `/login` | Entra de novo na conta escolhida na Config (token expirado ou revogado). O terra sai da tela e roda o `claude auth login` da conta, com o e-mail dela: o link aparece inteiro e clicável (e já copiado), e o código colado aparece ao digitar; `Enter` envia e `Enter` vazio cancela. No fim, o terra volta |
| `/usage` | Abre a aba **Consumo** (limites de uso, crédito extra e o gráfico por dia) |
| `/model` | Escolhe o modelo e o esforço desta conversa (`↑`/`↓` modelo, `←`/`→` esforço, `Enter` escolhe). O item Default abre o menu do modelo padrão, que fica salvo |
| `@` (escrevendo) | Abre a lista de arquivos e pastas do diretório, filtrada pelo que vier depois do `@` e atualizada em tempo real. `↑`/`↓` escolhem, `Tab`/`Enter` inserem o caminho como um badge cinza (`@/src/main.rs`), `Esc` fecha |
| `Esc` no modo NORMAL, com o Claude respondendo | Interrompe a resposta |
| `PageUp` / `PageDown`, roda do mouse | Rola o histórico do chat (a barra de rolagem aparece ao rolar e some 2 s depois) |
| Arrastar no histórico / duplo clique numa mensagem | Seleciona o trecho ou a mensagem inteira e copia para a área de transferência |
| `Insert` (escrevendo) | Alterna entre inserir e sobrescrever os caracteres à frente do cursor. Quando ligado, `● Insert` aparece na borda do input (assim como `● Caps` com o Caps Lock) |
| `Shift+Enter` (escrevendo) | Pula uma linha. No GNOME Terminal e em outros que não diferenciam o `Shift+Enter`, use `\` + `Enter`, `Alt+Enter` ou `Ctrl+J` |
| `Esc` no modo INSERT | Volta ao modo NORMAL |
| Mouse no input (aba Chat) | Clique posiciona o cursor; arrastar seleciona o texto (digitar substitui a seleção) |
| `Shift+Tab` (aba Chat) | Alterna o modo de permissão: Agente → Yolo → Manual → Accept Edits → Plan Mode. Fora do padrão (Agente), aparece o badge `Modo: ...`; o de consumo aparece depois do primeiro token. No canto direito da mesma linha, embaixo do status, a **barra de contexto** mostra o tamanho do contexto da sessão (ex.: `45K / 150K (30%)`): verde, laranja a partir da metade do ponto de compactar e vermelho a partir de 75% dele; no ponto, ela mostra **Compacte a conversa**. A barra vem oculta: para vê-la, troque o Display na seção Contexto da aba Config |
| `r` (aba Consumo) | Atualiza o consumo na hora (ele também se atualiza sozinho a cada minuto com a aba aberta) |
| `c` ou clique nos botões (aba Consumo) | Alterna o gráfico Consumo por dia entre **Sem cache** (padrão) e **Com cache** |
| Clique num dia (aba Consumo) | Mostra o consumo daquele dia por hora; **‹ Dias** ou `Esc` volta |
| `Esc` | Leva o foco ao botão vermelho **Sair**; de novo (ou `Tab`), volta às abas |
| `Enter` em **Sair** | Abre a confirmação de saída |
| `←` / `→` | Alterna entre **Sim** (verde) e **Não** (vermelho) na confirmação |
| `Enter` na confirmação | **Sim** sai; **Não** fecha a caixa |
| Saindo de um workspace não salvo | Pergunta se salva: **Sim** (salva e sai), **Não** (sai) ou **Não (não perguntar novamente)** (sai e não pergunta mais neste workspace). `Esc` desiste de sair |
| `Ctrl+C` | Sai direto, sem confirmação |

### Aba Consumo

Mostra o **route** (provedor e conta do chat, com o plano) e, para cada limite de uso da conta, uma barra de 0 a 100% (verde, amarela a partir de 70% e vermelha a partir de 90%) com quando ela zera:

```text
 Consumo
  Route           Claude · Default · eu@gmail.com · Pro

  Sessão (5 h)
  ██░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░  6%
  Reseta 27-10-2026 14:10 · zera em 4h 52m

  Semana (todos os modelos)
  ██████████████████████████████████████░░  96%
  Reseta 27-10-2026 12:00 · zera em 2h 42m

  Crédito extra (mensal)
  ██░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░  4%
  US$ 1,78 de US$ 40,00 usados no mês
```

A barra do **Crédito extra** só aparece quando a conta tem o crédito extra ativo.

Ao lado das barras, com a mesma altura da seção dos limites (embaixo, em telas com menos de 100 colunas), fica o gráfico **Consumo por dia**: os tokens de cada um dos últimos 14 dias, com o de hoje em destaque, o maior valor e o resumo (`Hoje · média/dia · total`). Os botões **Sem cache** (verde, o padrão: entrada e saída) e **Com cache** (vermelho: soma a criação e a leitura de cache) trocam o que é somado, com um clique ou com `c`. **Clicar num dia** mostra as horas daquele dia (00 a 23), com o consumo do dia embaixo (total e o horário de pico); **‹ Dias** ou `Esc` volta aos dias. No Claude, ele soma os transcripts do Claude Code da conta (`projects/**/*.jsonl`, incluindo as conversas do terra), no fuso local; não depende do login.

No Claude, os limites são os mesmos do `/usage` do Claude Code (sessão de 5 h e semana, mais Opus/Sonnet quando o plano tem), lidos do endpoint de uso com o token da conta escolhida na Config. Não chama o modelo nem gasta tokens; precisa do `curl` no `PATH`.

### Aba Config

A seção **Agente** reúne as configurações do agente do chat, que são de cada workspace. `↑`/`↓` escolhem e `←`/`→` ou `Enter` trocam o valor; a mudança é salva e vale na hora.

As configurações ficam em `~/.terraforme/workspace/<nome do workspace>/config.json`. O nome é o do workspace salvo para o diretório (`terra workspace --save`) ou, sem um, o nome da pasta. O arquivo é lido ao iniciar; um workspace sem ele começa com o padrão do router (o provedor): modo **Do provedor**, modelo e esforço padrão e a conta **Default**.

```json
{
  "agent": {
    "start_mode": "accept_edits",
    "model": "sonnet",
    "account": ".claude-trabalho",
    "effort": "high",
    "interface": "terraforme",
    "context": { "display": "hidden", "auto_compact": false, "limit": 200000, "compact_at": 75 }
  }
}
```

| Configuração | Valores |
| --- | --- |
| Provedor | Qual agente o chat usa e se está disponível (só leitura) |
| Conta | Default (a conta padrão do `claude`) ou uma das contas encontradas em `~/.claude` e `~/.claude-*`, como Pessoal e Trabalho, com o e-mail de cada uma |
| Modo inicial | Do provedor (o `defaultMode` do Claude), Agente, Yolo, Manual, Accept Edits ou Plan |
| Modelo | Padrão ou um modelo do provedor (Opus, Sonnet, Haiku), enviado com `--model` |
| Interface | **Agente Terraforme** (o chat e o input do terra, o padrão) ou **Original do provedor**: a aba Chat roda o `claude` interativo no workspace, dentro da tela do terra (abaixo da barra de abas, com o rodapé), aberto ao iniciar e com `Enter`, com a conta, o modelo, o esforço e o modo inicial da Config (o Yolo vira `bypassPermissions`). As teclas vão para o `claude` (inclusive `Tab`, `Esc` e `Ctrl+C`); `Ctrl+Q` volta às abas e `Enter` na aba Chat volta ao terminal. Trocar a Interface não fecha o `claude` se já foi enviada alguma mensagem: a sessão reaparece ao voltar |

Dentro da seção Agente, separada por uma linha, fica a seção **Contexto**, da barra de contexto do chat:

| Configuração | Valores |
| --- | --- |
| Display | **Oculto** (o padrão) ou **Visível**: mostra ou esconde a barra de contexto |
| Auto compactação | **Não** (o padrão) ou **Sim**: ao fim de uma resposta que deixou o contexto no ponto de compactar, o terra manda o `/compact` sozinho |
| Limite de tokens | Um limite fixo, de 50K a 1M (padrão **200K**), ou **Do modelo** (o limite que o agente informa, ou 200K até ele informar). No `config.json`, `"limit"` aceita qualquer número de tokens, e `null` é o do modelo |
| Compactar em | O ponto de compactar, de 50% a 95% do limite de tokens (padrão **75%**), com os tokens que ele representa (ex.: `75% · 150K tokens`). A barra vai até ele: fica laranja na metade, vermelha a partir de 75% dele e mostra **Compacte a conversa** ao chegar nele |

### Navegação no `terra workspace`

| Tecla | Ação |
| --- | --- |
| `↑` / `k` | Sobe |
| `↓` / `j` | Desce |
| `Home` / `g` | Primeiro item |
| `End` / `G` | Último item |
| `Enter` | Abre o workspace ou executa "...Mostrar mais" |
| `q` / `Esc` / `Ctrl+C` | Sai sem trocar de diretório |

Cada seção mostra 5 itens. "...Mostrar mais" carrega mais 5 e some quando a lista acaba. Workspaces cujo diretório não existe mais aparecem esmaecidos e não podem ser abertos.

### Regras de nome ao salvar

- O nome padrão é o nome da pasta. Na raiz do disco, o nome é `root`.
- Se o nome já existir para outro diretório, ganha um sufixo: `projeto-2`, `projeto-3`...
- Com `--name`, um nome repetido gera erro (código de saída 1). Espaços nas pontas são removidos, e o nome não pode ficar vazio.
- Um diretório já salvo não é salvo de novo. O comando mostra "Este path já está salvo como [nome]".

## Chat com o Claude

A aba Chat do `terra cli` usa o [Claude CLI](https://claude.com/claude-code) por baixo, então é preciso ter o `claude` instalado, no `PATH` e com login feito. Sem ele, o status fica **Desconectado**.

Cada mensagem roda `claude -p` (entrada e saída em `stream-json`) no diretório onde o `terra cli` foi aberto. As seguintes continuam a mesma sessão com `--resume`. O badge de permissão define o `--permission-mode`:

| Badge | `--permission-mode` |
| --- | --- |
| Agente | `auto` |
| Yolo | `manual`, e o terra aprova automaticamente cada pedido de permissão do agente |
| Manual | `manual` |
| Accept Edits | `acceptEdits` |
| Plan Mode | `plan` |

Para gastar menos tokens, o chat não carrega o ambiente do seu Claude Code: servidores MCP, skills e hooks ficam de fora (`--strict-mcp-config`, `--disable-slash-commands` e `disableAllHooks`). As ferramentas embutidas (Bash, Read, Edit, Write, Grep, WebFetch...) continuam disponíveis.

O modo fica sincronizado com o agente, menos o Yolo, que é só do terra: nele o agente pede permissão e o terra aceita tudo. O `terra cli` começa no `permissions.defaultMode` configurado no Claude (settings local, do projeto ou do usuário), e a cada resposta confere o modo em que o agente diz estar; se for outro, o terminal passa a mostrar o modo real. Trocar com `Shift+Tab` durante uma resposta mostra `Modo: atual → próximo`, e o novo modo vale a partir da próxima mensagem. Com o agente ativo, o badge de modo fica sempre visível.

No modo Manual ainda não há como aprovar ferramentas pela interface: as que pedem aprovação são bloqueadas, e o histórico avisa quantas foram.

## Como o `terra` troca de diretório

Um processo não consegue mudar o diretório do shell que o chamou. Por isso `terra` é uma função de shell:

1. cria um arquivo temporário e chama `terra <args> --cwd-file <arquivo>`;
2. ao escolher um workspace, o binário grava o path nesse arquivo e encerra;
3. a função lê o arquivo e, se o diretório existir, faz `cd`. Depois apaga o arquivo.

Rodando o binário direto (`command terra workspace`), o path escolhido é impresso no terminal.

## Dados

Tudo fica em `~/.terraforme/terra.sqlite`, criado na primeira execução.

- `recents`: `id`, `path` (único), `last_used_at` (epoch em ms)
- `workspaces`: `id`, `name` (único), `path` (único), `created_at` (epoch em ms)
- `settings`: as configurações globais de antes da 0.4.12; desde a 0.4.14 não são mais usadas

As configurações de cada workspace ficam em `~/.terraforme/workspace/<nome>/config.json` (veja a aba Config).

Os paths são canonicalizados antes de serem salvos: ficam absolutos, sem `..` e sem barra no final. No Windows, a comparação de paths não diferencia maiúsculas de minúsculas.

## Desenvolvimento

```bash
cargo run -- workspace   # roda sem instalar
cargo test               # testes da camada de banco
```

O código é organizado por funcionalidade: cada módulo tem uma responsabilidade, os testes de unidade ficam junto do código (`#[cfg(test)]`), e módulos com muitos testes os separam num `tests.rs` próprio.

```
src/
├── main.rs, cli.rs, db.rs, theme.rs
├── settings.rs           # configurações da aba Config, salvas no banco
├── files.rs              # índice dos arquivos do diretório, atualizado em tempo real
├── agent.rs              # o agente por trás do chat (só o contrato)
│   ├── adapter.rs        # ChatAdapter, Session e Request: o contrato
│   ├── event.rs          # AgentEvent: o que volta de uma resposta
│   ├── mode.rs           # PermissionMode
│   └── adapters.rs       # registro dos provedores (routes)
│       └── claude.rs     # ClaudeAdapter: o Claude CLI no contrato
│           ├── accounts.rs # contas do Claude (pastas ~/.claude*)
│           ├── catalog.rs  # comandos e modelos (pedido initialize)
│           └── stream.rs # parser do stream-json
├── platform.rs           # integração com o sistema
│   ├── clipboard.rs      # área de transferência (wl-copy, xclip, xsel ou OSC 52)
│   └── keyboard.rs       # Caps Lock (LED no Linux, API no Windows)
└── tui.rs                # loop da tela principal e do terminal
    ├── app.rs            # tela principal: abas, Sair e confirmação
    ├── workspace.rs      # navegador de workspaces
    ├── config.rs         # aba Config: seção Agente
    ├── widgets.rs → widgets/badge.rs, widgets/popup.rs
    └── chat.rs           # aba Chat: liga o adapter, histórico, input e badges
        ├── mode.rs       # como status e modo aparecem
        ├── input.rs      # caixa de input: edição, cursor, seleção, Insert
        │   └── tests.rs
        ├── mention.rs    # menções de arquivo com @: lista, filtro e badge
        ├── commands.rs   # comandos com /: lista do agente e interfaces do terra
        ├── model_menu.rs # interface do /model: modelo e esforço
        ├── history.rs    # histórico: quebra de linha, rolagem, seleção
        └── tests.rs      # testes do Chat (com um adapter de mentira)
```

| Módulo | Responsabilidade |
| --- | --- |
| `src/cli.rs` | Argumentos (clap) e execução dos comandos |
| `src/db.rs` | SQLite, canonicalização de paths e regras de nome |
| `src/agent/` | Agente do chat: eventos e integração com o Claude CLI |
| `src/platform/` | Área de transferência e estado do teclado |
| `src/tui/` | Telas ratatui: tela principal, chat, consumo, config e workspaces |
| `src/theme.rs` | Cores do Catppuccin; troque `FLAVOR` para mudar de variante |

### Adicionar um provedor (route) ao chat

O chat usa o padrão **Adapter**: ele só conhece o trait `ChatAdapter` (`src/agent/adapter.rs`), e cada provedor o implementa traduzindo o seu protocolo. Para adicionar um:

1. Crie `src/agent/adapters/<provedor>.rs` com uma struct que implemente `ChatAdapter`:
   - `name()`: o nome no botão do título (ex.: `"Claude"`);
   - `is_available()`: se o provedor está instalado ou configurado (sem ele, o chat fica Desconectado com o `unavailable_hint()`);
   - `start(Request)`: começa uma resposta e devolve uma `Session`, cujo `poll()` entrega os `AgentEvent`s até `Done` ou `Failed`. No `PermissionMode::Yolo`, aprove tudo o que o agente pedir;
   - opcionais: `configured_mode()` (modo inicial vindo da configuração do provedor), `model_label()` (nome amigável do modelo), `load_usage()` e `load_daily()` (o consumo e os tokens por dia da aba Consumo, em segundo plano) e `login()` (o comando interativo do `/login`).
2. Declare o módulo e inclua-o na lista de `all()` em `src/agent/adapters.rs`.

Nada na interface muda: o histórico, os modos, o consumo e a sincronia do modo funcionam com qualquer adapter. O `src/tui/chat/tests.rs` tem um `FakeAdapter` que serve de exemplo mínimo.
| `install-ui.sh` | Interface dos instaladores (banner, progresso e confirmação) |
