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
- **A server for your group.** One person runs the relay (below); everyone else just needs its
  address and the world code.

## Installing

GoldLink has no installer: it runs from its own folder. Put that folder somewhere you can write
to (your home folder, not `Program Files`), because the **Update** button installs new versions
into it.

### Windows

1. From the [latest release](https://github.com/eloyball/pokemon-goldlink/releases/latest),
   download `GoldLink-vX.Y-windows.zip`.
2. Right-click the zip, choose **Extract All…**, and extract it to a folder of yours, for example
   `C:\Users\<you>\Games`. That gives you a `GoldLink-vX.Y` folder; you can rename it to
   `GoldLink`.
3. Open the folder and double-click `GoldLink.exe`. If Windows says it protected your PC (an
   unknown publisher), choose **More info**, then **Run anyway**.
4. For the Start menu: right-click `GoldLink.exe` and choose **Pin to Start** (on Windows 11 it
   may be under **Show more options**). For the desktop: **Send to → Desktop (create
   shortcut)**.

### Linux

In a terminal, set `V` to the version you're installing (the latest is shown on the
[releases page](https://github.com/eloyball/pokemon-goldlink/releases/latest)):

```sh
V=v0.6.1
cd ~/Downloads
curl -LO https://github.com/eloyball/pokemon-goldlink/releases/download/$V/GoldLink-$V-linux.tar.gz
mkdir -p ~/Games
tar xzf GoldLink-$V-linux.tar.gz -C ~/Games
mv ~/Games/GoldLink-$V ~/Games/GoldLink
~/Games/GoldLink/install.sh
```

`install.sh` adds GoldLink to your app menu (under Games), with its icon. Start it from there, or
with `~/Games/GoldLink/GoldLink`. If you move the folder, run `install.sh` again from its new
place. Fish shell users: write `set V v0.6.1` instead of `V=v0.6.1`.

To uninstall: `~/Games/GoldLink/install.sh --uninstall`, then delete `~/Games/GoldLink`. Your
saves stay in `~/.local/share/goldlink`.

### Updating

From v0.6.1 on, the launcher tells you when a new version is out: press **Update to vX.Y** and
it downloads it, checks its signature, installs it in the same folder and restarts. The Start
menu pin and the app menu entry keep working. From v0.6 or older, update by hand once: install
v0.6.1 as above, then delete the old folder (your saves aren't in it).

## Playing

1. Start GoldLink.
2. The first time, choose your HeartGold ROM.
3. Press **PLAY**, pick your avatar, then **Create a world** (and share its code) or
   **Join a world** with a code and the server address your host gave you.
4. The game takes over the window. The bar at the top shows who's online; **Leave** saves your
   game and brings the launcher back.

Everyone in a world needs the same GoldLink release. Saves stay on each player's computer, and
**Import a save** brings your worlds over from another computer or an older GoldLink.

## Hosting a server

The relay (`coop-relay`) is a small program any Linux machine, Windows PC, Raspberry Pi or cheap
VPS can run. It needs one UDP port (7777) and a few MB of memory. Get it from the
[releases](https://github.com/eloyball/pokemon-goldlink/releases) (`coop-relay-…` files): its
`README.md` covers running it, opening the port and running it as a service. Connections are
encrypted; the first time a player joins a world, GoldLink remembers the relay's identity.

## Building from source

```
git clone --recurse-submodules https://github.com/eloyball/pokemon-goldlink
cd pokemon-goldlink
cargo build --release -p goldlink                              # the program
cargo build --release --manifest-path coop-relay/Cargo.toml    # the relay
```

The program needs Rust, CMake and a C++17 compiler (on Linux also the ALSA and udev development
headers: `libasound2-dev libudev-dev` on Debian and Ubuntu). The co-op code inside the game lives
in `hg-engine/`; `./build_all.sh` builds the ROM from it (it needs an `arm-none-eabi` toolchain
and your ROM as `hg-engine/rom.nds`). `scripts/package_client.sh` makes the release packages.

## Licenses and credits

The GoldLink program contains melonDS and is free software under the **GNU GPL, version 3 or
later**. GoldLink's other own work (the co-op code in `hg-engine/`, the relay, the scripts) is
under **CC BY-NC-SA 4.0**; [NOTICE.md](NOTICE.md) has the details, and [CREDITS.md](CREDITS.md)
thanks everyone GoldLink builds on.

Pokémon and Pokémon HeartGold are trademarks of Nintendo, Creatures Inc. and GAME FREAK inc.
GoldLink is an unofficial fan project, not affiliated with or endorsed by them, and contains no
ROM.
