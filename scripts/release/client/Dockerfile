# The unified client's Linux build (scripts/package_client.sh): Debian 11 (glibc 2.31), so the
# program runs on older distributions too. melonDS needs CMake and a C++17 compiler; cpal and
# gilrs link ALSA and libudev. Debian 11's security updates moved to archive.debian.org.
FROM rust:1.98.0-bullseye
RUN for f in /etc/apt/sources.list /etc/apt/sources.list.d/*; do \
        [ -f "$f" ] && sed -i 's|deb.debian.org/debian-security|archive.debian.org/debian-security|' "$f"; done; \
    apt-get update -q && apt-get install -y -q --no-install-recommends cmake libasound2-dev libudev-dev \
    && rm -rf /var/lib/apt/lists/*
