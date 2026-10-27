#!/usr/bin/env bash
# Instala o terraforme direto do GitHub, sem clonar o repositório:
#   curl -fsSL https://raw.githubusercontent.com/Augustowtg/terraforme/main/install.sh | bash
# Baixa o código da branch (ou tag) em TERRAFORME_REF (padrão: main) e roda o
# instalador do sistema (install-linux.sh ou install-win.sh). Os argumentos,
# como -y, passam para ele: `... | bash -s -- -y`.
set -euo pipefail

REPO="Augustowtg/terraforme"
REF="${TERRAFORME_REF:-main}"

if ! command -v curl >/dev/null 2>&1; then
  echo "erro: 'curl' não encontrado." >&2
  exit 1
fi

case "$(uname -s)" in
  Linux*) installer="install-linux.sh" ;;
  MINGW* | MSYS* | CYGWIN*) installer="install-win.sh" ;;
  *)
    echo "erro: sistema não suportado: $(uname -s)." >&2
    exit 1
    ;;
esac

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

echo "Baixando o terraforme ($REF) de github.com/$REPO..."
curl -fsSL "https://codeload.github.com/$REPO/tar.gz/$REF" | tar -xz -C "$tmp" --strip-components=1

# Com `curl | bash`, a entrada é o pipe: o instalador lê as teclas do terminal.
if [ -r /dev/tty ] && { : </dev/tty; } 2>/dev/null; then
  bash "$tmp/$installer" "$@" </dev/tty
else
  bash "$tmp/$installer" "$@"
fi
