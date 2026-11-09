GoldLink {{VERSION}}: six-player co-op Pokémon HeartGold
==========================================================

What you need
-------------
- Your own Pokémon HeartGold (USA) ROM, unmodified (an .nds file). GoldLink doesn't include
  the game; it checks your copy and makes the co-op version from it.
- Windows 10 or 11, 64-bit, or Linux, 64-bit. Nothing else to install: the emulator is built
  into GoldLink.
- The address of the server your group uses (host:port; ask whoever runs it) and your world
  code. GoldLink connects encrypted: the first time, it learns the server's identity and from
  then on accepts only that server for the world. If the server's identity ever changes,
  GoldLink stops and asks you before trusting the new one (only do so if the host says they
  replaced it).

Starting
--------
1. Unpack this folder anywhere (keep everything in it together).
2. Start GoldLink: GoldLink.exe on Windows, ./GoldLink on Linux.
   Windows may warn about an unknown publisher: choose "More info", then "Run anyway".
3. The first time, choose your HeartGold ROM. GoldLink checks it and keeps a copy.
4. Press Play. The game opens in the same window with the World panel: pick your avatar,
   enter the server address and the world code, and join (next time, pick the world from
   "Played before"). Esc opens the settings; the World panel is there too.

Controls
--------
A = X or K, B = Z or J, X = V, Y = C, L = Q, R = E, Start = Enter, Select = Space,
the arrows or WASD, the mouse on the bottom screen. Gamepads work too (Xbox layout).
Change them in goldlink.toml in your GoldLink folder. The game always runs at the DS's
speed: there is no fast-forward.

Your files
----------
Saves, settings and your ROM's copy live in your GoldLink folder, not in this one:
  Windows: %LOCALAPPDATA%\GoldLink
  Linux:   ~/.local/share/goldlink
If you played with the BizHawk version of GoldLink, the first start brings over your
worlds, your place in them and your saves (its folder is left as it was).

If GoldLink closes unexpectedly it saves a report in that folder (crashes/); pass it on to
whoever helps your group.

Licenses
--------
GoldLink's program is free software under the GNU GPL, version 3 or later (it contains the
melonDS emulator). Its source is at the address in LICENSES/SOURCE.txt. See LICENSES/.
