#!/bin/sh
# Aria installer for Linux.
# Usage: curl -fsSL https://raw.githubusercontent.com/laopro/aria/main/install.sh | sh
set -eu

REPO="laopro/aria"
BIN="aria"
PREFIX="${ARIA_PREFIX:-$HOME/.local/bin}"

say() { printf '%s\n' "$*"; }
die() { printf 'error: %s\n' "$*" >&2; exit 1; }

[ "$(uname -s)" = "Linux" ] || die "this installer only supports Linux (use the release binaries elsewhere)"
[ "$(uname -m)" = "x86_64" ] || die "only x86_64 builds are published right now"

command -v curl >/dev/null 2>&1 || die "curl is required"
command -v tar >/dev/null 2>&1 || die "tar is required"

say "Fetching the latest Aria release..."
URL=$(curl -fsSL "https://api.github.com/repos/$REPO/releases/latest" |
    grep -o '"browser_download_url": *"[^"]*aria-linux-x86_64.tar.gz"' |
    head -n 1 | sed 's/.*"\(https[^"]*\)"/\1/')
[ -n "$URL" ] || die "could not find a release asset (has a release been published?)"

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT
curl -fsSL "$URL" -o "$TMP/aria.tar.gz" || die "download failed"
tar -xzf "$TMP/aria.tar.gz" -C "$TMP" || die "could not extract the archive"

mkdir -p "$PREFIX"
install -m 755 "$TMP/aria-linux-x86_64" "$PREFIX/$BIN"

say "Installed $PREFIX/$BIN"

case ":$PATH:" in
*":$PREFIX:"*) ;;
*)
    LINE="export PATH=\"$PREFIX:\$PATH\""
    for RC in "$HOME/.bashrc" "$HOME/.zshrc" "$HOME/.profile"; do
        [ -f "$RC" ] || continue
        grep -qsF "$PREFIX" "$RC" || printf '\n# Added by the Aria installer\n%s\n' "$LINE" >>"$RC"
    done
    say "Added $PREFIX to PATH in your shell profile. Open a new terminal."
    ;;
esac

# Aria plays through yt-dlp: fetch it too when it is missing or broken (a pip
# install stops working when the distro moves to a new Python version).
if ! yt-dlp --version >/dev/null 2>&1 && ! "$PREFIX/yt-dlp" --version >/dev/null 2>&1; then
    if [ -e "$PREFIX/yt-dlp" ]; then
        mv "$PREFIX/yt-dlp" "$PREFIX/yt-dlp.broken"
    fi
    say "Fetching yt-dlp (needed to play music)..."
    # The plain script starts faster but needs Python; the standalone build does not.
    if command -v python3 >/dev/null 2>&1; then
        YTDLP_ASSET="yt-dlp"
    else
        YTDLP_ASSET="yt-dlp_linux"
    fi
    if curl -fsSL "https://github.com/yt-dlp/yt-dlp/releases/latest/download/$YTDLP_ASSET" -o "$TMP/yt-dlp"; then
        install -m 755 "$TMP/yt-dlp" "$PREFIX/yt-dlp"
        say "Installed $PREFIX/yt-dlp"
    else
        say "Could not download yt-dlp. Install it from https://github.com/yt-dlp/yt-dlp so Aria can play music."
    fi
fi

say "Done. Run: aria"
