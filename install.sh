#!/bin/sh
# Instala ou atualiza o terraforme a partir das releases do GitHub (sem Rust).
#
#   curl -fsSL https://raw.githubusercontent.com/Augustowtg/terraforme/main/install.sh | sh
#
# TERRAFORME_VERSION=v0.19.0   versão a instalar (padrão: a última release)
# PREFIX=/usr/local            instala em $PREFIX/bin (padrão: ~/.local; pede sudo se preciso)
#
# Para compilar a partir do código, use o install.linux.sh.
set -eu

REPO="Augustowtg/terraforme"
PREFIX="${PREFIX:-$HOME/.local}"
BIN_DIR="$PREFIX/bin"

info() { printf '\033[1;34m==>\033[0m %s\n' "$1"; }
ok() { printf '\033[1;32m✔\033[0m %s\n' "$1"; }
die() { printf '\033[1;31m✘\033[0m %s\n' "$1" >&2; exit 1; }

[ "$(uname -s)" = Linux ] || die "Este script é só para Linux (no Windows use o install.ps1)."
case "$(uname -m)" in
    x86_64 | amd64) arch=x86_64 ;;
    aarch64 | arm64) arch=aarch64 ;;
    *) die "Arquitetura $(uname -m) sem binário pronto — compile com o install.linux.sh." ;;
esac
target="$arch-unknown-linux-musl"
command -v curl >/dev/null 2>&1 || die "curl não encontrado."

tag="${TERRAFORME_VERSION:-}"
if [ -z "$tag" ]; then
    tag=$(curl -fsSL "https://api.github.com/repos/$REPO/releases/latest" |
        sed -n 's/.*"tag_name": *"\([^"]*\)".*/\1/p' | head -n 1)
    [ -n "$tag" ] || die "Não encontrei a última release de $REPO."
fi
case "$tag" in v*) ;; *) tag="v$tag" ;; esac
version="${tag#v}"

previous=""
if [ -x "$BIN_DIR/terraforme" ]; then
    previous=$("$BIN_DIR/terraforme" version 2>/dev/null || true)
fi
if [ "$previous" = "terraforme $version" ] && [ -x "$BIN_DIR/terraforme-mcp" ]; then
    ok "Já está na versão $version ($BIN_DIR/terraforme)"
    exit 0
fi

# Usa sudo só quando a pasta (ou o ancestral mais próximo que existe) não é gravável.
run() {
    dir="$BIN_DIR"
    while [ ! -e "$dir" ]; do dir=$(dirname "$dir"); done
    if [ -w "$dir" ]; then "$@"; else sudo "$@"; fi
}

asset="terraforme-$target.tar.gz"
url="https://github.com/$REPO/releases/download/$tag/$asset"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT INT TERM

info "Baixando terraforme $version ($target)…"
curl -fsSL -o "$tmp/$asset" "$url" || die "Falha ao baixar $url"
curl -fsSL -o "$tmp/$asset.sha256" "$url.sha256" || die "Falha ao baixar $url.sha256"
(cd "$tmp" && sha256sum -c "$asset.sha256" >/dev/null 2>&1) || die "O sha256 de $asset não confere."
tar -xzf "$tmp/$asset" -C "$tmp"

# Copia para um arquivo temporário e troca com `mv`: substituir no lugar falha
# ("Text file busy") se o binário estiver rodando.
info "Instalando em $BIN_DIR"
run mkdir -p "$BIN_DIR"
for bin in terraforme terraforme-mcp; do
    run install -m 755 "$tmp/$bin" "$BIN_DIR/$bin.new"
    run mv -f "$BIN_DIR/$bin.new" "$BIN_DIR/$bin"
done

if [ -n "$previous" ]; then
    ok "Atualizado: $previous → terraforme $version"
    printf '  novidades: https://github.com/%s/releases/tag/%s\n' "$REPO" "$tag"
else
    ok "Instalado: terraforme $version"
fi
if pgrep -x terraforme-mcp >/dev/null 2>&1; then
    printf '\033[1;33m!\033[0m terraforme-mcp antigo ainda rodando: feche os `terraforme cli` abertos (ou `pkill -x terraforme-mcp`).\n'
fi

case ":$PATH:" in
    *":$BIN_DIR:"*) ;;
    *)
        printf '\n\033[1;33m!\033[0m %s não está no PATH. Adicione ao ~/.bashrc (ou ~/.zshrc):\n' "$BIN_DIR"
        printf '    export PATH="%s:$PATH"\n' "$BIN_DIR"
        ;;
esac
[ -n "$previous" ] || printf '\nPróximo passo: terraforme setup\n'
