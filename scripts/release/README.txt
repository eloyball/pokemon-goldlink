GoldLink {{VERSION}}: six-player co-op Pokémon HeartGold
==========================================================

What you need
-------------
- Your own Pokémon HeartGold (USA) ROM, unmodified (an .nds file). GoldLink doesn't include
  the game; it checks your copy and makes the co-op version from it.
- Windows 10 or 11, 64-bit. BizHawk also needs the Microsoft Visual C++ Redistributable
  (x64); GoldLink tells you if it's missing.
- Or Linux, 64-bit, with Mono installed (Arch: sudo pacman -S mono; Debian / Ubuntu:
  sudo apt install mono-complete; Fedora: sudo dnf install mono-complete).
- The address of the server your group uses (host:port; ask whoever runs it) and your
  world code.

Starting
--------
1. Unpack this folder anywhere (keep everything in it together).
2. Start GoldLink: GoldLink.exe on Windows, ./GoldLink on Linux.
   Windows may warn about an unknown publisher: choose "More info", then "Run anyway".
3. The first time, choose your HeartGold ROM. GoldLink checks it, keeps a copy and builds
   the co-op game (a few seconds).
4. Press Play. BizHawk opens with the Co-op Bridge window: pick your avatar, enter the server
   address and the world code, and join (next time, pick the world from the list).

Your files
----------
Saves, settings and the co-op game live in your GoldLink folder, not in this one:
  Windows: %LOCALAPPDATA%\GoldLink
  Linux:   ~/.local/share/goldlink
A new GoldLink release goes into a new folder; your saves stay where they are, and the new
release updates the game the first time you start it. Settings > "Open your GoldLink folder"
opens it.

Everyone in a world must use the same GoldLink release ({{VERSION}}). This release speaks relay
protocol {{WIRE}}: the server needs a coop-relay release for that protocol.

Controls are BizHawk's (Config > Controllers). Settings > "Reset BizHawk settings" brings back
GoldLink's.

GoldLink is free, and must stay free: CC BY-NC-SA 4.0, on hg-engine's terms (no money
involved, donations included). See LICENSES/NOTICE.md; the people it builds on are in
CREDITS.md. Pokémon belongs to Nintendo, Creatures Inc. and GAME FREAK inc.; this is an
unofficial fan project.

Project page: https://github.com/{{REPOSITORY}}
