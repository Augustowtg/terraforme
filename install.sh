#!/usr/bin/env bash
# Instala o terraforme direto do GitHub, sem clonar o repositório:
#   curl -fsSL https://raw.githubusercontent.com/Augustowtg/terraforme/main/install.sh | bash
# Baixa o código da branch (ou tag) em TERRAFORME_REF (padrão: a última
# release) e roda o instalador do sistema (install-linux.sh ou
# install-win.sh). Numa tag com release, baixa também o binário compilado,
# e o instalador não precisa do Rust. Os argumentos, como -y, passam para
# ele: `... | bash -s -- -y`.
set -euo pipefail

REPO="Augustowtg/terraforme"

if ! command -v curl >/dev/null 2>&1; then
  echo "erro: 'curl' não encontrado." >&2
  exit 1
fi

case "$(uname -s)-$(uname -m)" in
  Linux-x86_64) installer="install-linux.sh" asset="terra-x86_64-unknown-linux-gnu" ;;
  Linux-*) installer="install-linux.sh" asset="" ;;
  MINGW*-x86_64 | MSYS*-x86_64 | CYGWIN*-x86_64) installer="install-win.sh" asset="terra-x86_64-pc-windows-msvc.exe" ;;
  MINGW* | MSYS* | CYGWIN*) installer="install-win.sh" asset="" ;;
  *)
    echo "erro: sistema não suportado: $(uname -s)." >&2
    exit 1
    ;;
esac

# A última release: o /releases/latest redireciona para /releases/tag/<tag>.
# Sem release, fica a main.
latest_release() {
  local url
  url="$(curl -fsSLI -o /dev/null -w '%{url_effective}' "https://github.com/$REPO/releases/latest" 2>/dev/null || true)"
  case "$url" in
    */releases/tag/*) echo "${url##*/}" ;;
    *) echo "main" ;;
  esac
}

REF="${TERRAFORME_REF:-$(latest_release)}"

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

echo "Baixando o terraforme ($REF) de github.com/$REPO..."
curl -fsSL "https://codeload.github.com/$REPO/tar.gz/$REF" | tar -xz -C "$tmp" --strip-components=1

# O binário compilado da release, se a tag tiver um para este sistema; sem
# ele, o instalador compila com o cargo o código baixado (--dev).
if [ -n "$asset" ] && curl -fsSL -o "$tmp/$asset" "https://github.com/$REPO/releases/download/$REF/$asset" 2>/dev/null; then
  export TERRAFORME_BIN="$tmp/$asset"
  echo "Usando o binário compilado da release $REF."
else
  [ -z "$asset" ] || rm -f "$tmp/$asset"
  set -- "$@" --dev
fi

# Com `curl | bash`, a entrada é o pipe: o instalador lê as teclas do terminal.
if [ -r /dev/tty ] && { : </dev/tty; } 2>/dev/null; then
  bash "$tmp/$installer" "$@" </dev/tty
else
  bash "$tmp/$installer" "$@"
fi
