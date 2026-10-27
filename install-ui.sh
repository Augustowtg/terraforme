# Interface do instalador: banner, versão, barra de progresso, lista de passos
# e confirmação SIM/NÃO. Carregado por install-linux.sh e install-win.sh.
#
# Uso:
#   ui_init "$SCRIPT_DIR" "Passo 1" "Passo 2" ...   (o passo 1 é a confirmação)
#   ui_summaries "" "Resumo 2" ...                (opcional; "" é sem resumo)
#   ui_confirm "Pergunta?" || exit 0
#   ui_step <função> [args...]                     (roda o próximo passo)
#   ui_ask "Pergunta?"                             (passo com SIM/NÃO; NÃO segue)
#   ui_select "Pergunta?" "Opção 1" "Opção 2" ...  (marca as opções em UI_SELECTED)
#   ui_add_step "Passo" "Resumo"                   (acrescenta um passo no fim)
#   ui_note "mensagem"                             (mostrada no resumo final)
#   ui_finish "mensagem final"

C_RESET=$'\033[0m'
C_BOLD=$'\033[1m'
C_DIM=$'\033[2m'
C_GREEN=$'\033[32m'
C_RED=$'\033[31m'
C_YELLOW=$'\033[33m'
C_CYAN=$'\033[36m'
C_ON_GREEN=$'\033[1;30;42m'
C_ON_RED=$'\033[1;30;41m'

BANNER=(
  "█████  █████  ████   ████    ███   █████   ███   ████   █   █  █████"
  "  █    █      █   █  █   █  █   █  █      █   █  █   █  ██ ██  █"
  "  █    ████   ████   ████   █████  ████   █   █  ████   █ █ █  ████"
  "  █    █      █  █   █  █   █   █  █      █   █  █  █   █   █  █"
  "  █    █████  █   █  █   █  █   █  █       ███   █   █  █   █  █████"
)

UI_STEPS=()
UI_SUMMARIES=()
UI_SELECTED=()
UI_OPTIONS=()
UI_CURRENT=0
UI_FAILED=0
UI_NOTES=()
UI_VERSION=""
UI_LOG=""
UI_INTERACTIVE=0

ui_init() {
  local dir="$1"
  shift
  UI_STEPS=("$@")
  UI_VERSION="$(grep -m1 '^version' "$dir/Cargo.toml" | cut -d'"' -f2)"
  UI_LOG="$(mktemp)"
  if [ -t 0 ] && [ -t 1 ]; then
    UI_INTERACTIVE=1
  fi
  trap ui_on_exit EXIT
}

ui_on_exit() {
  local rc=$?
  printf '\033[?25h'
  if [ "$rc" -ne 0 ] && [ "$UI_CURRENT" -lt "${#UI_STEPS[@]}" ]; then
    UI_FAILED=1
    ui_render
    echo "${C_RED}${C_BOLD}Falha em: ${UI_STEPS[$UI_CURRENT]}${C_RESET}"
    echo
    tail -n 30 "$UI_LOG"
    echo
    echo "${C_DIM}Log completo: $UI_LOG${C_RESET}"
  else
    rm -f "$UI_LOG"
  fi
}

# O resumo de cada passo, na ordem do ui_init, mostrado ao lado da label só
# enquanto o passo roda (ou no passo que falhou). Passos sem resumo usam "".
ui_summaries() {
  UI_SUMMARIES=("$@")
}

# " — resumo" em cinza, ou nada quando o passo não tem resumo.
ui_summary() {
  local summary="${UI_SUMMARIES[$1]:-}"
  if [ -n "$summary" ]; then
    printf '  %s— %s%s' "$C_DIM" "$summary" "$C_RESET"
  fi
}

ui_note() {
  UI_NOTES+=("$1")
}

ui_progress_bar() {
  local total=${#UI_STEPS[@]} width=40 filled pct
  filled=$((UI_CURRENT * width / total))
  pct=$((UI_CURRENT * 100 / total))
  local bar="" i
  for ((i = 0; i < width; i++)); do
    if [ "$i" -lt "$filled" ]; then bar+="█"; else bar+="░"; fi
  done
  printf '  %s%s%s %3d%%  %s(%d/%d)%s\n' "$C_GREEN" "$bar" "$C_RESET" "$pct" "$C_DIM" "$UI_CURRENT" "$total" "$C_RESET"
}

ui_render() {
  local line i
  printf '\033[H\033[2J\033[3J'
  echo
  for line in "${BANNER[@]}"; do
    echo "  ${C_GREEN}${C_BOLD}${line}${C_RESET}"
  done
  echo
  echo "  ${C_DIM}versão${C_RESET} ${C_CYAN}${C_BOLD}v${UI_VERSION}${C_RESET}"
  echo
  ui_progress_bar
  echo
  for i in "${!UI_STEPS[@]}"; do
    if [ "$i" -lt "$UI_CURRENT" ]; then
      echo "  ${C_GREEN}✔${C_RESET} ${UI_STEPS[$i]}"
    elif [ "$i" -eq "$UI_CURRENT" ] && [ "$UI_FAILED" -eq 1 ]; then
      echo "  ${C_RED}✘ ${UI_STEPS[$i]}${C_RESET}$(ui_summary "$i")"
    elif [ "$i" -eq "$UI_CURRENT" ]; then
      echo "  ${C_YELLOW}●${C_RESET} ${C_BOLD}${UI_STEPS[$i]}${C_RESET}$(ui_summary "$i")"
    else
      echo "  ${C_DIM}○ ${UI_STEPS[$i]}${C_RESET}"
    fi
  done
  echo
}

ui_buttons() {
  local selected="$1" yes no
  if [ "$selected" -eq 0 ]; then
    yes="${C_ON_GREEN}  ▶ SIM  ${C_RESET}"
    no="${C_RED}    NÃO  ${C_RESET}"
  else
    yes="${C_GREEN}    SIM  ${C_RESET}"
    no="${C_ON_RED}  ▶ NÃO  ${C_RESET}"
  fi
  printf '\r\033[K    %s   %s' "$yes" "$no"
}

# Pergunta SIM/NÃO selecionado pelas setas ←/→ e confirmado com Enter.
# Retorna 0 para SIM. Com -y, responde SIM sem perguntar; sem terminal
# interativo, exige o -y.
ui_choose() {
  local question="$1" selected=0 key rest
  if [ "${UI_ASSUME_YES:-0}" -eq 1 ]; then
    return 0
  fi
  if [ "$UI_INTERACTIVE" -ne 1 ]; then
    echo "erro: terminal não interativo. Rode com -y para instalar sem confirmação." >&2
    UI_CURRENT=${#UI_STEPS[@]}
    exit 1
  fi
  ui_render
  echo "  ${C_BOLD}${question}${C_RESET}"
  echo "  ${C_DIM}←/→ para escolher, Enter para confirmar${C_RESET}"
  echo
  printf '\033[?25l'
  while true; do
    ui_buttons "$selected"
    IFS= read -rsn1 key || key=""
    if [ "$key" = $'\e' ]; then
      IFS= read -rsn2 -t 0.05 rest || rest=""
      key+="$rest"
    fi
    case "$key" in
      $'\e[D' | $'\e[C' | $'\t' | h | l) selected=$((1 - selected)) ;;
      s | S) selected=0 && break ;;
      n | N) selected=1 && break ;;
      "") break ;;
    esac
  done
  printf '\033[?25h\n'
  return "$selected"
}

# A confirmação da instalação: NÃO cancela tudo.
ui_confirm() {
  if ui_choose "$1"; then
    UI_CURRENT=$((UI_CURRENT + 1))
    return 0
  fi
  UI_CURRENT=${#UI_STEPS[@]}
  echo
  echo "  ${C_YELLOW}Instalação cancelada.${C_RESET}"
  return 1
}

# Um passo que é uma pergunta: o passo conta como feito com SIM ou NÃO, e
# o retorno diz a resposta (0 para SIM) para o script pular os seguintes.
ui_ask() {
  local rc=0
  ui_choose "$1" || rc=1
  UI_CURRENT=$((UI_CURRENT + 1))
  return "$rc"
}

# As linhas do menu do ui_select, como a lista de extensões da Config:
# "[x] 1. Opção", com o ▌ e em negrito na opção em foco.
ui_select_lines() {
  local current="$1" i check
  for i in "${!UI_SELECTED[@]}"; do
    if [ "${UI_SELECTED[$i]}" -eq 1 ]; then check="[x]"; else check="[ ]"; fi
    if [ "$i" -eq "$current" ]; then
      printf '\r\033[K  %s▌ %s %d. %s%s\n' "$C_GREEN$C_BOLD" "$check" $((i + 1)) "${UI_OPTIONS[$i]}" "$C_RESET"
    elif [ "${UI_SELECTED[$i]}" -eq 1 ]; then
      printf '\r\033[K    %s%s%s %d. %s\n' "$C_GREEN" "$check" "$C_RESET" $((i + 1)) "${UI_OPTIONS[$i]}"
    else
      printf '\r\033[K    %s%s%s %d. %s\n' "$C_DIM" "$check" "$C_RESET" $((i + 1)) "${UI_OPTIONS[$i]}"
    fi
  done
}

# Menu com várias opções, todas marcadas no começo: ↑/↓ andam, Espaço marca
# ou desmarca e Enter confirma. Deixa em UI_SELECTED[i] 1 para as marcadas.
# Com -y, marca todas sem perguntar; sem terminal interativo, exige o -y.
ui_select() {
  local question="$1" current=0 key rest n i
  shift
  UI_OPTIONS=("$@")
  n=$#
  UI_SELECTED=()
  for ((i = 0; i < n; i++)); do UI_SELECTED[i]=1; done
  if [ "${UI_ASSUME_YES:-0}" -eq 1 ]; then
    return 0
  fi
  if [ "$UI_INTERACTIVE" -ne 1 ]; then
    echo "erro: terminal não interativo. Rode com -y para instalar sem confirmação." >&2
    UI_CURRENT=${#UI_STEPS[@]}
    exit 1
  fi
  ui_render
  echo "  ${C_BOLD}${question}${C_RESET}"
  echo "  ${C_DIM}↑/↓ para escolher, Espaço para marcar, Enter para confirmar${C_RESET}"
  echo
  printf '\033[?25l'
  while true; do
    ui_select_lines "$current"
    IFS= read -rsn1 key || key=""
    if [ "$key" = $'\e' ]; then
      IFS= read -rsn2 -t 0.05 rest || rest=""
      key+="$rest"
    fi
    case "$key" in
      $'\e[A' | k) [ "$current" -gt 0 ] && current=$((current - 1)) ;;
      $'\e[B' | j | $'\t') [ "$current" -lt $((n - 1)) ] && current=$((current + 1)) ;;
      " ") UI_SELECTED[current]=$((1 - UI_SELECTED[current])) ;;
      "") break ;;
    esac
    printf '\033[%dA' "$n"
  done
  printf '\033[?25h'
}

# Acrescenta um passo (e o resumo dele) no fim da lista, para os passos que
# só existem depois de uma resposta (ex.: as extensões escolhidas).
ui_add_step() {
  UI_SUMMARIES[${#UI_STEPS[@]}]="${2:-}"
  UI_STEPS+=("$1")
}

# Roda o próximo passo com a saída no log. Se falhar, o set -e encerra o
# script e o trap de saída mostra o passo com erro e o fim do log.
ui_step() {
  ui_render
  "$@" >>"$UI_LOG" 2>&1
  UI_CURRENT=$((UI_CURRENT + 1))
}

ui_finish() {
  local note
  ui_render
  for note in "${UI_NOTES[@]}"; do
    echo "  $note"
  done
  echo
  echo "  ${C_GREEN}${C_BOLD}$1${C_RESET}"
}
