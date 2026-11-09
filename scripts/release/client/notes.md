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

Windows shows an "unknown publisher" warning for this unsigned program: **More info > Run anyway**.

## Compatibility

- **Everyone updates together.** This release speaks relay protocol **{{WIRE}}**: it needs a
  `coop-relay` release for protocol {{WIRE}} (relay-v0.5), and a relay for protocol {{WIRE}} refuses
  older GoldLink releases (v0.3 speaks protocol 8): they can't join, and get a message saying
  which side is behind. Update the relay and every player at the same time. Your worlds, saves
  and places in them carry over.
- GoldLink connects encrypted (QUIC) by default, on the relay's usual port (7777): nothing new
  to open. A relay run with `--no-quic` is played over plain UDP. The first connection to a
  world trusts the relay's certificate and keeps it with the world.
- The ROM it accepts: SHA-1 `{{ROM_SHA1}}`.

## Changes

{{CHANGES}}
