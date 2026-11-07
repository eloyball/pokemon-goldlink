#!/usr/bin/env python3
"""Checks that a coop-relay answers on its UDP port, without joining or creating a room.

Sends one protocol packet without a cookie: a relay of this protocol answers with its 24-byte
handshake challenge (and nothing else happens); a relay of another protocol (from the release
with the protocol refusal on) answers with status 11 and its own version.

    scripts/relay_ping.py <host> [port]      (port 7777 by default)
"""
import socket
import struct
import sys
import time
from pathlib import Path

MAGIC = 0x434F4F50  # 'COOP'
PACKET_SIZE = 120  # coop-relay/src/protocol.rs CLIENT_PACKET_SIZE
ROOT = Path(__file__).resolve().parent.parent


def protocol_version() -> int:
    text = (ROOT / "coop-relay" / "src" / "protocol.rs").read_text()
    return int(text.split("PROTOCOL_VERSION: u16 =")[1].split(";")[0])


def main() -> int:
    if len(sys.argv) < 2:
        print(__doc__.strip())
        return 2
    host, port = sys.argv[1], int(sys.argv[2]) if len(sys.argv) > 2 else 7777
    version = protocol_version()
    packet = struct.pack("<IH", MAGIC, version) + bytes(PACKET_SIZE - 6)
    addr = socket.getaddrinfo(host, port, type=socket.SOCK_DGRAM)[0]
    s = socket.socket(addr[0], socket.SOCK_DGRAM)
    s.settimeout(2.0)
    for attempt in range(3):
        start = time.monotonic()
        s.sendto(packet, addr[4])
        try:
            reply, _ = s.recvfrom(4096)
        except socket.timeout:
            print(f"[ ] attempt {attempt + 1}: no answer within 2 s")
            continue
        ms = (time.monotonic() - start) * 1000
        magic, theirs = struct.unpack_from("<IH", reply)
        status = reply[12] if len(reply) > 12 else None
        if magic != MAGIC:
            print(f"[-] {host}:{port} answered, but not as a coop-relay ({len(reply)} bytes)")
            return 1
        if status == 7:
            print(f"[*] {host}:{port}: a coop-relay for protocol {theirs} answered in {ms:.0f} ms (handshake challenge)")
            return 0
        if status == 11:
            print(f"[-] {host}:{port}: a coop-relay for protocol {theirs} (this checkout speaks {version})")
            return 1
        print(f"[*] {host}:{port}: a coop-relay answered in {ms:.0f} ms (status {status})")
        return 0
    print(f"[-] {host}:{port}: no answer. Is the relay running, and UDP {port} open in the VPS's firewall?")
    return 1


if __name__ == "__main__":
    sys.exit(main())
