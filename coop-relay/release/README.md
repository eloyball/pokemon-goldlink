# coop-relay {{VERSION}}: the GoldLink server

The relay is the server GoldLink players meet on: up to six players per room, each room a
campaign with its own roster and progress. It speaks **relay protocol {{WIRE}}**: it serves the
GoldLink player releases for protocol {{WIRE}} (their release notes and `README.txt` say which
protocol they need). One relay serves any number of rooms (up to `--max-rooms`).

It needs very little: one UDP port, about 22 KB/s per player each way, a few MB of memory, no
database. Anything from a Raspberry Pi to the smallest VPS will do.

## Quick start

Linux (`coop-relay-{{VERSION}}-linux-x86_64.tar.gz`, or `-linux-aarch64` for ARM: Raspberry Pi,
Oracle Cloud's free Ampere machines):

```
tar xzf coop-relay-{{VERSION}}-linux-x86_64.tar.gz
cd coop-relay-{{VERSION}}
./coop-relay
```

Windows (`coop-relay-{{VERSION}}-windows.zip`): unzip, open a terminal in the folder, run
`coop-relay.exe`, and allow it through Windows Firewall when Windows asks (private networks,
or public too if players come from the internet).

It listens on UDP port **7777** on every address and keeps the campaigns in `campaigns/` next to
where it was started. Players enter `<your address>:7777` as the server address in the Co-op
Bridge. Stop it with Ctrl+C.

The binaries are static: no runtime or library to install, on any 64-bit Linux distribution.

## Reaching it from the internet

- **A VPS:** open UDP port 7777 in the provider's firewall (security group / security list) and
  in the machine's own (`sudo ufw allow 7777/udp`, or `firewalld`: `sudo firewall-cmd
  --permanent --add-port=7777/udp && sudo firewall-cmd --reload`).
- **At home:** forward UDP 7777 on your router to the machine running the relay, and give
  players your public address. If that's not possible, a VPN such as Tailscale or ZeroTier lets
  everyone use the relay machine's VPN address instead.
- It's UDP only: a TCP-only port forward or tunnel doesn't work.

## Linux server: systemd

```
sudo cp coop-relay /usr/local/bin/
sudo cp coop-relay.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now coop-relay
journalctl -u coop-relay -f        # the log
```

It starts at boot and restarts after a crash, runs as its own unprivileged user, and keeps the
campaigns in `/var/lib/coop-relay/campaigns`. Change the options on its `ExecStart` line
(`sudo systemctl edit --full coop-relay`).

## Docker

The `Dockerfile` packs the binary beside it into a small Alpine image:

```
docker build -t coop-relay .
docker run -d --name coop-relay --restart unless-stopped -p 7777:7777/udp -v coop-relay:/data coop-relay
docker logs -f coop-relay
```

Campaigns live in the `coop-relay` volume (`/data`). The aarch64 archive builds an ARM image the
same way.

## Options

| Option | Default | |
| :--- | :--- | :--- |
| `--bind <address:port>` | `0.0.0.0:7777` | where it listens |
| `--save-dir <folder>` | `campaigns` | the campaign files |
| `--min-players <n>` | `2` | a room's party size when its creator doesn't set one (2-6) |
| `--tick-rate <Hz>` | `60` | how often it sends each player the others' state |
| `--client-timeout-secs <s>` | `5` | silence before a player's slot is freed |
| `--room-idle-secs <s>` | `120` | how long an empty room stays in memory |
| `--max-rooms <n>` | `256` | rooms in memory at once |
| `--inspect-all` | off | log every packet (debugging) |
| `--version`, `--help` | | |

The log level comes from `RUST_LOG` (`info` by default; `RUST_LOG=debug` says more).

## Campaigns and backups

Each room's campaign (its roster, rules, progress) is one file, `<save-dir>/<ROOM>.json`,
written as soon as it changes (through a temporary file, so a crash never leaves half a file).
To back up a relay, copy that folder; to move it, copy the folder to the new machine. The
players' own saves stay on their machines (the Co-op Bridge keeps them per room).

Deleting a room's file forgets that campaign: the next group to use the code starts afresh.

## Updating

Stop the relay, replace the binary (or rebuild the image), start it again: the campaigns stay.
A relay release for another protocol version only works with player releases for that version,
so update the relay and the players together when the protocol changes.

## License

GoldLink is free: CC BY-NC-SA 4.0, on hg-engine's terms (no money involved, donations included):
`LICENSE`, `NOTICE.md`. The Rust libraries' licenses: `THIRD_PARTY_LICENSES.txt`.
Project page: https://github.com/{{REPOSITORY}}
