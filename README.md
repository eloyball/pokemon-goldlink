# GoldLink

**Six-player co-op Pokémon HeartGold.** Up to six friends play one adventure together, each on
their own game: you see each other walking around Johto, trade, battle side by side and against
each other, and progress as a team.

GoldLink is a fan-made mod built on [hg-engine](https://github.com/BluRosie/hg-engine), with the
melonDS emulator built in: one program, nothing else to install.

**[Download the latest release](https://github.com/eloyball/pokemon-goldlink/releases/latest)**
(Windows 10/11 and Linux, 64-bit)

## What you need

- **Your own Pokémon HeartGold (USA) ROM**, unmodified. GoldLink doesn't include the game: it
  checks your copy and builds the co-op version from it.
- **A server for your group.** One person runs the relay ([Hosting a server](#hosting-a-server));
  everyone else just needs its address and the world code.

## Installing

Download the installer for your system from the
[latest release](https://github.com/eloyball/pokemon-goldlink/releases/latest) and run it. It
fetches the newest GoldLink, checks its signature and installs it for you. No administrator or
root password is needed.

### Windows

1. Download `GoldLink-Setup.exe` and run it. If Windows says it protected your PC (an unknown
   publisher), choose **More info**, then **Run anyway**.
2. GoldLink goes in your Start menu, and on the desktop if you like.

To uninstall, use Windows' **Installed apps** list. Your saves are kept apart (in
`%LOCALAPPDATA%\GoldLink`) and stay when you uninstall.

### Linux

1. Download `GoldLink-Setup-linux`, make it executable (`chmod +x GoldLink-Setup-linux`, or in the
   file's properties), and run it.
2. GoldLink is installed in `~/.local/opt/GoldLink`. It goes in your app menu (under Games), and
   on the desktop if you like.

To uninstall, right-click GoldLink's menu entry and choose **Uninstall GoldLink**. Your saves stay
in `~/.local/share/goldlink`.

### Without the installer

Each release also has the program on its own: `GoldLink-vX.Y.Z-windows.zip` and
`GoldLink-vX.Y.Z-linux.tar.gz`. Unpack it into a folder you can write to (your home folder, not
`Program Files`), because updates install into that folder. Then start `GoldLink.exe` (Windows)
or `GoldLink` (Linux). On Linux, `./install.sh` in that folder adds it to your app menu
(`./install.sh --uninstall` removes it).

### Updating

GoldLink tells you when a new version is out: press **Update** on the screen with PLAY. It
downloads the new version, checks its signature, installs it and restarts. Your worlds, saves
and settings stay where they are. Versions before v0.6.1 can't do this: install the new version as
above, then delete the old folder (your saves aren't in it).

## Playing

1. Start GoldLink.
2. The first time, choose your HeartGold ROM.
3. Press **PLAY**, pick your avatar, then **Create a world** (and share its code) or
   **Join a world** with a code and the server address your host gave you.
4. The game takes over the window. The bar at the top shows who's online; **Leave** saves your
   game and brings the launcher back.

Everyone in a world needs the same GoldLink release. Saves stay on each player's computer, and
**Import a save** brings your worlds over from another computer or an older GoldLink.

If something goes wrong, your GoldLink folder (`%LOCALAPPDATA%\GoldLink` or
`~/.local/share/goldlink`) has a log (`goldlink.log`, the run before in `goldlink.old.log`) and a
`crashes` folder: please attach them when you report a problem.

## Hosting a server

The relay (`coop-relay`) is a small program any Linux machine, Windows PC, Raspberry Pi or cheap
VPS can run. It needs one UDP port (7777) and a few MB of memory. Get it from the
[releases](https://github.com/eloyball/pokemon-goldlink/releases) (`coop-relay-…` files): its
`README.md` covers running it, opening the port and running it as a service. Connections are
encrypted; the first time a player joins a world, GoldLink remembers the relay's identity.

## Contributing

GoldLink is open source, and contributions are welcome: bug reports, fixes and new ideas. Open an
[issue](https://github.com/eloyball/pokemon-goldlink/issues) to report a problem or talk an idea
through before a large change, then send a pull request.

### How it fits together

- `hg-engine/`: the game itself, hg-engine with GoldLink's co-op code (C, built into the ROM).
- `crates/`: the program, in Rust.
  - `goldlink`: the launcher and game window.
  - `goldlink-core`: melonDS, built in.
  - `goldlink-bridge`: the link between the game and the relay.
  - `goldlink-light`: the lighting.
  - `goldlink-setup` and `goldlink-update`: the installers and the Update button.
  - `goldlink-dev`: development tools.
- `coop-relay/`: the server.
- `scripts/`: building and packaging.

### Building from source

```
git clone --recurse-submodules https://github.com/eloyball/pokemon-goldlink
cd pokemon-goldlink
cargo build --release -p goldlink                              # the program
cargo build --release --manifest-path coop-relay/Cargo.toml    # the relay
cargo test                                                     # the tests
```

The program needs Rust, CMake and a C++17 compiler. On Linux it also needs the ALSA and udev
development headers (`libasound2-dev libudev-dev` on Debian and Ubuntu).

`./build_all.sh` builds the ROM from `hg-engine/`. It needs an `arm-none-eabi` toolchain and your
ROM as `hg-engine/rom.nds`, which is never committed. `scripts/package_client.sh` makes the
release packages.

## Licenses and credits

The GoldLink program contains melonDS and is free software under the **GNU GPL, version 3 or
later**. GoldLink's other own work (the co-op code in `hg-engine/`, the relay, the scripts) is
under **CC BY-NC-SA 4.0**; [NOTICE.md](NOTICE.md) has the details, and [CREDITS.md](CREDITS.md)
thanks everyone GoldLink builds on.

Pokémon and Pokémon HeartGold are trademarks of Nintendo, Creatures Inc. and GAME FREAK inc.
GoldLink is an unofficial fan project, not affiliated with or endorsed by them, and contains no
ROM.
