# terraforme

**O framework de desenvolvimento com agentes de IA.** Um único comando prepara o seu agente (Claude
Code, OpenCode ou Antigravity) com economia de tokens, memória entre sessões e uma sandbox de
segurança, e o abre num terminal que mostra em tempo real quanto você está consumindo.

## O que é

O terraforme é uma CLI em Rust, para Linux e Windows, que faz três coisas:

1. **Instala e configura extensões abertas para agentes de IA**: [RTK](https://github.com/rtk-ai/rtk)
   (economia de tokens), [AI Memory](https://github.com/akitaonrails/ai-memory) (memória entre
   sessões) e [AI Jail](https://github.com/akitaonrails/ai-jail) (sandbox do sistema).
2. **Abre o agente no modo que você escolher**: liga só as extensões daquele modo, sem você editar
   a configuração do agente. Dá para rodar com RTK hoje, com AI Memory amanhã, ou sem nada.
3. **Envolve o agente num terminal com abas**: várias instâncias lado a lado, consumo de tokens e
   limites do plano, relatórios das extensões e configuração, tudo sem sair do terminal.

### Por que usar

Agentes de IA são poderosos, mas, usados "crus", gastam tokens demais, esquecem tudo a cada sessão
e rodam com acesso livre à sua máquina. O terraforme junta as melhores extensões abertas num só
fluxo:

- **Menos custo**: com o RTK, as saídas de comandos chegam enxutas ao agente, economizando até 90%
  dos tokens dessas saídas.
- **Memória de longo prazo**: com o AI Memory, o agente lembra decisões, regras e contexto do
  projeto de uma sessão para outra.
- **Segurança**: com o AI Jail, o agente roda numa sandbox e só altera a pasta do projeto (só no
  Linux).
- **Visibilidade**: limites do plano, tokens por sessão, por projeto e por subagente, em abas ao
  lado do agente.
- **Produtividade**: até 9 instâncias do agente ao mesmo tempo, pastas de trabalho salvas com um
  nome (*spaces*) e uma UI web para ver e mandar mensagens às instâncias de todos os spaces.

### Modos

| Modo | O que liga | Para quê |
| --- | --- | --- |
| `none` | nada | o agente puro |
| `eco` | RTK | sessões longas gastando menos |
| `high` | AI Memory | projetos em que o contexto acumulado importa |
| `hybrid` | RTK + AI Memory | o melhor dos dois |
| `--jail` | AI Jail (junto com qualquer modo) | rodar o agente isolado |

## Instalação

### Binários prontos (sem Rust)

Baixa a [última release](https://github.com/Augustowtg/terraforme/releases/latest):

```bash
# Linux (x86_64 e ARM64): instala em ~/.local/bin
curl -fsSL https://raw.githubusercontent.com/Augustowtg/terraforme/main/install.sh | sh
```

```powershell
# Windows (PowerShell): instala em %LOCALAPPDATA%\Programs\terraforme\bin e põe no PATH
irm https://raw.githubusercontent.com/Augustowtg/terraforme/main/install.ps1 | iex
```

### Compilando do código

Pré-requisitos: `git` e Rust ([rustup](https://rustup.rs); a versão certa do toolchain é baixada
sozinha). No Windows, também o "Desktop development with C++" do Visual Studio Build Tools. Para o
AI Jail no Linux, instale o `bwrap` (pacote `bubblewrap`).

```bash
# Linux: compila e instala em ~/.local/bin
git clone https://github.com/Augustowtg/terraforme.git
cd terraforme
./install.linux.sh
```

```powershell
# Windows: compila, instala em %LOCALAPPDATA%\Programs\terraforme\bin e põe no PATH do usuário
git clone https://github.com/Augustowtg/terraforme.git
cd terraforme
.\install.windows.ps1    # se a execução de scripts estiver bloqueada:
                         # powershell -ExecutionPolicy Bypass -File .\install.windows.ps1
```

Confira com `terraforme --version`. Se o comando não for encontrado, abra um novo terminal ou
adicione a pasta de instalação ao `PATH`.

### Atualizar e remover

```bash
terraforme upgrade              # atualiza para a última release do GitHub
terraforme upgrade --dev        # pelo repositório: git pull + compila + reinstala
terraforme upgrade --restart    # … e reinicia o terraforme no fim sem perguntar
terraforme restart              # reinicia o daemon MCP e a janela flutuante com os binários novos
./install.linux.sh --uninstall  # remove (Windows: .\install.windows.ps1 -Uninstall)
```

## Como usar

### 1. Prepare a máquina

```bash
terraforme setup
```

Identifica os agentes instalados (e oferece instalar um, se não houver nenhum), instala e configura
as extensões neles e instala as skills embutidas. Para ver o que ficou instalado:
`terraforme status`.

### 2. Escolha o padrão

```bash
terraforme config                   # mostra a configuração atual
terraforme config --cli             # preset de início: eco, high, security (AI Jail) ou custom
terraforme config --mode hybrid     # ou defina o modo diretamente
terraforme config --agent opencode  # agente padrão
```

### 3. Abra o agente

```bash
terraforme cli                    # abre com as preferências salvas
terraforme cli --hybrid --jail    # RTK + AI Memory, dentro do AI Jail
terraforme cli -n 3               # três instâncias, cada uma numa sub-aba
terraforme cli --agent opencode   # usa o OpenCode só nesta sessão
terraforme cli -- --continue      # repassa --continue ao agente
terraforme cli --plain            # abre o agente direto, sem o terminal do terraforme
```

No terminal do terraforme, as abas ficam no topo: o **agente** (uma sub-aba por instância),
**Consumo** (limites, tokens, subagentes e economia do RTK), **Extensões** (relatórios do AI Memory
e do AI Jail) e **Config**. Atalhos principais:

| Tecla | Ação |
| --- | --- |
| `Ctrl+Shift+F1`…`F4` | abre a aba Agente, Consumo, Extensões, Config |
| `Ctrl+Alt+N` / `Ctrl+Alt+W` | abre / fecha uma instância do agente |
| `Ctrl+Alt+1`…`9` (ou `Alt+1`…`9`) | vai para a sub-aba (instância) |
| `Ctrl+Alt+C` | copia a seleção ou a tela |
| `Ctrl+Alt+K` | lista todos os atalhos |
| `F10` | sai |

### 4. Salve as pastas como spaces

```bash
terraforme space api --save              # salva a pasta atual como "api"
terraforme space api                     # abre um shell na pasta do space
terraforme space                         # lista e pergunta qual abrir
terraforme space api --mode eco --jail true   # preferências do terraforme cli neste space
```

Dentro de um space, o `terraforme cli` usa as preferências dele (agente, modo, AI Jail, conta do
Claude Code) no lugar do `terraforme config`.

### 5. Gerencie as extensões

```bash
terraforme extension --status          # status e versão de todas
terraforme extension rtk --install     # instala uma
terraforme extension ai-memory --setup # refaz a configuração nos agentes
terraforme extension rtk --update      # atualiza
terraforme extension rtk --delete      # remove a extensão e a configuração dela
```

### UI web

```bash
terraforme config --mcp-ui true   # liga; a URL aparece em `terraforme config`
```

Mostra as instâncias abertas em todos os spaces e as mensagens entre elas, e deixa você mandar uma
mensagem para qualquer instância (ela chega como um prompt).

## Documentação

- [Guia de uso](docs/guia.md): todos os comandos, o terminal com abas e os atalhos.
- [Arquitetura](docs/arquitetura.md): como o terraforme funciona por dentro.
- [Desenvolvimento](docs/desenvolvimento.md): compilar, testar e contribuir.
- [Changelog](CHANGELOG.md)
