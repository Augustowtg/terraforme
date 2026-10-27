#!/usr/bin/env bash
# Instala o terraforme no Linux e adiciona a função `terra` ao bash, zsh e fish.
set -euo pipefail

BEGIN_MARK="# >>> terraforme >>>"
END_MARK="# <<< terraforme <<<"
BIN_DIR="$HOME/.local/bin"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# shellcheck source=install-ui.sh
. "$SCRIPT_DIR/install-ui.sh"

UI_ASSUME_YES=0
for arg in "$@"; do
  case "$arg" in
    -y | --yes) UI_ASSUME_YES=1 ;;
  esac
done

# O binário compilado da release, baixado pelo install.sh; sem ele, compila
# com o cargo.
PREBUILT="${TERRAFORME_BIN:-}"
BUILT="$SCRIPT_DIR/target/release/terra"

if [ -z "$PREBUILT" ] && ! command -v cargo >/dev/null 2>&1; then
  echo "erro: 'cargo' não encontrado. Instale o Rust em https://rustup.rs e rode o script de novo." >&2
  exit 1
fi

build() {
  if [ -n "$PREBUILT" ]; then
    return
  fi
  cargo build --release --manifest-path "$SCRIPT_DIR/Cargo.toml"
}

install_bin() {
  mkdir -p "$BIN_DIR"
  install -m 755 "${PREBUILT:-$BUILT}" "$BIN_DIR/terra"
  ui_note "Binário instalado em $BIN_DIR/terra"
}

POSIX_FUNC='terra() {
  local tmp rc dir
  tmp="$(mktemp)" || return 1
  command terra "$@" --cwd-file "$tmp"
  rc=$?
  if [ -s "$tmp" ]; then
    dir="$(cat "$tmp")"
    [ -d "$dir" ] && cd "$dir"
  fi
  rm -f "$tmp"
  return $rc
}'

FISH_FUNC='function terra
    set -l tmp (mktemp); or return 1
    command terra $argv --cwd-file $tmp
    set -l rc $status
    if test -s $tmp
        set -l dir (cat $tmp)
        test -d "$dir"; and cd "$dir"
    end
    rm -f $tmp
    return $rc
end'

# Remove um bloco antigo (entre os marcadores) e grava o novo no fim do arquivo.
write_block() {
  local file="$1" body="$2" tmp
  mkdir -p "$(dirname "$file")"
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

setup_shells() {
  write_block "$HOME/.bashrc" "$POSIX_FUNC"

  if [ -f "$HOME/.zshrc" ] || command -v zsh >/dev/null 2>&1; then
    write_block "$HOME/.zshrc" "$POSIX_FUNC"
  fi

  if [ -d "$HOME/.config/fish" ] || command -v fish >/dev/null 2>&1; then
    write_block "$HOME/.config/fish/config.fish" "$FISH_FUNC"
  fi

  case ":$PATH:" in
    *":$BIN_DIR:"*) ;;
    *) ui_note "${C_YELLOW}aviso: $BIN_DIR não está no PATH. Adicione-o ao seu shell para usar o 'terra'.${C_RESET}" ;;
  esac
}

# As extensões do agente (aba Extensões). O terra passa os hooks e o MCP
# delas ao agente; o RTK só precisa do binário, e o ai-memory, do servidor
# que o `ai-memory setup` instala como serviço do usuário.
install_rtk() {
  if command -v rtk >/dev/null 2>&1; then
    ui_note "RTK já instalado: $(rtk --version)"
    return
  fi
  if ! command -v cargo >/dev/null 2>&1; then
    ui_note "${C_YELLOW}aviso: o RTK não foi instalado: ele é compilado com o cargo. Instale o Rust em https://rustup.rs e rode o instalador de novo.${C_RESET}"
    return
  fi
  cargo install --locked --git https://github.com/rtk-ai/rtk rtk
  ui_note "RTK instalado em ~/.cargo/bin/rtk"
}

install_ai_memory() {
  local bin
  bin="$(command -v ai-memory || true)"
  if [ -z "$bin" ]; then
    if ! command -v curl >/dev/null 2>&1; then
      echo "erro: 'curl' não encontrado; é preciso para baixar o ai-memory." >&2
      return 1
    fi
    curl -fsSL https://github.com/akitaonrails/ai-memory/releases/latest/download/install.sh | sh
    bin="$HOME/.local/bin/ai-memory"
    ui_note "ai-memory instalado em $bin"
  else
    ui_note "ai-memory já instalado: $("$bin" --version)"
  fi
  "$bin" setup --yes
  ui_note "ai-memory configurado: servidor como serviço do usuário e agentes ligados"
}

# O terraforme antigo, o de antes do terra (com o daemon terraforme-mcp e
# o mascote terraforme-pet): os binários dele no PATH ou em BIN_DIR.
OLD_BINS=(terraforme terraforme-mcp terraforme-pet)
OLD_PATHS=()
for bin in "${OLD_BINS[@]}"; do
  for path in "$(command -v "$bin" || true)" "$BIN_DIR/$bin"; do
    if [ -f "$path" ] && ! printf '%s\n' "${OLD_PATHS[@]}" | grep -qxF "$path"; then
      OLD_PATHS+=("$path")
    fi
  done
done

# Guarda uma pasta do terraforme antigo com o sufixo .antigo-<data>, em vez
# de apagar, para dar para voltar.
backup_dir() {
  local dir="$1" backup
  [ -d "$dir" ] || return 0
  backup="$dir.antigo-$(date +%Y%m%d-%H%M%S)"
  mv "$dir" "$backup"
  ui_note "Pasta do terraforme antigo guardada em $backup"
}

# Encerra o daemon e o mascote (o `terraforme cli` aberto continua até ser
# fechado), remove os binários e guarda a config e os dados. O RTK, o
# ai-memory e o ~/.terraforme do terra ficam.
uninstall_old_terraforme() {
  local path
  pkill -x terraforme-mcp || true
  pkill -x terraforme-pet || true
  for path in "${OLD_PATHS[@]}"; do
    rm -f "$path"
    ui_note "Removido o terraforme antigo: $path"
  done
  backup_dir "${XDG_CONFIG_HOME:-$HOME/.config}/terraforme"
  backup_dir "${XDG_DATA_HOME:-$HOME/.local/share}/terraforme"
}

# As extensões que já estão no PATH (como o terra as detecta): 1 para o RTK
# e para o ai-memory, na ordem do menu.
EXT_INSTALLED=(0 0)
if command -v rtk >/dev/null 2>&1; then EXT_INSTALLED[0]=1; fi
if command -v ai-memory >/dev/null 2>&1; then EXT_INSTALLED[1]=1; fi

# O passo 2 compila ou usa o binário baixado da release.
if [ -n "$PREBUILT" ]; then
  BUILD_LABEL="Usar o binário compilado da release"
  BUILD_SUMMARY="baixado do GitHub; não precisa compilar"
else
  BUILD_LABEL="Compilar em modo release"
  BUILD_SUMMARY="cargo build --release; pode levar alguns minutos"
fi

ui_init "$SCRIPT_DIR" \
  "Confirmar instalação" \
  "$BUILD_LABEL" \
  "Instalar o binário em $BIN_DIR" \
  "Adicionar a função 'terra' aos shells"

# Os arquivos que o setup_shells altera, para o resumo do passo.
shell_files() {
  local files="~/.bashrc"
  if [ -f "$HOME/.zshrc" ] || command -v zsh >/dev/null 2>&1; then
    files+=", ~/.zshrc"
  fi
  if [ -d "$HOME/.config/fish" ] || command -v fish >/dev/null 2>&1; then
    files+=", ~/.config/fish/config.fish"
  fi
  echo "$files"
}

ui_summaries \
  "" \
  "$BUILD_SUMMARY" \
  "copia o binário" \
  "$(shell_files)"

# O passo do terraforme antigo só aparece se ele estiver na máquina.
if [ "${#OLD_PATHS[@]}" -gt 0 ]; then
  ui_add_step "Desinstalar o terraforme antigo" "encerra e remove terraforme, terraforme-mcp e terraforme-pet"
fi
# Com todas as extensões instaladas, o instalador nem pergunta.
if [ "${EXT_INSTALLED[0]}" -eq 0 ] || [ "${EXT_INSTALLED[1]}" -eq 0 ]; then
  ui_add_step "Instalar as extensões?" ""
fi

ui_confirm "Instalar o terraforme CLI?" || exit 0
ui_step build
ui_step install_bin
ui_step setup_shells
if [ "${#OLD_PATHS[@]}" -gt 0 ]; then
  ui_ask_step "Desinstalar o terraforme antigo (a config fica guardada como .antigo)?" uninstall_old_terraforme
fi
# Os passos das extensões só entram na lista com o SIM, e só os das
# escolhidas no menu; as já instaladas aparecem nele com a bola verde.
if [ "${EXT_INSTALLED[0]}" -eq 1 ] && [ "${EXT_INSTALLED[1]}" -eq 1 ]; then
  ui_note "Extensões já instaladas: RTK e ai-memory"
elif ui_ask "Instalar as extensões (RTK e ai-memory)?"; then
  UI_INSTALLED=("${EXT_INSTALLED[@]}")
  ui_select "Quais extensões instalar?" \
    "RTK - filtra e comprime a saída dos comandos antes de ela chegar ao agente" \
    "ai-memory - memória de longo prazo entre sessões e agentes"
  if [ "${UI_SELECTED[0]}" -eq 1 ]; then
    ui_add_step "Instalar o RTK" "cargo install --git https://github.com/rtk-ai/rtk rtk"
  fi
  if [ "${UI_SELECTED[1]}" -eq 1 ]; then
    ui_add_step "Instalar e configurar o ai-memory" "install.sh do GitHub e ai-memory setup"
  fi
  if [ "${UI_SELECTED[0]}" -eq 1 ]; then ui_step install_rtk; fi
  if [ "${UI_SELECTED[1]}" -eq 1 ]; then ui_step install_ai_memory; fi
fi
ui_finish "Pronto! Abra um novo terminal (ou rode 'source ~/.bashrc') e use: terra workspace"
