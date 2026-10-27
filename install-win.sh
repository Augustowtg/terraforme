#!/usr/bin/env bash
# Instala o terraforme no Windows. Rode pelo Git Bash.
set -euo pipefail

BEGIN_MARK="# >>> terraforme >>>"
END_MARK="# <<< terraforme <<<"
BIN_DIR="$HOME/.terraforme/bin"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# shellcheck source=install-ui.sh
. "$SCRIPT_DIR/install-ui.sh"

UI_ASSUME_YES=0
for arg in "$@"; do
  case "$arg" in
    -y | --yes) UI_ASSUME_YES=1 ;;
  esac
done

if ! command -v cargo >/dev/null 2>&1; then
  echo "erro: 'cargo' não encontrado. Instale o Rust em https://rustup.rs, reabra o Git Bash e rode o script de novo." >&2
  exit 1
fi

build() {
  cargo build --release --manifest-path "$SCRIPT_DIR/Cargo.toml"
}

install_bin() {
  mkdir -p "$BIN_DIR"
  cp "$SCRIPT_DIR/target/release/terra.exe" "$BIN_DIR/terra.exe"
  # Versões anteriores instalavam como terraforme.exe nesta pasta, o que
  # escondia o terraforme principal no PATH.
  rm -f "$BIN_DIR/terraforme.exe"
  ui_note "Binário instalado em $BIN_DIR/terra.exe"
}

# O binário é nativo do Windows: recebe o arquivo temporário em formato
# Windows (cygpath -w) e devolve um path Windows, convertido de volta com
# cygpath -u antes do cd.
BLOCK='export PATH="$HOME/.terraforme/bin:$PATH"

terra() {
  local tmp rc dir
  tmp="$(mktemp)" || return 1
  command terra "$@" --cwd-file "$(cygpath -w "$tmp")"
  rc=$?
  if [ -s "$tmp" ]; then
    dir="$(cygpath -u "$(cat "$tmp")")"
    [ -d "$dir" ] && cd "$dir"
  fi
  rm -f "$tmp"
  return $rc
}'

# Remove um bloco antigo (entre os marcadores) e grava o novo no fim do arquivo.
write_block() {
  local file="$1" body="$2" tmp
  touch "$file"
  tmp="$(mktemp)"
  awk -v b="$BEGIN_MARK" -v e="$END_MARK" '
    $0 == b { skip = 1 }
    !skip   { print }
    $0 == e { skip = 0 }
  ' "$file" >"$tmp"
  printf '%s\n%s\n%s\n' "$BEGIN_MARK" "$body" "$END_MARK" >>"$tmp"
  cat "$tmp" >"$file"
  rm -f "$tmp"
  ui_note "Função 'terra' adicionada em $file"
}

setup_shell() {
  write_block "$HOME/.bashrc" "$BLOCK"

  # O Git Bash abre como login shell e lê o ~/.bash_profile; garante que ele
  # carregue o ~/.bashrc.
  if [ ! -f "$HOME/.bash_profile" ] || ! grep -q '\.bashrc' "$HOME/.bash_profile"; then
    printf '\n[ -f ~/.bashrc ] && . ~/.bashrc\n' >>"$HOME/.bash_profile"
    ui_note "~/.bash_profile agora carrega o ~/.bashrc"
  fi
}

# As extensões do agente (aba Extensões). O terra passa os hooks e o MCP
# delas ao agente; o RTK só precisa do binário, e o ai-memory, do servidor
# que o `ai-memory setup` deixa rodando ao fazer login.
install_rtk() {
  if command -v rtk >/dev/null 2>&1; then
    ui_note "RTK já instalado: $(rtk --version)"
    return
  fi
  cargo install --locked --git https://github.com/rtk-ai/rtk rtk
  ui_note "RTK instalado em ~/.cargo/bin/rtk.exe"
}

install_ai_memory() {
  local bin
  bin="$(command -v ai-memory || true)"
  if [ -z "$bin" ]; then
    # O instalador do Windows é em PowerShell e põe o ai-memory no PATH do
    # usuário, que só vale para os terminais abertos depois.
    powershell.exe -NoProfile -ExecutionPolicy Bypass -Command \
      "irm https://github.com/akitaonrails/ai-memory/releases/latest/download/install.ps1 | iex"
    bin="$(cygpath -u "$LOCALAPPDATA")/Programs/ai-memory/ai-memory.exe"
    ui_note "ai-memory instalado em $bin"
  else
    ui_note "ai-memory já instalado: $("$bin" --version)"
  fi
  "$bin" setup --yes
  ui_note "ai-memory configurado: servidor ao fazer login e agentes ligados"
}

ui_init "$SCRIPT_DIR" \
  "Confirmar instalação" \
  "Compilar em modo release" \
  "Instalar o binário em $BIN_DIR" \
  "Adicionar a função 'terra' ao Git Bash" \
  "Instalar as extensões?"

ui_summaries \
  "" \
  "cargo build --release; pode levar alguns minutos" \
  "copia target/release/terra.exe e remove o antigo terraforme.exe" \
  "~/.bashrc e ~/.bash_profile" \
  ""

ui_confirm "Instalar o terraforme CLI?" || exit 0
ui_step build
ui_step install_bin
ui_step setup_shell
# Os passos das extensões só entram na lista com o SIM, e só os das
# escolhidas no menu.
if ui_ask "Instalar as extensões (RTK e ai-memory)?"; then
  ui_select "Quais extensões instalar?" \
    "RTK - filtra e comprime a saída dos comandos antes de ela chegar ao agente" \
    "ai-memory - memória de longo prazo entre sessões e agentes"
  if [ "${UI_SELECTED[0]}" -eq 1 ]; then
    ui_add_step "Instalar o RTK" "cargo install --git https://github.com/rtk-ai/rtk rtk"
  fi
  if [ "${UI_SELECTED[1]}" -eq 1 ]; then
    ui_add_step "Instalar e configurar o ai-memory" "install.ps1 do GitHub e ai-memory setup"
  fi
  if [ "${UI_SELECTED[0]}" -eq 1 ]; then ui_step install_rtk; fi
  if [ "${UI_SELECTED[1]}" -eq 1 ]; then ui_step install_ai_memory; fi
fi
ui_finish "Pronto! Feche e reabra o Git Bash para usar: terra workspace"
