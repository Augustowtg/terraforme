# Feature: menu de abas da TUI

Menu do topo do `terraforme cli`, troca de abas e sub-abas. Implementação em
`crates/terraforme-cli/src/term.rs`; os atalhos completos estão na janela `⌃⌥K` (`shortcuts`).

## Comportamento

### Layout do menu

- Ocupa a primeira linha da tela (`App::draw_menu`).
- À esquerda, os blocos separados por um espaço (`App::menu_items`):
  `Terraforme`, `Agente (<agente>)`, `Consumo`, `Extensões`, `Configurações`, `Sair`.
- `<agente>` é o nome do agente da aba (ex.: `claude`, `opencode`). Com o
  [Middleware](agente/agente.feature.md) ligado, a aba se chama `Middleware`.
- **Extensões** só aparece quando o modo tem relatórios fora do RTK (AI Memory, AI Jail); os do
  RTK ficam no [Consumo](consumo/consumo.feature.md).
- À direita: `<provider> · modo <modo> (<descrição>)[ · AI Jail] · ⌃⌥K Atalhos`; com o mouse
  desligado, antes vem `🖱 seleção nativa · `. Só é desenhado se couber sem cobrir as abas.

```text
 Terraforme   Agente (claude)   Consumo   Extensões   Configurações   Sair       <provider · modo> ·  ⌃⌥K  Atalhos
└── verde ──┘└──── verde ─────┘└─ cinza ─┘└── cinza ──┘└──── cinza ────┘└cinza─┘                       └cinza┘
               (aba ativa)                                              (texto vermelho)
```

- Logo abaixo, à direita da linha das sub-abas, o total de tokens desta sessão do terraforme; a
  última linha da tela é o rodapé com o prompt atual e o total da instância (ver
  [Consumo sempre à vista](consumo/consumo.feature.md#consumo-sempre-à-vista)).

### Estilo

| Bloco | Estilo |
| --- | --- |
| `Terraforme` | preto em negrito sobre verde, sempre |
| Aba ativa | igual ao `Terraforme` |
| Outras abas | preto em negrito sobre cinza escuro |
| `Sair` | vermelho claro em negrito sobre cinza escuro |
| `⌃⌥K` | bloco cinza escuro; o resto do texto da direita em cinza |

### Atalhos de aba

| Atalho | Ação |
| --- | --- |
| `Ctrl+Shift+F1` | Agente |
| `Ctrl+Shift+F2` | Consumo |
| `Ctrl+Shift+F3` | Extensões (só quando a aba existe) |
| `Ctrl+Shift+F4` | Configurações |
| `Ctrl+F13`–`F16`, `F25`–`F28` | o mesmo, em terminais legados |
| `Ctrl+Shift+←` / `→` | aba anterior / próxima, com volta no fim (`tab_step`, `App::step_tab`) |
| `←` / `→` | trocam de aba no Consumo e em Extensões (na Configurações alteram o campo; no agente vão para ele) |
| `Esc` / `q` | nas abas de conteúdo, voltam ao agente |
| `Ctrl+Alt+K` | lista de atalhos |
| `F10` | sai (encerra todas as instâncias) |

Desativados de propósito, para chegarem ao agente: `F1`–`F9`, `Shift+F1`–`F9`, `Shift+1`/`2`,
`Ctrl+Shift+1`–`9`, `Shift+Tab`, `Ctrl+Shift+Tab`. `Ctrl+←→` também vai para o agente.

### Instâncias do agente

| Atalho | Ação |
| --- | --- |
| `Ctrl+Alt+N` ou `F12` | nova instância (até 9, `MAX_INSTANCES`) |
| `Ctrl+Alt+W`, `Shift+F12` ou `F24` | fecha a instância aberta |
| `Ctrl+Alt+R` | reinicia a instância aberta |
| `Ctrl+Alt+C` | copia a seleção, a tela do agente ou o conteúdo da aba |
| `Ctrl+Alt+S` | alterna a captura do mouse (`[tui] mouse`) |

### Clique com o mouse

Com a captura do mouse ligada, um clique esquerdo na primeira linha é resolvido por
`App::menu_hit` (`MenuItem`):

- numa aba: abre a aba (`MenuItem::Tab`);
- em `Sair`: encerra a TUI, como `F10` (`MenuItem::Quit`);
- em `Terraforme`, nos espaços ou no texto da direita: nada.

### Sub-abas

- Ficam na segunda linha (instâncias do agente) ou no topo da visão (Consumo, Extensões) e são
  desenhadas por `sub_tab_spans`: `⌃⌥N rótulo` (ou `⌥N` sem o protocolo de teclado do kitty).
- A aberta fica em destaque (preto sobre ciano); as outras, texto ciano. Os relatórios levam
  antes o nome da extensão como grupo.
- `Ctrl+Alt+Tab` (com `Shift`, para trás) e `Ctrl+PgUp` / `Ctrl+PgDn` alternam.
- `Ctrl+Alt+1`–`9`, `Alt+1`–`9` ou `Shift+Alt+1`–`9` vão direto à sub-aba
  (`sub_tab_number`); na aba do agente, um número além da última instância abre uma nova.

## Como funciona

- As abas vêm do registro `TABS` (`TabSpec`: aba, número do atalho, quando aparece, sub-abas
  próprias, `refresh` e `TabKind` `Agent`/`Content`/`Form`). Menu, atalhos, ajuda e abertura
  leem essa lista (`spec(tab)`); uma aba nova é uma entrada nova.
- `Ctrl+Shift`/`Ctrl+Alt` chegam distintos quando o terminal aceita o protocolo de teclado do
  kitty (`App::enhanced`); sem ele, `Ctrl+Alt+dígito` costuma chegar como `Alt+dígito`.
- Abas que não são do agente carregam o conteúdo numa thread ao serem abertas.
