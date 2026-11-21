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
- **A server for your group.** GoldLink players meet on a server called the relay,
  **GoldLink-RelayServer**, and there's no public one: to play together, someone in your group
  has to run it on a computer or a VPS (a small rented server). It's free and has its own release
  on the [releases page](https://github.com/eloyball/pokemon-goldlink/releases); see
  [Hosting a server](#hosting-a-server). Everyone else just needs its address and the world code.

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

### Your first game

1. Start GoldLink. The first time, choose your HeartGold ROM.
2. Press **PLAY**.
3. One friend creates a world: **Create a world**, enter the server address and a world code,
   choose the number of players and the rules, then **Create world**.
4. Everyone else picks **Join a world** and enters the same server address and world code.
5. The game starts in the window. In a new world, play through the intro: the adventure begins
   once every player is ready in their bedroom.

Everyone in a world needs the same GoldLink version: the **Update** button keeps you there.

### The launcher, explained

- **Server address:** where your group's server (the relay) runs, written like
  `203.0.113.5:7777` or `myserver.example.com:7777`. The person hosting gives it to you (see
  [Hosting a server](#hosting-a-server)).
- **World code:** six letters or numbers that name your group's adventure (a "world"). The
  creator picks any code and shares it; everyone joins with the same server address and code.
  One server can hold many worlds.
- **Continue:** back to the world you played last, in one click.
- **Join a world:** enter a world code and server address, or pick a world you've played before.
  **Forget** removes a world from the list; its save stays on your computer.
- **Create a world:** you choose the code, the number of players (2 to 6) and the challenge rules
  below. Once everyone is in, the world locks to those players and its rules can't change.
- **YOU PLAY AS:** your avatar, the character your friends see you as.
- **Restore a backup:** while you play, GoldLink takes a backup of your game every few minutes.
  If something goes wrong (a crash, a bad save), pick one to put your game back to that moment.
  GoldLink asks before overwriting your save.
- **Import a save:** brings your worlds over from another computer or an older GoldLink: choose
  that GoldLink's folder. Nothing you already have is replaced.
- **Settings:**
  - **Screens:** both DS screens top and bottom, side by side, or **one screen**, where the
    touch screen pops up only when the game needs it (Tab shows or hides it).
  - **Widescreen** fills a wide monitor in the overworld and in battles (one-screen mode).
  - **Sharp pixels** or smooth, and **whole-number scaling** for perfectly even pixels.
  - **Revamped lighting** (light and shade, day and night) with **dynamic shadows**.
  - **Smooth camera:** the camera glides as you walk instead of shimmering.
  - Volume, and **Open your GoldLink folder** (your saves, settings and backups).

### In the game

- **Controls:** arrows or WASD to move, A = X or K, B = Z or J, X = V, Y = C, L = Q, R = E,
  Start = Enter, Select = Space. The mouse works on the touch screen, and gamepads work too.
- **Tab:** shows or hides the touch screen in one-screen mode.
- **Esc:** the menu, with settings, the world's details and **Leave world**. Leave saves your game
  and takes you back to the launcher. The next time you join, you pick up where you left off.
- **F3:** the world's details: who's online, the rules, and where everyone is.
- **F11:** fullscreen.
- **Gym gate:** you can't challenge a gym leader while a teammate is behind on badges, so the
  group moves forward together. A player who's behind can tick **Let teammates challenge bosses
  while I'm behind** in the Esc menu to let the others go ahead.

### Challenge rules

The world's creator picks these; they can be combined.

- **Random encounters:** wild Pokémon, starters and gifts are shuffled.
  - **Semi-random:** each route gets new species, the same for everyone in the world.
  - **Preserve difficulty:** the same, but each replacement is about as strong as the original.
  - **Fully random:** any species, rolled anew on every encounter.
  - **Legends in the wild:** legendary Pokémon can show up among wild Pokémon too.
  - **Random abilities:** each randomized wild Pokémon gets a random ability.
- **Random items:** the item balls on the ground hold random items.
- **Nuzlocke:**
  - You can only catch the first Pokémon you meet in each area, and you must nickname it.
  - A Pokémon that faints is gone for good: it moves to a Graveyard box in the PC.
  - If all your Pokémon faint, you become a ghost: you roam freely but can't battle or interact,
    X takes you to a teammate, and meeting a teammate brings you back. The run is lost only when
    every player is a ghost.
- **Soul-Link** (with Nuzlocke): Pokémon caught in the same area are linked, and when one dies its
  linked partners die too. **World-wide** links everyone's catches; **Duo** links only yours and
  your teammate's.
- **Monolocke:** every player gets their own type and can only catch (and receive) Pokémon of that
  type, starters included.
- **Level cap:** your Pokémon stop gaining experience at the level of the next gym leader's
  strongest Pokémon.

### If something goes wrong

Your GoldLink folder (`%LOCALAPPDATA%\GoldLink` on Windows, `~/.local/share/goldlink` on Linux)
has a log (`goldlink.log`, and `goldlink.old.log` from the run before) and a `crashes` folder.
Please attach them when you [report a problem](https://github.com/eloyball/pokemon-goldlink/issues).
If the game stops responding, press **Leave**, then **Restore a backup** with the newest one.

## Hosting a server

To play together, your group needs a server, called the relay (**GoldLink-RelayServer**):
everyone's games meet there, and GoldLink has no public one. One person runs it, on a computer that stays on while you play or on
a VPS (a small rented server). The relay is its own release, separate from GoldLink: a small
program for Linux, Windows or a Raspberry Pi that uses one network port and a few MB of memory.
Players don't need it; only the person hosting does.

1. Download the relay for your system from the
   [releases page](https://github.com/eloyball/pokemon-goldlink/releases): the newest
   **GoldLink RelayServer** release (`GoldLink-RelayServer-…-windows.zip`,
   `…-linux-x86_64.tar.gz`, or `…-linux-aarch64.tar.gz` for a Raspberry Pi).
2. Unpack it and start `GoldLink-RelayServer` (`GoldLink-RelayServer.exe` on Windows; allow it
   through the firewall when Windows asks). Leave it running while your group plays.
3. Make it reachable for your friends on **UDP port 7777**:
   - **At home:** forward UDP port 7777 on your router to the computer running the relay. If you
     can't, a VPN such as Tailscale or ZeroTier works too: everyone joins it and uses the relay
     computer's VPN address.
   - **On a VPS:** open UDP port 7777 in the provider's firewall.
4. Give your friends the server address: your public IP address (or the VPN address) followed
   by `:7777`.

Keep the relay's folder: your worlds' progress and the relay's identity live in it. The relay's
own `README.md` covers running it as a background service, Docker and its options. Connections
are encrypted, and GoldLink remembers each world's relay, so a stranger's server can't pose as
yours.

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
- `coop-relay/`: the relay server (released as GoldLink-RelayServer).
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
