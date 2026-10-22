# Setup do terraforme

Referência dos comandos que preparam a máquina (`terraforme setup` e `terraforme extension
--setup/--project`) e do que cada configuração altera. O passo a passo da seleção manual está em
[Setup interativo](setup-interativo.md); o uso geral, no [Guia](guia.md).

## Qual comando usar

| Comando | Quando usar |
| --- | --- |
| `terraforme setup` | primeira vez: detecta (ou instala) os agentes, instala e configura as extensões e copia as skills, sem perguntas |
| `terraforme extension --setup` | escolher à mão quais agentes e extensões configurar ([Setup interativo](setup-interativo.md)) |
| `terraforme extension <ext> --setup` | refazer a configuração global de uma extensão |
| `terraforme extension ai-jail --project` | ligar o AI Jail numa pasta de projeto |

## `terraforme setup`

Executa em três passos:

1. **Agentes** — lista os agentes implementados (Claude Code, OpenCode, Antigravity) com versão e
   caminho. Sem nenhum no `PATH`, mostra uma lista para marcar quais instalar (`↑`/`↓` movem,
   `Espaço` marca, `Enter` confirma) e roda o instalador oficial de cada um:
   - Claude Code: `claude.ai/install.sh` (`install.ps1` no Windows);
   - OpenCode: `opencode.ai/install` (`npm install -g opencode-ai` no Windows);
   - Antigravity: sem instalador automático, instale pelo site.

   Sem terminal interativo, ou se nenhum agente ficar disponível, o setup para com erro.
2. **Extensões** — instala RTK, AI Memory e AI Jail se ainda não estiverem (as já instaladas são
   puladas) e configura cada extensão instalada em cada agente encontrado, como em
   [`setup_selected`](#referência-de-implementação).
3. **Skills** — copia as skills embutidas no binário para a pasta global de skills de cada agente.

Falhas são impressas na hora e contadas; o restante continua. No fim: **✔ Setup concluído** ou um
erro com o número de etapas que falharam.

## Setup interativo

```bash
terraforme extension --setup
```

Mesma configuração do passo 2, mas você escolhe os agentes e as extensões antes de qualquer
alteração e nenhum binário é instalado. Fluxo e regras em [Setup interativo](setup-interativo.md).

## Reconfigurar uma extensão

```bash
terraforme extension rtk --setup        # rtk init -g
terraforme extension ai-memory --setup  # ai-memory setup
terraforme extension ai-jail --setup    # confere versão e bwrap e completa o ~/.ai-jail
```

- Roda só o comando global da extensão, sem perguntar e sem passar por cada agente.
- Os plugins do terraforme para cada agente são remontados a cada `terraforme cli`, então não
  precisam ser refeitos aqui.
- Para refazer a integração por agente (ex.: o `install-hooks` do AI Memory no OpenCode), use
  `terraforme extension --setup` ou `terraforme setup`.

## AI Jail num projeto

```bash
terraforme extension ai-jail --project
```

- Cria (ou atualiza) o `.ai-jail` na pasta atual com `ai-jail --init <agente>`, usando o agente
  padrão do `config.toml`.
- Com ele, o `terraforme cli` aberto nessa pasta roda no sandbox mesmo com `jail = false` no
  config (`--no-jail` desliga numa sessão).
- As regras do projeto (pastas, hosts liberados) vão nesse arquivo.
- O `terraforme cli` nunca cria esse arquivo sozinho.

Para ligar o AI Jail em todas as pastas: `terraforme extension ai-jail --default true` (o mesmo que
`terraforme config --jail true`).

## O que cada configuração faz

### RTK

| Alvo | O que acontece |
| --- | --- |
| Global | `rtk init -g` registra os hooks do RTK. |
| Claude Code | os hooks saem do `~/.claude/settings.json` (backup em `settings.json.terraforme-bak`) e vão para o plugin `~/.config/terraforme/plugins/terraforme-rtk/`. |
| OpenCode | o terraforme escreve o próprio plugin do RTK no formato do OpenCode 2; o plugin legado de `rtk init --opencode` (formato do OpenCode 1) não é usado. |
| Antigravity | ainda não implementado (a etapa falha). |

### AI Memory

| Alvo | O que acontece |
| --- | --- |
| Global | `ai-memory setup` prepara o serviço local (systemd no Linux). |
| Claude Code | `ai-memory setup` e migração dos hooks para `~/.config/terraforme/plugins/terraforme-ai-memory/`. O servidor MCP do AI Memory é copiado do `~/.claude.json` para o `.mcp.json` do plugin (servidor `memory`); sem esse servidor, um aviso pede para rodar `ai-memory setup`. |
| OpenCode | `ai-memory install-hooks --agent opencode2 --apply` e o plugin compatível com o OpenCode 2. |
| Antigravity | ainda não implementado (a etapa falha). |

### AI Jail

Configurado uma vez para a máquina, independentemente dos agentes escolhidos:

1. Confere o ai-jail instalado, versão ≥ 2.9 e, no Linux, o `bwrap` (pacote `bubblewrap`).
2. Sem `~/.ai-jail`: cria o arquivo com `no_save_config = true`, `no_status_bar = true` e as seções
   `[commands.claude]` e `[commands.opencode]`.
3. Com `~/.ai-jail`: acrescenta só as seções de agente que faltam, sem tocar no resto. TOML
   inválido interrompe com erro.

As seções são as de todos os agentes que o terraforme conhece para o AI Jail (Claude Code e
OpenCode), estejam ou não no `PATH`.

Os mapeamentos das extensões (plugins, binários, pastas de dados, porta do servidor do AI Memory)
**não** ficam no `~/.ai-jail`: são passados por linha de comando a cada execução. Veja
[Arquitetura](arquitetura.md#ai-jail).

## Skills embutidas

| Agente | Pasta de destino |
| --- | --- |
| Claude Code | `~/.claude/skills/<skill>/` (respeita `CLAUDE_CONFIG_DIR`) |
| OpenCode | `~/.config/opencode/skills/<skill>/` |

- Copiadas pelo passo 3 do `terraforme setup`, que lista cada skill instalada.
- Sobrescrevem versões anteriores.
- Agentes sem pasta de skills conhecida (Antigravity) são pulados com um aviso.

## Arquivos alterados

| Arquivo / recurso | Criado por | Removido ou refeito por |
| --- | --- | --- |
| Configuração global do RTK | `rtk init -g` (install/setup) | `rtk init -g --uninstall` (via `extension rtk --delete`) |
| Serviço do AI Memory | `ai-memory setup` (install/setup) | `ai-memory uninstall` (via `extension ai-memory --delete`) |
| Plugins do terraforme (`~/.config/terraforme/plugins/`) | setup por agente e cada `terraforme cli` | `extension <ext> --delete` |
| `~/.ai-jail` | install ou `extension ai-jail --setup` | nunca removido (preservado no `--delete`) |
| `.ai-jail` do projeto | `extension ai-jail --project` | manual |
| Skills globais dos agentes | `terraforme setup` (passo 3) | sobrescritas a cada setup |
| `~/.claude/settings.json.terraforme-bak` | primeira migração de hooks | permanece como backup |

## Referência de implementação

Para contribuidores; pontos de entrada em `crates/terraforme-cli/src/main.rs`:

| Comando | Função | Descrição |
| --- | --- | --- |
| `terraforme setup` | `setup_flow()` | 3 passos; `install_agents()` quando não há agente; `install_skills()` no fim. |
| `terraforme extension --setup` | `setup_interactive()` | exige `stdin().is_terminal()`; chama `setup_selected()`. |
| `terraforme extension <ext> --setup` | `Extension::setup()` | só o comando global da extensão. |
| `terraforme extension ai-jail --project` | `jail_project()` → `jail::init_project` | `.ai-jail` do projeto. |

- `Extension::install()` termina chamando `Extension::setup()`: instalar já configura.
- `setup_selected(agents, extensions)` percorre extensão por extensão: AI Jail chama
  `Extension::setup()` uma vez; RTK e AI Memory chamam `Extension::setup_for(provider)` para cada
  agente, que roda o comando do agente e fecha com `provider.adapter().setup(mode)`. Falhas são
  contadas, não interrompem o loop.
- Adapters: `crates/terraforme-core/src/agent/{claude,opencode,antigravity}.rs`; AI Jail:
  `crates/terraforme-core/src/jail.rs` (`jail::setup`, `jail::init_project`).
