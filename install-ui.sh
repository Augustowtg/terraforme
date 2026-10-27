# Interface do instalador: banner, versão, barra de progresso, lista de passos
# e confirmação SIM/NÃO. Carregado por install-linux.sh e install-win.sh.
#
# Uso:
#   ui_init "$SCRIPT_DIR" "Passo 1" "Passo 2" ...   (o passo 1 é a confirmação)
#   ui_summaries "" "Resumo 2" ...                (opcional; "" é sem resumo)
#   ui_confirm "Pergunta?" || exit 0
#   ui_step <função> [args...]                     (roda o próximo passo)
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
# Retorna 0 para SIM. Sem terminal interativo, exige o argumento -y.
ui_confirm() {
  local question="$1" selected=0 key rest
  if [ "${UI_ASSUME_YES:-0}" -eq 1 ]; then
    UI_CURRENT=$((UI_CURRENT + 1))
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
  if [ "$selected" -eq 0 ]; then
    UI_CURRENT=$((UI_CURRENT + 1))
    return 0
  fi
  UI_CURRENT=${#UI_STEPS[@]}
  echo
  echo "  ${C_YELLOW}Instalação cancelada.${C_RESET}"
  return 1
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
