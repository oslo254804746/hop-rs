#!/bin/sh
set -eu

HOP_DATA_DIR="${HOP_DATA_DIR:-/data}"
HOP_CONFIG="${HOP_CONFIG:-$HOP_DATA_DIR/config.toml}"
HOP_CONFIG_TEMPLATE="${HOP_CONFIG_TEMPLATE:-/usr/share/hop/config.docker.toml}"

if [ "${1:-}" = "hop-server" ]; then
    mkdir -p "$HOP_DATA_DIR"

    if [ ! -e "$HOP_CONFIG" ]; then
        cp "$HOP_CONFIG_TEMPLATE" "$HOP_CONFIG"
    fi

    if [ "$(id -u)" = "0" ]; then
        chown -R hop:hop "$HOP_DATA_DIR"
    fi
fi

if [ "$(id -u)" = "0" ]; then
    exec setpriv --reuid=hop --regid=hop --init-groups "$@"
fi

exec "$@"
