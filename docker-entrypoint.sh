#!/bin/sh
set -eu

HOP_DATA_DIR="${HOP_DATA_DIR:-/data}"
HOP_CONFIG="${HOP_CONFIG:-$HOP_DATA_DIR/hop.yaml}"
HOP_CONFIG_TEMPLATE="${HOP_CONFIG_TEMPLATE:-/usr/share/hop/default-hop.yaml}"
export HOP_CONFIG
HOP_RUN_UID="$(id -u hop)"
HOP_RUN_GID="$(id -g hop)"

if [ "${1:-}" = "hop-server" ]; then
    mkdir -p "$HOP_DATA_DIR"

    if [ ! -e "$HOP_CONFIG" ]; then
        cp "$HOP_CONFIG_TEMPLATE" "$HOP_CONFIG"
        chmod 0600 "$HOP_CONFIG"
    else
        HOP_RUN_UID="$(stat -c %u "$HOP_CONFIG")"
        HOP_RUN_GID="$(stat -c %g "$HOP_CONFIG")"
    fi

    if [ "$(id -u)" = "0" ]; then
        chown -R "$HOP_RUN_UID:$HOP_RUN_GID" "$HOP_DATA_DIR"
    fi
fi

if [ "$(id -u)" = "0" ]; then
    exec setpriv --reuid="$HOP_RUN_UID" --regid="$HOP_RUN_GID" --clear-groups "$@"
fi

exec "$@"
