Six-player co-op Pokémon HeartGold, now **one program**: the emulator is built in, so there's no
BizHawk, .NET, Mono or Visual C++ to install. **Bring your own ROM:** GoldLink needs your own
unmodified Pokémon HeartGold (USA) ROM and builds the co-op game from it.

## Downloads

| File | For |
| :--- | :--- |
| `GoldLink-{{VERSION}}-windows.zip` | Windows 10 / 11, 64-bit |
| `GoldLink-{{VERSION}}-linux.tar.gz` | Linux, 64-bit |

Unpack, start `GoldLink` (`GoldLink.exe` on Windows), choose your ROM the first time, then
press **Play**. The `README.txt` inside says more. Coming from the BizHawk version: your worlds,
your place in them and your saves come over on the first start.

**Updating:** from v0.6.1 on, GoldLink tells you when a new version is out: press **Update** on
the screen with PLAY and it downloads, checks and installs it, then restarts. From v0.6 or older,
by hand: unpack the new version anywhere and start it. Your worlds, saves, settings and ROM live
in your GoldLink folder, not the program's, so nothing needs copying; delete the old version's
folder once the new one runs. On Linux, run the new folder's `./install.sh` again so the app
menu starts the new version.

Windows shows an "unknown publisher" warning for this unsigned program: **More info > Run anyway**.

## Compatibility

- **Who can play together.** This release speaks relay protocol **{{WIRE}}**, like v0.5: v0.5 and
  newer players share worlds on a `coop-relay` release for protocol {{WIRE}} (relay-v0.5), so
  nothing changes on the server. That relay refuses v0.3 and older (protocol 8): they can't join,
  and get a message saying which side is behind. Your worlds, saves and places in them carry over.
- GoldLink connects encrypted (QUIC) by default, on the relay's usual port (7777): nothing new
  to open. A relay run with `--no-quic` is played over plain UDP. The first connection to a
  world trusts the relay's certificate and keeps it with the world.
- The ROM it accepts: SHA-1 `{{ROM_SHA1}}`.

## Changes

{{CHANGES}}
