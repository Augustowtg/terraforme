# terraforme

CLI multiplataforma (Linux e Windows) para salvar e alternar entre diretórios de trabalho pelo terminal, com interface ratatui no tema Catppuccin Mocha.

O binário se chama `terra`. O dia a dia é feito pelo comando `terra`, uma função de shell instalada junto com ele.

## Instalação

A instalação [direto do GitHub](#direto-do-github) baixa o binário já compilado da última release, sem precisar do Rust. Os instaladores rodados de um clone compilam o código: é preciso ter o Rust instalado (`cargo`); se não tiver, instale em <https://rustup.rs>.

Os instaladores mostram o banner TERRAFORME, a versão, uma barra de progresso com os passos e pedem confirmação antes de começar: escolha **SIM** ou **NÃO** com `←`/`→` e confirme com `Enter`. Para instalar sem a pergunta (ex.: fora de um terminal interativo), passe `-y`. Se um passo falhar, o instalador mostra o fim do log de compilação.

Se o **terraforme antigo** (o de antes do terra, com o `terraforme-mcp` e o `terraforme-pet`) estiver na máquina, aparece o passo **Desinstalar o terraforme antigo**, com SIM ou NÃO (com `-y`, desinstala). Com SIM, o instalador encerra o daemon e o mascote (no Windows, também o `terraforme`), remove os binários e guarda a config e os dados dele com o sufixo `.antigo-<data>` (`~/.config/terraforme` e `~/.local/share/terraforme` no Linux; `%APPDATA%\terraforme` e `%LOCALAPPDATA%\terraforme` no Windows), em vez de apagar. O RTK, o ai-memory e o `~/.terraforme` do terra ficam. Sem o antigo, o passo nem aparece.

Depois, o instalador pergunta se é para instalar as [extensões](#aba-extensões) (SIM ou NÃO, como na confirmação; com `-y`, instala as que faltam). Com SIM, aparece um menu no estilo da lista de extensões das Configurações (`[x] 1. RTK - descrição`), com as duas marcadas: `↑`/`↓` escolhem, `Espaço` marca ou desmarca e `Enter` confirma. As que já estão instaladas (no `PATH`) aparecem com uma bola verde (`● 1. RTK - descrição (instalada)`), sem a caixa, e não entram na instalação; com as duas instaladas, o instalador nem pergunta. Só então os passos das escolhidas entram na lista do instalador. Para cada uma:

- instala o **RTK** com `cargo install --git https://github.com/rtk-ai/rtk rtk` (se o `rtk` ainda não estiver no `PATH`). O RTK não precisa de setup: o terra passa o hook dele ao agente;
- instala o **ai-memory** pelo instalador oficial (`install.sh` no Linux, `install.ps1` pelo PowerShell no Windows), se ainda não estiver no `PATH`, e roda o `ai-memory setup --yes`: o servidor como serviço do usuário (ou ao fazer login, no Windows) e os hooks, o MCP e as skills nos agentes que ele detectar. Rodar de novo não muda nada.

Com NÃO, os passos das extensões nem aparecem. O LLM do ai-memory (para a consolidação das memórias) continua desligado até você configurar um provedor (`AI_MEMORY_LLM_PROVIDER`, ver a documentação do ai-memory).

### Direto do GitHub

Sem clonar o repositório, um comando baixa a última release (o código e o binário compilado para Linux x86_64 ou Windows x86_64) e roda o instalador do sistema, que instala o binário baixado em vez de compilar. Assim, não precisa do Rust:

```bash
# Linux ou Git Bash
curl -fsSL https://raw.githubusercontent.com/Augustowtg/terraforme/main/install.sh | bash
```

```powershell
# Windows (PowerShell; precisa do Git for Windows, que roda o instalador)
irm https://raw.githubusercontent.com/Augustowtg/terraforme/main/install.ps1 | iex
```

Para instalar sem a pergunta, use `curl ... | bash -s -- -y`. Para outra branch ou tag, defina `TERRAFORME_REF` (ex.: `TERRAFORME_REF=v0.8.17`; no PowerShell, `$env:TERRAFORME_REF='v0.8.17'` antes do `irm`). Numa tag sem binário compilado (as anteriores à v0.8.17), numa branch (como a `main`) ou em outro processador, o instalador compila com o `cargo` o código baixado, como os instaladores abaixo com `--dev`. O RTK das extensões é compilado com o `cargo`: sem o Rust, o instalador pula o RTK e avisa no fim. Os passos são os mesmos dos instaladores abaixo.

Quando há versão nova, a aba **Atualização** aparece no `terra cli` com o que mudou, e o `Enter` nela instala. Para atualizar pelo terminal, rode `terra upgrade`: ele roda o mesmo comando (o `install.sh` no Linux, o `install.ps1` no Windows), sem clonar o repositório: antes, confere no CHANGELOG do GitHub se há versão nova (se não há, avisa e para), instala sem perguntar e, no fim, mostra o que mudou. Com `--repo`, roda o instalador do repositório do diretório atual (ou de um diretório acima): o `install-linux.sh` no Linux e o `install.ps1` no Windows, que, rodado de um clone, chama o `install-win.sh` do próprio clone em vez de baixar. Os argumentos depois passam para o instalador: `terra upgrade -y`, `terra upgrade --repo --dev` (compila e instala o código do repositório).

Os binários vêm do workflow `release` do GitHub Actions: a cada tag `vX.Y.Z`, ele compila o `terra` para Linux (`terra-x86_64-unknown-linux-gnu`) e Windows (`terra-x86_64-pc-windows-msvc.exe`) e anexa os dois à release da tag, criando a release com as notas do CHANGELOG se ela ainda não existir. Para uma tag já publicada, rode o workflow pelo Actions com a tag.

### Linux

```bash
./install-linux.sh        # instala a última release do GitHub
./install-linux.sh --dev  # compila e instala este repositório
```

O script:

- baixa o binário da última release do GitHub (só x86_64) ou, com `--dev`, compila este repositório em modo release (precisa do Rust);
- copia o binário para `~/.local/bin/terra`;
- adiciona a função `terra` ao `~/.bashrc`, ao `~/.zshrc` (se o zsh estiver instalado) e ao `~/.config/fish/config.fish` (se o fish estiver instalado).

O bloco fica entre `# >>> terraforme >>>` e `# <<< terraforme <<<`. Rodar o script de novo substitui o bloco em vez de duplicar.

`~/.local/bin` precisa estar no `PATH`; o script avisa se não estiver.

### Windows (Git Bash)

```bash
./install-win.sh        # instala a última release do GitHub
./install-win.sh --dev  # compila e instala este repositório
```

O script:

- baixa o binário da última release do GitHub ou, com `--dev`, compila este repositório (precisa do Rust);
- copia `terra.exe` para `~/.terraforme/bin`;
- adiciona ao `~/.bashrc` essa pasta no `PATH` e a função `terra`, com os mesmos marcadores;
- garante que o `~/.bash_profile` carregue o `~/.bashrc`;
- põe `~/.terraforme/bin` no `PATH` do usuário, para o `terra` funcionar no PowerShell e no CMD;
- grava a função `terra` nos perfis do PowerShell (5.1 e 7, em `Documentos\WindowsPowerShell` e `Documentos\PowerShell`), com os mesmos marcadores, para o `terra workspace` entrar no diretório escolhido. Com a política de execução `Restricted` ou `AllSigned`, que não carrega o perfil, o passo pula os perfis e avisa: o `terra` funciona, mas o `terra workspace` só mostra o path (`Set-ExecutionPolicy -Scope CurrentUser RemoteSigned` e o instalador de novo resolvem). No CMD, o `terra workspace` também só mostra o path.

Depois, abra um terminal novo (Git Bash, PowerShell ou CMD). Pelo `irm ... | iex`, o `terra` já fica no `PATH` da janela do PowerShell em que você instalou.

## Uso

| Comando | O que faz |
| --- | --- |
| `terra --version` | Mostra a versão |
| `terra cli` | Abre a interface com as abas Chat, Consumo, Extensões, Orquestrador e Configurações e registra o diretório atual em **Recentes** |
| `terra workspace` | Lista Recentes e Salvos; `Enter` entra no diretório escolhido |
| `terra workspace --save` | Salva o diretório atual como workspace |
| `terra workspace --save --name <nome>` | Salva com um nome definido por você |
| `terra upgrade` | Atualiza o terra para a última release pelo instalador do GitHub, sem clonar o repositório, e mostra o que mudou |
| `terra upgrade --repo` | Atualiza pelo instalador do repositório do diretório atual (`install-linux.sh` no Linux, `install.ps1` no Windows). Os argumentos passam para o instalador (ex.: `terra upgrade --repo --dev`) |

### Navegação no `terra cli`

| Tecla | Ação |
| --- | --- |
| `Tab` | Troca de aba (Chat, Consumo, Extensões, Orquestrador e Configurações) |
| `i` / `a` (aba Chat) | Entra no modo INSERT da caixa de mensagem |
| `Enter` (escrevendo) | Envia a mensagem ao Claude |
| `↑` / `↓` (escrevendo) | Mostram as mensagens já enviadas (desde que o terra abriu, de todas as sessões), da mais nova para a mais antiga. Com o texto em várias linhas, primeiro andam entre as linhas. Depois da mais nova, o `↓` volta ao que estava sendo escrito |
| `/` no começo do input | Abre a lista de comandos do agente (com descrição), filtrada pelo que vier depois da `/`. `Enter` executa; cada comando é uma interface do terra (hoje, `/clear`: conversa nova; `/compact`: o agente resume a conversa e libera o contexto; `/model`: modelo e esforço; `/login`: login na conta; `/usage`: aba Consumo). A lista tem todos os comandos do Claude: os embutidos, os do modo interativo (ex.: `/help`, `/status`, `/resume`) e as skills e plugins, e os que ainda não têm aparecem esmaecidos, com "padrão do router": escolhidos com `Enter`, eles vão para o agente e a resposta dele aparece no chat (ex.: o `/context` e o `/cost` do Claude); com `Tab`, completam o input (ex.: `/config `) para receber argumentos (ex.: `/config model=opus`). Os que só existem no modo interativo do Claude respondem que não estão disponíveis |
| `/login` | Entra de novo na conta escolhida nas Configurações (token expirado ou revogado). O terra sai da tela e roda o `claude auth login` da conta, com o e-mail dela: o link aparece inteiro e clicável (e já copiado), e o código colado aparece ao digitar; `Enter` envia e `Enter` vazio cancela. No fim, o terra volta |
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
| `Shift+Tab` (aba Chat) | Alterna o modo de permissão: Agente → Yolo → Manual → Accept Edits → Plan Mode. Fora do padrão (Agente), aparece o badge `Modo: ...`; o de consumo aparece depois do primeiro token. No canto direito da mesma linha, embaixo do status, a **barra de contexto** mostra o tamanho do contexto da sessão (ex.: `45K / 150K (30%)`): verde, laranja a partir da metade do ponto de compactar e vermelho a partir de 75% dele; no ponto, ela mostra **Compacte a conversa**. A barra vem oculta: para vê-la, troque o Display na seção Contexto da aba Configurações. À esquerda dela, a **barra de consumo** mostra quanto resta da sessão de 5 horas do provedor e quando ela zera (ex.: `58% · reseta 15:00`): verde, laranja de 50% para baixo, vermelha de 25% para baixo e cinza ao chegar em 0%. Ela vem oculta; para vê-la, troque o Display na seção Consumo da aba Configurações. Visível, é atualizada a cada minuto |
| `↑` / `↓` (aba Extensões) | Escolhem a extensão; `r` atualiza o relatório dela |
| `Shift+Tab` (aba Configurações) | Alterna entre a seção Agente e a seção Extensões |
| `Espaço` (aba Configurações, Extensões) | Passa a extensão em foco de inativa para ativa nesta conta, usar sempre e inativa de novo; `←`/`→` trocam de página |
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

Mostra o **route** (provedor e conta do chat, com o plano) e, para cada limite de uso da conta, uma barra com **quanto ainda resta** do limite (cheia quando nada foi usado; verde, amarela quando restam 30% ou menos e vermelha quando restam 10% ou menos) com quando ela zera:

```text
 Consumo
  Route           Claude · Default · eu@gmail.com · Pro

  Sessão (5 h)
  ██████████████████████████████████████░░  94% restantes
  Reseta 27-10-2026 14:10 · zera em 4h 52m

  Semana (todos os modelos)
  ██░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░  4% restantes
  Reseta 27-10-2026 12:00 · zera em 2h 42m

  Crédito extra (mensal)
  ██████████████████████████████████████░░  96% restantes
  US$ 38,22 restantes de US$ 40,00 no mês
```

A barra do **Crédito extra** só aparece quando a conta tem o crédito extra ativo.

Ao lado das barras, com a mesma altura da seção dos limites (embaixo, em telas com menos de 100 colunas), fica o gráfico **Consumo por dia**: os tokens de cada um dos últimos 14 dias, com o de hoje em destaque, o maior valor e o resumo (`Hoje · média/dia · total`). Os botões **Sem cache** (verde, o padrão: entrada e saída) e **Com cache** (vermelho: soma a criação e a leitura de cache) trocam o que é somado, com um clique ou com `c`. **Clicar num dia** mostra as horas daquele dia (00 a 23), com o consumo do dia embaixo (total e o horário de pico); **‹ Dias** ou `Esc` volta aos dias. No Claude, ele soma os transcripts do Claude Code da conta (`projects/**/*.jsonl`, incluindo as conversas do terra), no fuso local; não depende do login.

No Claude, os limites são os mesmos do `/usage` do Claude Code (sessão de 5 h e semana, mais Opus/Sonnet quando o plano tem), lidos do endpoint de uso com o token da conta escolhida nas Configurações. Não chama o modelo nem gasta tokens; precisa do `curl` no `PATH`.

### Aba Extensões

Extensões são aplicações secundárias que melhoram o agente do chat. Hoje:

| Extensão | O que faz | Como entra no agente |
| --- | --- | --- |
| [RTK](https://github.com/rtk-ai/rtk) | Filtra e comprime a saída dos comandos antes de ela chegar ao agente (60-90% menos tokens) | Hook `PreToolUse` do Bash (`rtk hook claude`), que troca o comando pelo do RTK (ex.: `git status` → `rtk git status`); o agente também inicia pelo `rtk` (`rtk claude`) |
| [ai-memory](https://github.com/akitaonrails/ai-memory) | Memória de longo prazo entre sessões e agentes, num wiki versionado com git | Servidor MCP e hooks do ciclo de vida, os que o próprio `ai-memory install-mcp`/`install-hooks` imprime para o Claude Code; o agente inicia pelo `ai-memory run --no-autowire` (com `--fresh` numa conversa nova), que abre a sessão no workstream do projeto |

Cada extensão precisa estar instalada e no `PATH` (o instalador oferece instalar as duas); o terra em si não muda a configuração do Claude: ele só passa as extensões ativas para o agente, em cada mensagem (e para a Interface original do provedor).

A aba lista as extensões (ativa, inativa ou não instalada) e mostra, da escolhida com `↑`/`↓`, a versão, o link e o relatório: no RTK, os tokens economizados (do `rtk gain`), com o gráfico dos tokens economizados por dia; no ai-memory, as páginas, sessões e observações guardadas, o tamanho do banco e o estado dos embeddings e do LLM (do `ai-memory status`). O relatório é carregado ao abrir a aba e atualizado a cada minuto com ela aberta; `r` atualiza na hora.

### Aba Configurações

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
    "context": { "display": "hidden", "auto_compact": false, "limit": 200000, "compact_at": 75 },
    "usage": { "display": "hidden" }
  }
}
```

| Configuração | Valores |
| --- | --- |
| Provedor | Qual agente o chat usa e se está disponível (só leitura) |
| Conta | Default (a conta padrão do `claude`) ou uma das contas encontradas em `~/.claude` e `~/.claude-*`, como Pessoal e Trabalho, com o e-mail de cada uma |
| Modo inicial | Do provedor (o `defaultMode` do Claude), Agente, Yolo, Manual, Accept Edits ou Plan |
| Modelo | Padrão ou um modelo do provedor (Opus, Sonnet, Haiku), enviado com `--model` |
| Interface | **Agente Terraforme** (o chat e o input do terra, o padrão) ou **Original do provedor**: a aba Chat roda o `claude` interativo no workspace, dentro da tela do terra (abaixo da barra de abas, com o rodapé), aberto ao iniciar e com `Enter`, com a conta, o modelo, o esforço e o modo inicial das Configurações (o Yolo vira `bypassPermissions`). As teclas vão para o `claude` (inclusive `Tab`, `Esc` e `Ctrl+C`); `Ctrl+Q` volta às abas e `Enter` na aba Chat volta ao terminal. Trocar a Interface não fecha o `claude` se já foi enviada alguma mensagem: a sessão reaparece ao voltar |

Dentro da seção Agente, separada por uma linha, fica a seção **Contexto**, da barra de contexto do chat:

| Configuração | Valores |
| --- | --- |
| Display | **Oculto** (o padrão) ou **Visível**: mostra ou esconde a barra de contexto |
| Auto compactação | **Não** (o padrão) ou **Sim**: ao fim de uma resposta que deixou o contexto no ponto de compactar, o terra manda o `/compact` sozinho, no máximo uma vez por mensagem enviada (nunca depois de um `/compact`, nem quando o próprio agente já compactou a conversa naquela resposta) |
| Limite de tokens | Um limite fixo, de 50K a 1M (padrão **200K**), ou **Do modelo** (o limite que o agente informa, ou 200K até ele informar). No `config.json`, `"limit"` aceita qualquer número de tokens, e `null` é o do modelo |
| Compactar em | O ponto de compactar, de 50% a 95% do limite de tokens (padrão **75%**), com os tokens que ele representa (ex.: `75% · 150K tokens`). A barra vai até ele: fica laranja na metade, vermelha a partir de 75% dele e mostra **Compacte a conversa** ao chegar nele |

Embaixo do Contexto, também separada por uma linha, fica a seção **Consumo**, da barra de consumo do chat:

| Configuração | Valores |
| --- | --- |
| Display | **Oculto** (o padrão) ou **Visível**: esconde ou mostra a barra de consumo (a sessão de 5 horas). Oculta, o terra não busca o consumo para ela |

À direita da seção Agente (embaixo, em telas com pouca largura), a seção **Extensões** lista as extensões no estilo da lista de modelos do `/model` (`1. RTK - descrição`), com `[x]` nas ativas nesta conta e `[*]` nas de usar sempre, 5 por página (`‹ Página 1/1 ›`). `Shift+Tab` leva o foco do Agente para as Extensões e de volta; nelas, `↑`/`↓` escolhem, `←`/`→` trocam de página e `Espaço` (ou `Enter`) passa a extensão de inativa para **ativa nesta conta**, **usar sempre** e inativa de novo. Embaixo da lista ficam a versão, se está ativa e o link da extensão em foco. Uma extensão que não está instalada aparece como **indisponível** (`(indisponível)` na lista e `indisponível (não instalada)` embaixo) e não pode ser ativada. As extensões são da conta do agente (o provedor e a conta da seção Agente), valem em todos os workspaces e ficam no banco (`~/.terraforme/terra.sqlite`); as de usar sempre continuam ativas ao trocar de conta. Valem a partir da próxima mensagem. Na primeira vez de uma conta, ela começa com as que estavam em `extensions.enabled` no `config.json` do workspace (de antes da v0.8.28).

### Aba Orquestrador

A aba **Orquestrador**, entre Extensões e Configurações, mostra as configurações dos chats delegados (os sub agentes): o provedor, o modelo, o modo e o que fazer ao abrir e ao terminar.

```
 Orquestrador

 Configurações
   Provedor            Claude
   Modelo              Default (Opus - 5.5)
   Modo dos chats      Accept Edits
   Ao terminar         Fechar o chat
   Ao abrir            Em segundo plano
```

`↑`/`↓` passam pelas configurações; `←`/`→` (ou `Enter`) mudam a escolhida. No Provedor, `←`/`→` passam pelo do workspace e pelos provedores conhecidos. No Modelo, `Enter` abre o mesmo menu do `/model` (`↑`/`↓` escolhem o modelo, `←`/`→` o esforço, `Esc` fecha).

As configurações ficam no `~/.terraforme/config.json` (`{"orchestrator": {...}}`), valem para todos os workspaces e, por enquanto, só ficam salvas: o terra ainda não delega chats. A antiga chave `mcp` ainda é lida e sai ao salvar.

| Configuração | O que faz |
| --- | --- |
| Provedor | O provedor dos chats delegados: o do workspace ou outro dos conhecidos |
| Modelo | O modelo e o esforço dos chats delegados. `Enter` abre o mesmo menu do `/model`. **Default** (o padrão): no Claude, Sonnet com esforço médio; nos outros provedores, o modelo padrão |
| Modo dos chats | O modo com que começam os chats delegados. **Accept Edits** por padrão |
| Ao terminar | **Fechar o chat** (o padrão) ou **Manter aberto** |
| Ao abrir | **Em segundo plano** (o padrão) ou **Mostrar o chat** |

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

As extensões ativas (aba Configurações) são a exceção: os servidores MCP delas entram com `--mcp-config` (continuam só eles, pelo `--strict-mcp-config`) e os hooks delas, no `--settings`. Com uma extensão de hooks ativa, o `disableAllHooks` sai (ele desliga até os hooks do `--settings`), então os hooks do seu Claude Code também rodam.

O modo fica sincronizado com o agente, menos o Yolo, que é só do terra: nele o agente pede permissão e o terra aceita tudo. O `terra cli` começa no `permissions.defaultMode` configurado no Claude (settings local, do projeto ou do usuário), e a cada resposta confere o modo em que o agente diz estar; se for outro, o terminal passa a mostrar o modo real. Trocar com `Shift+Tab` durante uma resposta mostra `Modo: atual → próximo`, e o novo modo vale a partir da próxima mensagem. Com o agente ativo, o badge de modo fica sempre visível.

Fora do Yolo, quando o agente pede permissão para uma ferramenta, aparece acima do input uma caixa **Permissão** com a ferramenta e o que ela vai fazer (ex.: `Bash` e o comando), com os botões **Permitir** e **Negar**: `←`/`→` escolhem e `Enter` confirma; `s` permite direto, e `n` ou `Esc` negam. Enquanto a caixa está aberta, ela fica com as teclas (menos o `Tab`, que continua trocando de aba) e a atividade mostra `Aguardando permissão para Bash...`. Pedidos que chegam juntos entram numa fila e aparecem um de cada vez. Cada resposta fica no histórico (`Permitido: Bash (ls).` ou `Negado: ...`), e interromper a resposta descarta os pedidos abertos. Os pedidos chegam pelo `--permission-prompt-tool stdio`.

No modo **Agente** (`auto`), quem decide é o classificador do próprio Claude, que não pergunta: quando ele bloqueia uma ferramenta, o agente segue sem ela. O terra mostra cada bloqueio na mesma caixa, com o título **Bloqueado pelo modo Agente** e o motivo, e ela continua aberta depois do fim da resposta. Com **Permitir**, quando todos os pedidos estiverem respondidos, o terra manda uma mensagem nova na mesma sessão pedindo ao agente para fazer o que foi bloqueado, com só aquela chamada liberada no `--allowedTools`: o comando exato no `Bash`, o arquivo nas ferramentas de arquivo e o domínio no `WebFetch` (nas outras, a ferramenta). O histórico mostra `Fazendo o que foi permitido...`. Ferramentas negadas por uma regra `deny` dos settings continuam negadas, sem caixa.

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

As configurações de cada workspace ficam em `~/.terraforme/workspace/<nome>/config.json` e as da máquina (as do orquestrador), em `~/.terraforme/config.json` (veja a aba Configurações).

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
├── upgrade.rs            # terra upgrade: roda o instalador do GitHub ou do repositório
├── settings.rs           # configurações da aba Configurações, salvas no banco
├── files.rs              # índice dos arquivos do diretório, atualizado em tempo real
├── extension.rs          # extensões do agente (Strategy + Registry)
│   ├── contract.rs       # Extension (a estratégia), Contribution e Report
│   ├── registry.rs       # Registry: as extensões e o que as ativas acrescentam
│   └── extensions.rs     # registro das extensões conhecidas
│       ├── rtk.rs        # RTK: hook do Bash e o rtk gain
│       └── ai_memory.rs  # ai-memory: MCP, hooks e o ai-memory status
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
    ├── config.rs         # aba Configurações: seções Agente, Consumo, Extensões
    ├── orchestrator.rs   # aba Orquestrador: as configurações dos chats delegados
    ├── extensions.rs     # aba Extensões: relatório de cada extensão
    ├── widgets.rs → widgets/badge.rs, widgets/popup.rs
    └── chat.rs           # aba Chat: liga o adapter, histórico, input e badges
        ├── mode.rs       # como status e modo aparecem
        ├── input.rs      # caixa de input: edição, cursor, seleção, Insert
        │   └── tests.rs
        ├── mention.rs    # menções de arquivo com @: lista, filtro e badge
        ├── commands.rs   # comandos com /: lista do agente e interfaces do terra
        ├── model_menu.rs # interface do /model: modelo e esforço
        ├── permission.rs # pedidos de permissão do agente: Permitir ou Negar
        ├── history.rs    # histórico: quebra de linha, rolagem, seleção
        └── tests.rs      # testes do Chat (com um adapter de mentira)
```

| Módulo | Responsabilidade |
| --- | --- |
| `src/cli.rs` | Argumentos (clap) e execução dos comandos |
| `src/upgrade.rs` | `terra upgrade`: roda o instalador do GitHub ou, com `--repo`, o do repositório |
| `src/db.rs` | SQLite, canonicalização de paths e regras de nome |
| `src/agent/` | Agente do chat: eventos e integração com o Claude CLI |
| `src/extension/` | Extensões do agente: o contrato (Strategy), o Registry e as extensões |
| `src/platform/` | Área de transferência e estado do teclado |
| `src/tui/` | Telas ratatui: tela principal, chat, consumo, extensões, configurações, orquestrador e workspaces |
| `src/theme.rs` | Cores do Catppuccin; troque `FLAVOR` para mudar de variante |

### Adicionar um provedor (route) ao chat

O chat usa o padrão **Adapter**: ele só conhece o trait `ChatAdapter` (`src/agent/adapter.rs`), e cada provedor o implementa traduzindo o seu protocolo. Para adicionar um:

1. Crie `src/agent/adapters/<provedor>.rs` com uma struct que implemente `ChatAdapter`:
   - `name()`: o nome no botão do título (ex.: `"Claude"`);
   - `is_available()`: se o provedor está instalado ou configurado (sem ele, o chat fica Desconectado com o `unavailable_hint()`);
   - `start(Request)`: começa uma resposta e devolve uma `Session`, cujo `poll()` entrega os `AgentEvent`s até `Done` ou `Failed`. No `PermissionMode::Yolo`, aprove tudo o que o agente pedir; nos outros modos, mande cada pedido de permissão como `AgentEvent::PermissionRequest` e aplique a resposta que o chat der em `Session::respond`;
   - opcionais: `configured_mode()` (modo inicial vindo da configuração do provedor), `model_label()` (nome amigável do modelo), `load_usage()` e `load_daily()` (o consumo e os tokens por dia da aba Consumo, em segundo plano) e `login()` (o comando interativo do `/login`).
2. Declare o módulo e inclua-o na lista de `all()` em `src/agent/adapters.rs`.

Nada na interface muda: o histórico, os modos, o consumo e a sincronia do modo funcionam com qualquer adapter. O `src/tui/chat/tests.rs` tem um `FakeAdapter` que serve de exemplo mínimo.
| `install.sh` / `install.ps1` | Instalação direto do GitHub (`curl ... \| bash` e `irm ... \| iex`); o `terra upgrade` roda os mesmos. Rodado de um clone, o `install.ps1` chama o `install-win.sh` dele |
| `install-powershell.ps1` | Põe o `terra` no `PATH` do usuário e nos perfis do PowerShell (chamado pelo `install-win.sh`) |
| `.github/workflows/release.yml` | Compila o `terra` para Linux e Windows a cada tag e anexa os binários à release |
| `install-ui.sh` | Interface dos instaladores (banner, progresso e confirmação) |
