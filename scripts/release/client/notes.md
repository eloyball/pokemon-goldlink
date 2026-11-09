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

- Relay protocol **{{WIRE}}**: use a `coop-relay` release for protocol {{WIRE}}. Players of the
  last BizHawk release on the same protocol can share a world with this one.
- GoldLink connects encrypted (QUIC) by default: the relay listens on its port + 1 too (7778
  beside 7777), which its host must open. A relay run with `--no-quic` is played over plain UDP.
  The first connection to a world trusts the relay's certificate and keeps it with the world.
- The ROM it accepts: SHA-1 `{{ROM_SHA1}}`.

## Changes

{{CHANGES}}
