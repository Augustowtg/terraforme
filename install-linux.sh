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

if ! command -v cargo >/dev/null 2>&1; then
  echo "erro: 'cargo' não encontrado. Instale o Rust em https://rustup.rs e rode o script de novo." >&2
  exit 1
fi

build() {
  cargo build --release --manifest-path "$SCRIPT_DIR/Cargo.toml"
}

install_bin() {
  mkdir -p "$BIN_DIR"
  install -m 755 "$SCRIPT_DIR/target/release/terra" "$BIN_DIR/terra"
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

ui_init "$SCRIPT_DIR" \
  "Confirmar instalação" \
  "Compilar em modo release" \
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
  "cargo build --release; pode levar alguns minutos" \
  "copia target/release/terra" \
  "$(shell_files)"

ui_confirm "Instalar o terraforme CLI?" || exit 0
ui_step build
ui_step install_bin
ui_step setup_shells
ui_finish "Pronto! Abra um novo terminal (ou rode 'source ~/.bashrc') e use: terra workspace"
