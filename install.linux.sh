#!/usr/bin/env bash
# Instala ou atualiza a CLI `terraforme` a partir deste repositório.
#
#   ./install.linux.sh                 # compila o código local e instala em ~/.local/bin
#   ./install.linux.sh --update        # git pull (fast-forward) antes de compilar
#   ./install.linux.sh --dist          # build otimizada (LTO, mais lenta para compilar)
#   ./install.linux.sh --restart       # reinicia o terraforme no fim sem perguntar (terraforme restart)
#   PREFIX=/usr/local ./install.linux.sh   # instala em /usr/local/bin (pede sudo)
#   ./install.linux.sh --uninstall     # remove
#
# `terraforme upgrade` roda este script com --update.
set -euo pipefail

BIN_NAME="terraforme"
PREFIX="${PREFIX:-$HOME/.local}"
BIN_DIR="$PREFIX/bin"
TARGET="$BIN_DIR/$BIN_NAME"
# Daemon MCP compartilhado (conversa entre instâncias, subagentes), iniciado
# pelo `terraforme cli` a partir do PATH.
MCP_TARGET="$BIN_DIR/terraforme-mcp"
# Janela flutuante com o mascote (`terraforme pet`).
PET_TARGET="$BIN_DIR/terraforme-pet"
REPO_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# Extensão do GNOME Shell que deixa a janela flutuante trazer o terminal para
# frente no Wayland (`platform::raise_terminal`).
GNOME_UUID="terraforme@augustowtg.github.io"
GNOME_EXT_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/gnome-shell/extensions/$GNOME_UUID"

info() { printf '\033[1;34m==>\033[0m %s\n' "$*"; }
ok()   { printf '\033[1;32m✔\033[0m %s\n' "$*"; }
warn() { printf '\033[1;33m!\033[0m %s\n' "$*" >&2; }
die()  { printf '\033[1;31m✘\033[0m %s\n' "$*" >&2; exit 1; }

usage() { sed -n '2,11s/^# \{0,1\}//p' "${BASH_SOURCE[0]}"; }

update=false
profile=release
uninstall=false
# 1 = reinicia sem perguntar, 0 = não reinicia (`terraforme upgrade --restart`
# passa 1); vazio = pergunta.
restart="${TERRAFORME_RESTART:-}"
for arg in "$@"; do
    case "$arg" in
        --update) update=true ;;
        --dist) profile=dist ;;
        --uninstall) uninstall=true ;;
        --restart) restart=1 ;;
        -h|--help) usage; exit 0 ;;
        *) usage >&2; die "Opção desconhecida: $arg" ;;
    esac
done

# Usa sudo só quando o diretório de destino (ou o ancestral mais próximo que
# já existe) não for gravável.
writable_dir() {
    local dir="$BIN_DIR"
    while [[ ! -e "$dir" ]]; do dir="$(dirname "$dir")"; done
    [[ -w "$dir" ]]
}

run() {
    if writable_dir; then
        "$@"
    else
        sudo "$@"
    fi
}

if $uninstall; then
    [[ -e "$TARGET" ]] || die "$TARGET não existe — nada a remover."
    run rm -f "$TARGET" "$MCP_TARGET" "$PET_TARGET"
    if [[ -d "$GNOME_EXT_DIR" ]]; then
        gnome-extensions disable "$GNOME_UUID" >/dev/null 2>&1 || true
        rm -rf "$GNOME_EXT_DIR"
    fi
    ok "Removido $TARGET"
    exit 0
fi

[[ "$(uname -s)" == "Linux" ]] || die "Este script é só para Linux."
command -v cargo >/dev/null || die "cargo não encontrado. Instale o Rust: https://rustup.rs"

if $update; then
    command -v git >/dev/null || die "git não encontrado."
    if ! git -C "$REPO_DIR" rev-parse --abbrev-ref --symbolic-full-name '@{u}' >/dev/null 2>&1; then
        warn "A branch atual não tem upstream — compilando o código local sem git pull."
    else
        info "Atualizando o repositório ($(git -C "$REPO_DIR" rev-parse --abbrev-ref HEAD))…"
        git -C "$REPO_DIR" pull --ff-only \
            || die "git pull falhou (alterações locais ou branch divergente). Resolva em $REPO_DIR e rode de novo."
    fi
fi

previous=""
if [[ -x "$TARGET" ]]; then
    previous="$("$TARGET" version 2>/dev/null || true)"
fi

info "Compilando $BIN_NAME (perfil $profile)…"
cargo build --profile "$profile" --locked -p terraforme-cli -p terraforme-mcp -p terraforme-pet --manifest-path "$REPO_DIR/Cargo.toml"

# Copia para um arquivo temporário e troca com `mv`: substituir o binário no
# lugar falha ("Text file busy") se houver um `terraforme` rodando.
info "Instalando em $TARGET"
run mkdir -p "$BIN_DIR"
run install -m 755 "$REPO_DIR/target/$profile/$BIN_NAME" "$TARGET.new"
run mv -f "$TARGET.new" "$TARGET"
run install -m 755 "$REPO_DIR/target/$profile/terraforme-mcp" "$MCP_TARGET.new"
run mv -f "$MCP_TARGET.new" "$MCP_TARGET"
run install -m 755 "$REPO_DIR/target/$profile/terraforme-pet" "$PET_TARGET.new"
run mv -f "$PET_TARGET.new" "$PET_TARGET"
current="$("$TARGET" version)"
if [[ "$previous" == "$current" ]]; then
    ok "Recompilado: $current (mesma versão)"
elif [[ -n "$previous" ]]; then
    ok "Atualizado: $previous → $current"
    # Seções do CHANGELOG.md mais novas que a versão anterior.
    if [[ -f "$REPO_DIR/CHANGELOG.md" ]]; then
        printf '\n'
        awk -v prev="${previous##* }" '
            /^## / { if ($2 == prev) exit; show = 1 }
            show
        ' "$REPO_DIR/CHANGELOG.md"
    fi
else
    ok "Instalado: $current"
fi

# Extensão do GNOME: copia e habilita. Uma extensão nova só é carregada pelo
# GNOME no Wayland depois de sair e entrar na sessão, então, se ele ainda não a
# conhece, ela é habilitada direto na lista do gsettings.
if command -v gnome-shell >/dev/null 2>&1; then
    mkdir -p "$GNOME_EXT_DIR"
    cp "$REPO_DIR/assets/gnome-shell/$GNOME_UUID/"* "$GNOME_EXT_DIR/"
    if ! gnome-extensions enable "$GNOME_UUID" >/dev/null 2>&1; then
        enabled="$(gsettings get org.gnome.shell enabled-extensions 2>/dev/null || true)"
        if [[ -n "$enabled" && "$enabled" != *"'$GNOME_UUID'"* ]]; then
            case "$enabled" in
                *"[]") enabled="['$GNOME_UUID']" ;;
                *) enabled="${enabled%]}, '$GNOME_UUID']" ;;
            esac
            gsettings set org.gnome.shell enabled-extensions "$enabled" || true
        fi
    fi
    if gdbus introspect --session --dest org.gnome.Shell \
        --object-path /org/gnome/Shell/Extensions/Terraforme 2>/dev/null | grep -q Terraforme; then
        ok "Extensão do GNOME ativa ($GNOME_UUID)"
    else
        warn "Extensão do GNOME instalada: saia e entre na sessão para a janela flutuante trazer o terminal para frente."
    fi
fi

# Daemon MCP ou janela flutuante da versão antiga no ar: reinicia com os
# binários novos (os `terraforme cli` abertos se reconectam sozinhos).
if pgrep -x terraforme-mcp >/dev/null 2>&1 || pgrep -x terraforme-pet >/dev/null 2>&1; then
    if [[ -z "$restart" ]] && (: </dev/tty) 2>/dev/null; then
        printf '\n'
        read -r -p "Reiniciar o terraforme agora (daemon MCP e janela flutuante)? [S/n] " answer </dev/tty || answer=n
        [[ "$answer" =~ ^[NnTt] ]] && restart=0 || restart=1
    fi
    if [[ "$restart" == 1 ]]; then
        "$TARGET" restart || warn "Falha ao reiniciar — rode \`terraforme restart\`."
    else
        warn "terraforme-mcp antigo ainda rodando: rode \`terraforme restart\` para subir o novo."
    fi
fi

case ":$PATH:" in
    *":$BIN_DIR:"*) ;;
    *)
        printf '\n\033[1;33m!\033[0m %s não está no PATH. Adicione ao ~/.bashrc:\n' "$BIN_DIR"
        printf '    export PATH="%s:$PATH"\n' "$BIN_DIR"
        ;;
esac
