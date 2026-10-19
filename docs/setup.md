# Módulo de setup

O setup é o passo a passo que deixa a máquina pronta para usar o terraforme: detecta os agentes instalados, instala as extensões que faltam (RTK, AI Memory, AI Jail), configura cada uma nos agentes e instala as skills embutidas no binário. A lógica vive no `terraforme-core`; a CLI (`crates/terraforme-cli/src/main.rs`) é a casca que escolhe o fluxo, imprime o progresso e conta falhas.

## Pontos de entrada

| Comando | Fluxo (em `main.rs`) | O que faz |
| --- | --- | --- |
| `terraforme setup` | `setup_flow()` | Não interativo, em 3 passos: agentes → extensões → skills. Instala o que falta. |
| `terraforme extension --setup` | `setup_interactive()` | Interativo: seleciona agentes e extensões, executa só o que foi escolhido. Exige terminal interativo (`io::stdin().is_terminal()`). |
| `terraforme extension <ext> --setup` | `Extension::setup()` direto | Refaz a configuração de uma única extensão. |
| `terraforme extension ai-jail --project` | `jail_project()` → `jail::init_project` | Cria o `.ai-jail` do projeto (liga a sandbox só nele). |

O `Extension::setup()` também roda ao final de toda `Extension::install()` — instalar já deixa configurado.

## `terraforme setup` (`setup_flow`)

1. **Agentes:**
   Lista os `Provider::ALL` com adapter implementado (Claude Code e OpenCode; Cursor ainda não é) e marca os que estão no `PATH` com versão e caminho. Nenhum agente encontrado aborta com a orientação para instalar um dos que aparecem na lista. Os que estão são coletados em `agents` e usados nos próximos passos.
2. **Extensões:**
   Instala (`Extension::install`) cada `Extension::ALL` que ainda não está — extensão já presente é pulada. Com tudo instalado chama `setup_selected(&agents, &installed)` para configurar.
3. **Skills:**
   `install_skills(&agents)` copia as skills embutidas pelo `build.rs` (`skill::names()`) para a pasta global de skills de cada agente (`Adapter::skills_dir`). Skill copiada sobrescreve a anterior.

Falhas são impressas assim que acontecem e contadas; ao final, um contador maior que zero termina com erro ("N falha(s)…; veja os erros acima") em vez de abortar no meio. Com sucesso: "✔ Setup concluído".

## `terraforme extension --setup` (`setup_interactive`)

1. Detecta os agentes do `PATH`. Nenhum: avisa e segue com uma lista de agentes vazia.
2. Seleção de agentes (múltipla, `ask_selection`): números separados por vírgula, ou faixas como `1-3`; `q` (ou "cancelar") cancela sem mudar nada, Enter/"nenhum" deixa a lista vazia. Não mexe no agente padrão do `config.toml`.
3. Seleção de extensões: sem agentes selecionados, a única opção é o AI Jail (que é configuração da máquina); com agentes, as três aparecem.
4. `setup_selected(&selected_agents, &extensions)` — nada é executado antes das duas escolhas; cancelar antes não altera configuração.

## Ordem de execução e falhas (`setup_selected`)

Itera extensão por extensão e, dentro de cada uma, agente por agente — uma falha não esconde o resultado das outras:

- **AI Jail:** chamada global única (`Extension::setup()` → `jail::setup`), independentemente dos agentes.
- **RTK / AI Memory:** `Extension::setup_for(provider)` por agente selecionado.

Cada etapa imprime `✔ … configurado` (com avisos `⚠` e informações `ℹ` de `SetupMessages`) ou `✘` + erro. O total de falhas decide o resultado: zero → `Ok(())`, maior → erro com a contagem.

## O que cada configuração faz

### `Extension::setup()` (global) e `setup_for(provider)` (por agente) — `extension.rs`

Ambas exigem a extensão instalada (`require_installed`: não instalada → orienta `terraforme extension <ext> --install`; qualquer conflito → erro).

| Extensão | `setup()` global | `setup_for` Claude | `setup_for` OpenCode |
| --- | --- | --- | --- |
| RTK | `rtk init -g` | `rtk init -g` + adapter `setup(eco)` | nada (o plugin é do terraforme) + adapter `setup(eco)` |
| AI Memory | `ai-memory setup` | `ai-memory setup` + adapter `setup(high)` | `ai-memory install-hooks --agent opencode2 --apply` + adapter `setup(high)` |
| AI Jail | [`jail::setup`](#ai-jail-jailrs) | erro ("configurado uma vez para a máquina") | idem |

O AI Jail rejeita `setup_for` de propósito (`bail`); o Cursor é rejeitado em qualquer configuração por agente ("integração ainda não implementada").

### AI Jail (`jail::setup`, `jail.rs`)

`jail::setup()` verifica e completa, devolvendo uma linha por item verificado:

1. `require()`: ai-jail instalado (senão aponta `extension ai-jail --install` ou `--no-jail`), versão ≥ 2.9 (primeira com `--forward-port`) e, no Linux, o `bwrap` (pacote bubblewrap).
2. `ensure_default_config()`: se o `~/.ai-jail` não existe, cria-o com `default_config()` — `no_save_config = true`, `no_status_bar = true`, comentários de rede/SSH e a seção de cada agente implementado que tem integração (`[commands.claude]`/`[commands.opencode]` com `agent_state = true`, e no OpenCode os `allow_hosts` padrão). O arquivo não é sobrescrito se já existir.
3. Se ele já existe: `missing_sections()` faz parse TOML e aponta os agentes que estão sem a própria `[commands.<bin>]`. As seções que faltam são **acrescentadas** ao final do arquivo (o resto do TOML do usuário é preservado); TOML inválido aborta com erro.

Configuração por projeto: `jail::init_project(dir, agent)` roda `ai-jail --init <agent>` dentro de `dir` e exige que o `.ai-jail` apareça lá; com esse arquivo o `terraforme cli` aberto na pasta roda no jail mesmo com `jail = false` no config.

### Adapter `setup(mode)` — seleção de agente

O `setup_for` fecha a parte da extensão e chama `provider.adapter().setup(mode)` (RTK → `Mode::Eco`, AI Memory → `Mode::High`) para remontar a integração do agente para aquele modo. As regras por adapter são a referência final; o resumo:

- **Claude (`agent/claude.rs`):** migrate os hooks globais do `~/.claude/settings.json` (backup `settings.json.terraforme-bak`) para os plugins locais em `~/.config/terraforme/plugins/terraforme-<ext>/`; escreve `.mcp.json` do AI Memory (servidor `memory`) copiando o que o `ai-memory setup` registrou no `~/.claude.json` — sem esse servidor, é aviso para rodar `ai-memory setup`. O agente recebe `--plugin-dir <plugin>` por extensão e `--settings` bloqueando o MCP global do AI Memory (que no modo `none`/eco valeria demais).
- **OpenCode (`agent/opencode.rs`):** exige OpenCode 2 (`require_v2`), move plugins globais soltos para a raiz dos plugins, gera o plugin do AI Memory quando necessário (ferramenta caseira; o plugin do RTK é escrito pelo terraforme) e devolve o conteúdo do config (plugins + MCP bridge do AI Memory) para ser injetado por variável de ambiente na execução — nada é escrito globalmente.

Os `notices`/`warnings` desses adapters viram `SetupMessages` (o `ℹ`/`⚠` por etapa).

## Onde cada arquivo é mexido

| O que | Quem cria | Quem remove ou reconfigura |
| --- | --- | --- |
| `rtk init -g` | `Extension::setup`/`setup_for` (Claude) | `rtk init -g --uninstall` no `delete` |
| Serviço systemd do AI Memory | `ai-memory setup` | `stop_ai_memory_service` no `delete` |
| Plugins locais (hooks/config) | `Adapter::setup` a cada execução | `Adapter::forget(ext)` no `delete` de cada provider |
| `~/.ai-jail` | `jail::setup` (uma vez) | nunca removido pelo `delete` (dados preservados) |
| `.ai-jail` do projeto | `ai-jail --init` (`extension ai-jail --project`) | manual |
| Skills globais dos agentes | `terraforme setup` (passo 3) | sobrescritas a cada setup |

Os mapeamentos de extensões para o Jail (plugins, binários, pastas de dados, portas do servidor local do AI Memory) ficam **fora** do `~/.ai-jail`: são passados por linha de comando a cada execução, em `jail::wrap`.
