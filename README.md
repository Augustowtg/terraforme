# terraforme

**O framework de desenvolvimento com agentes de IA.** Um único comando prepara o seu agente (hoje o
Claude Code) com economia de tokens, memória entre sessões e uma sandbox de segurança, e o abre num
terminal que mostra em tempo real quanto você está consumindo.

## Por que usar

Agentes de IA são poderosos, mas, usados "crus", gastam tokens demais, esquecem tudo a cada sessão
e rodam com acesso livre à sua máquina. O terraforme resolve isso juntando as melhores extensões
abertas num só fluxo:

- **Menos custo** — com o [RTK](https://github.com/rtk-ai/rtk), as saídas de comandos chegam ao
  agente enxutas, economizando até 90% dos tokens dessas saídas.
- **Memória de longo prazo** — com o [AI Memory](https://github.com/akitaonrails/ai-memory), o agente
  lembra decisões, regras e contexto do projeto de uma sessão para outra.
- **Segurança** — com o [AI Jail](https://github.com/akitaonrails/ai-jail), o agente roda numa
  sandbox do sistema e só altera a pasta do projeto.
- **Visibilidade** — limites do plano, tokens por sessão, por projeto e por subagente, tudo em abas
  ao lado do agente.
- **Produtividade** — até 9 instâncias do agente lado a lado e pastas de trabalho salvas com um nome
  (*spaces*).

Você escolhe o modo de cada sessão sem mexer em configuração nenhuma:

| Modo | O que liga | Para quê |
| --- | --- | --- |
| `--eco` | RTK | sessões longas gastando menos |
| `--high` | AI Memory | projetos em que o contexto acumulado importa |
| `--hybrid` | RTK + AI Memory | o melhor dos dois |
| `--jail` | AI Jail (com qualquer modo) | rodar o agente isolado |

## Instalação

Pré-requisitos: Linux, `git` e Rust ([rustup](https://rustup.rs) — a versão certa é baixada
sozinha). Para usar o AI Jail, instale também o `bwrap` (pacote `bubblewrap`).

```bash
git clone git@github.com:Augustowtg/terraforme.git
cd terraforme
./install.linux.sh            # compila e instala em ~/.local/bin
```

Confira se `~/.local/bin` está no seu `PATH` e teste com `terraforme --version`. Para atualizar
depois, basta `terraforme upgrade`.

## Primeiros passos

```bash
terraforme extension --install   # instala RTK, AI Memory e AI Jail
terraforme config --mode hybrid  # define o modo padrão
terraforme cli                   # abre o agente no terminal do terraforme
```

## Documentação

- [Guia de uso](docs/guia.md) — todos os comandos, o terminal com abas e os atalhos.
- [Arquitetura](docs/arquitetura.md) — como o terraforme funciona por dentro.
- [Desenvolvimento](docs/desenvolvimento.md) — compilar, testar e contribuir.
- [Changelog](CHANGELOG.md)
