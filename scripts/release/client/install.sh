#!/bin/sh
# Adds GoldLink to your desktop's app menu, with its icon. Run it from this folder again if you
# move the folder. `./install.sh --uninstall` takes it out of the menu again (your saves stay in
# ~/.local/share/goldlink).
set -e
here=$(cd "$(dirname "$0")" && pwd)
apps="${XDG_DATA_HOME:-$HOME/.local/share}/applications"
entry="$apps/goldlink.desktop"
if [ "$1" = --uninstall ]; then
    rm -f "$entry"
    echo "GoldLink is out of your app menu."
    exit 0
fi
if [ ! -x "$here/GoldLink" ]; then
    echo "install.sh: run it from GoldLink's folder (GoldLink isn't beside it)" >&2
    exit 1
fi
case "$here" in
    *[\\\"\`\$]*)
        echo "install.sh: GoldLink's folder path has a \\, \", \` or \$ in it, which app menus can't run;" >&2
        echo "move the folder somewhere else (like ~/Games/GoldLink) and run install.sh again" >&2
        exit 1 ;;
esac
mkdir -p "$apps"
cat > "$entry.tmp" <<DESKTOP
[Desktop Entry]
Type=Application
Name=GoldLink
Comment=Six-player co-op Pokémon HeartGold
Exec="$here/GoldLink"
Path=$here
Icon=$here/goldlink.png
Terminal=false
Categories=Game;
StartupWMClass=goldlink
DESKTOP
mv "$entry.tmp" "$entry"
command -v update-desktop-database >/dev/null && update-desktop-database "$apps" 2>/dev/null || true
echo "GoldLink is in your app menu (Games)."
