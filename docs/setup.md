# Setup do terraforme

O setup configura a máquina para usar o terraforme: detecta os agentes instalados, instala as
extensões que faltam (RTK, AI Memory, AI Jail) e as integra a cada agente, e instala as skills
embutidas no binário.

## Caminho rápido

```bash
terraforme setup
```

Executa tudo sem perguntas, em três passos:

1. **Agentes** — lista Claude Code e OpenCode; interrompe se nenhum estiver no `PATH`.
2. **Extensões** — instala RTK, AI Memory e AI Jail se ainda não estiverem; configura cada uma nos
   agentes encontrados. Extensão já instalada é pulada.
3. **Skills** — copia as skills embutidas no binário para a pasta global de skills de cada agente
   (`~/.claude/skills`, `~/.config/opencode/skills`). Sobrescreve versões anteriores.

Falhas são impressas imediatamente e contadas; o restante do setup continua. Ao final: **✔ Setup
concluído** (todas as etapas foram bem) ou um erro indicando quantas etapas falharam.

## Setup interativo

```bash
terraforme extension --setup
```

Mesma lógica, com seleção manual de agentes e extensões antes de qualquer alteração. Exige
terminal interativo.

**Fluxo:**

1. Detecta os agentes no `PATH` e exibe os encontrados numerados. Nenhum agente: avisa e continua
   (ainda é possível configurar o AI Jail).
2. Pergunta quais agentes configurar (números separados por vírgula; Enter = nenhum; `q` = cancelar
   sem alterar nada).
3. Pergunta quais extensões configurar (RTK, AI Memory e/ou AI Jail). Sem agentes selecionados,
   só o AI Jail aparece — ele é configuração da máquina e não precisa de agente.
4. Executa as configurações selecionadas e informa o resultado por extensão e agente.

Cancelar antes da execução não altera nenhuma configuração.

## Reconfigurar uma extensão

```bash
terraforme extension rtk --setup       # refaz só o RTK em todos os agentes
terraforme extension ai-memory --setup # refaz só o AI Memory
terraforme extension ai-jail --setup   # reverifica requisitos e completa o ~/.ai-jail
```

Útil quando um agente foi atualizado, quando a integração parou de funcionar ou quando um novo
agente foi instalado depois do setup inicial.

## Configurar o AI Jail em um projeto

```bash
terraforme extension ai-jail --project
```

Cria o `.ai-jail` na pasta atual (`ai-jail --init <agente>`). Com esse arquivo, o `terraforme cli`
aberto nessa pasta (ou em qualquer subpasta) roda no sandbox automaticamente, mesmo com
`jail = false` no config global. Não é criado em `$HOME`: lá o `~/.ai-jail` é a configuração
global.

## O que cada configuração faz

### RTK

- **Global:** executa `rtk init -g` para registrar os hooks do RTK.
- **Claude Code:** move os hooks do `~/.claude/settings.json` (backup: `settings.json.terraforme-bak`)
  para o plugin local em `~/.config/terraforme/plugins/terraforme-rtk/`.
- **OpenCode:** escreve o plugin do RTK (no formato do OpenCode 2) no diretório de plugins do
  terraforme; não usa o plugin legado gerado por `rtk init --opencode`.

### AI Memory

- **Global:** executa `ai-memory setup` para preparar o serviço local (systemd no Linux).
- **Claude Code:** chama `ai-memory setup` e migra os hooks para o plugin local em
  `~/.config/terraforme/plugins/terraforme-ai-memory/`. O servidor MCP do AI Memory é copiado do
  `~/.claude.json` para o `.mcp.json` do plugin (servidor `memory`); sem esse servidor, um aviso
  orienta a rodar `ai-memory setup` manualmente.
- **OpenCode:** executa `ai-memory install-hooks --agent opencode2 --apply` e gera o plugin
  compatível com OpenCode 2.

### AI Jail

Configurado uma vez para a máquina, independentemente dos agentes:

1. Verifica: ai-jail instalado, versão ≥ 2.9 e, no Linux, o `bwrap` (pacote `bubblewrap`).
2. Se `~/.ai-jail` não existe: cria com `no_save_config = true`, `no_status_bar = true` e as
   seções `[commands.claude]` / `[commands.opencode]`.
3. Se já existe: completa as seções de agentes ausentes sem tocar no restante do arquivo.

Cada agente que está no PATH tem a sua seção `[commands.<bin>]` completada. TOML inválido no arquivo
existente interrompe com erro.

## Skills embutidas

O binário traz skills que são copiadas para a pasta global de skills de cada agente detectado:

| Agente | Pasta de destino |
| --- | --- |
| Claude Code | `~/.claude/skills/<skill>/` |
| OpenCode | `~/.config/opencode/skills/<skill>/` |

Skills copiadas sobrescrevem versões anteriores. Para ver quais skills estão embutidas, basta
rodar `terraforme setup` — elas aparecem listadas no Passo 3.

## Resumo dos arquivos alterados

| Arquivo / recurso | Criado por | Removido ou refeito por |
| --- | --- | --- |
| Configuração global do RTK (`rtk init -g`) | `extension rtk --setup` | `rtk init -g --uninstall` (via `extension rtk --delete`) |
| Serviço systemd do AI Memory | `ai-memory setup` | `extension ai-memory --delete` |
| Plugins locais do terraforme (`~/.config/terraforme/plugins/`) | `Adapter::setup` a cada setup | `extension <ext> --delete` (por provider) |
| `~/.ai-jail` | `extension ai-jail --setup` (uma vez) | nunca removido (dados preservados no `--delete`) |
| `.ai-jail` do projeto | `extension ai-jail --project` | manual |
| Skills globais dos agentes | `terraforme setup` (passo 3) | sobrescritas a cada setup |
| `~/.claude/settings.json.terraforme-bak` | primeira migração de hooks | permanece como backup |

Os mapeamentos de extensões passados ao AI Jail (plugins, binários, pastas de dados, portas do
servidor do AI Memory) **não ficam** no `~/.ai-jail`: são passados por linha de comando a cada
execução, em `jail::wrap`.

---

## Referência de implementação

> Esta seção é para contribuidores. Os pontos de entrada no código:

| Comando | Função em `main.rs` | Descrição |
| --- | --- | --- |
| `terraforme setup` | `setup_flow()` | Não interativo; 3 passos sequenciais. |
| `terraforme extension --setup` | `setup_interactive()` | Interativo; requer `stdin().is_terminal()`. |
| `terraforme extension <ext> --setup` | `Extension::setup()` direto | Reconfigura uma extensão. |
| `terraforme extension ai-jail --project` | `jail_project()` → `jail::init_project` | `.ai-jail` do projeto. |

`Extension::setup()` também é chamado ao final de `Extension::install()` — instalar já configura.

`setup_selected(agents, extensions)` itera extensão por extensão e, dentro de cada uma, agente por
agente: AI Jail chama `Extension::setup()` uma vez; RTK e AI Memory chamam
`Extension::setup_for(provider)` para cada agente. Falhas são contadas, não interrompem o loop.
Cada `setup_for` fecha com `provider.adapter().setup(mode)` para remontar a integração.

Detalhes do adapter do Claude: `crates/terraforme-core/src/agent/claude.rs`.
Detalhes do adapter do OpenCode: `crates/terraforme-core/src/agent/opencode.rs`.
Detalhes do AI Jail: `crates/terraforme-core/src/jail.rs` (`jail::setup`, `jail::init_project`).
