# Janela flutuante (mascote)

Uso, estados, funcionamento e instalação do `terraforme pet`, a janela com o mascote (o tatu-peba)
que acompanha os agentes de todos os `terraforme cli` abertos. Só no **Linux** por enquanto; o que
falta, inclusive o Windows, está no [ROADMAP](../ROADMAP.md) (issue #34).

## O que faz

- Janela pequena, sem borda e transparente, sobre as outras janelas.
- O mascote muda de animação conforme o que os agentes estão fazendo.
- Quando um agente **termina** ou **precisa de você**, mostra um balão com a instância e uma
  notificação do sistema.
- Um clique leva ao terminal da instância escolhida e abre a sub-aba dela (se o terminal está na
  aba Consumo, por exemplo, ele volta para a aba do agente, na sub-aba `claude 2`).

## Uso

```bash
terraforme pet                 # abre a janela (uma só por usuário)
terraforme config --pet true   # abre a janela junto com o terraforme cli (false desliga)
```

- Na TUI, o campo **Mascote** (aba Configurações, seção Plugins · config global) faz o mesmo e já
  abre ou fecha a janela ao alternar.
- A janela continua aberta depois que os terminais fecham; sem agentes abertos, o mascote dorme.
- `terraforme restart` reabre a janela, se estava aberta, com o binário novo.

| Ação | O que faz |
| --- | --- |
| Clique (esquerdo ou direito) no mascote | abre ou fecha o menu dos agentes |
| Clique num agente do menu | vai ao terminal dele, na sub-aba da instância, e limpa o aviso |
| Clique no balão | vai à instância do aviso |
| `✕ Sair` no menu | fecha a janela |
| Arrastar o mascote | move a janela |

No menu, cada agente aparece como `<space ou pasta> · claude 2`, com o estado:

| Ícone | Estado |
| --- | --- |
| `●` | trabalhando |
| `✔` | terminou |
| `!` | precisa de você |
| `○` | ocioso |

## Estados do mascote

O mascote mostra o estado mais urgente entre todas as instâncias:

| Animação | Quando |
| --- | --- |
| Perigo | algum agente pediu sua atenção (sino do terminal ou notificação OSC 9/777, ex.: pedido de permissão) |
| Notificação | algum agente terminou e você ainda não olhou |
| Atenção | algum agente está trabalhando |
| Feliz | agentes abertos, todos ociosos |
| Cansado | agentes abertos e ociosos há mais de 10 minutos |
| Dormindo | nenhum agente aberto (ou o daemon MCP não está rodando) |

## Como funciona

```text
terraforme cli (TUI) ──POST /instances/state──▶ terraforme-mcp (hub) ◀──GET /pet/state── terraforme pet
        ▲                                                │                                    │
        └──────────── POST /focus ◀──────────────────────┴────────── POST /pet/focus ◀───────┘
```

1. **Detecção na TUI.** Cada instância tem um tracker (`terraforme_core::pet::Tracker`)
   alimentado pela saída do pseudoterminal e pelo que você digita:
   - saída contínua por 2s sem digitação no último 1,5s (o eco da digitação não conta) =
     **trabalhando**;
   - 4s sem saída depois de trabalhar = **terminou**;
   - sino (`BEL`) ou notificação OSC 9/777 = **precisa de você**;
   - abrir a sub-aba da instância ou digitar nela = **ocioso** (aviso visto).

   Os primeiros 5s depois de abrir a instância são ignorados (a tela inicial do agente). Só as
   mudanças vão para o daemon, em `POST /instances/state`.
2. **Daemon.** O `hub` guarda o estado, a hora da mudança e o número da sub-aba de cada instância.
   `GET /pet/state` devolve as instâncias de todos os terminais; `POST /pet/focus` repassa
   `POST /focus` ao terminal dono da instância. Os dois só aceitam pedidos locais (`Host` e
   `Origin`) e não dependem de `[mcp] ui`. Veja [MCP](MCP/mcp.feature.md).
3. **Terminal.** O `POST /focus` abre a aba do agente na sub-aba da instância (o que marca o aviso
   como visto) e tenta trazer a janela do terminal para frente.
4. **Janela.** O `terraforme-pet` (egui/eframe) lê o lock file do daemon e consulta `/pet/state` a
   cada segundo. Quando uma instância passa a "terminou" ou "precisa de você", mostra o balão e
   chama `notify-send` (se instalado).

Os sprites são os de `assets/*_strip.png` do repositório [terraforme-pet](https://github.com/Augustowtg/terraforme-pet) (4 quadros de 48×34), embutidos no binário e desenhados
em escala 3 sem suavização.

## Trazer o terminal para frente

| Sessão | Comportamento |
| --- | --- |
| GNOME (X11 e Wayland) | a extensão `terraforme@augustowtg.github.io` (instalada e habilitada pelo `install.linux.sh`) ativa a janela do processo pai mais próximo do `terraforme cli`: troca de área de trabalho, tira do minimizado e foca. No Wayland, saia e entre na sessão depois de instalar para o GNOME carregá-la. |
| X11 | `xdotool windowactivate` (ou `wmctrl -i -a`) na janela de `$WINDOWID`; sem ela, procura a janela dos processos pais (`xdotool search --pid`). Instale `xdotool` ou `wmctrl`. |
| Wayland fora do GNOME | o protocolo não deixa um app ativar a janela de outro: a sub-aba muda, mas a janela precisa ser trazida à mão. A notificação do sistema diz qual terminal. |

- Em terminais com abas próprias (gnome-terminal, Konsole), a janela vem para frente, mas a aba do
  emulador não muda.
- O "sempre no topo" depende do compositor: no GNOME/Wayland ele é ignorado.

## Instalação

- Instalado pelo `install.linux.sh`, junto com `terraforme` e `terraforme-mcp` (veja o
  [Guia](guia.md#do-código-fonte-linux)).
- Os binários do release (`install.sh`) ainda não o trazem: a build estática com musl não carrega
  as bibliotecas gráficas (X11/Wayland/OpenGL).
- `terraforme pet` procura o binário ao lado do `terraforme` e depois no `PATH`.
- Opcionais: `notify-send` (notificações), `xdotool` ou `wmctrl` (X11 fora do GNOME).
