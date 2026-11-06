#!/bin/sh
# Container entrypoint. Starts as root only to make the data directory writable by the
# runtime user (PUID/PGID, default 10001), so host bind mounts work without a manual chown;
# then drops privileges and execs the server. If the container is already started as a
# non-root user (compose `user:`), it skips the fix-up and runs the server directly.
set -eu

PUID="${PUID:-10001}"
PGID="${PGID:-10001}"
DATA_DIR="${DATA_DIR:-/data}"

if [ "$(id -u)" != "0" ]; then
    exec cloudit-server "$@"
fi

case "$PUID:$PGID" in
    *[!0-9:]* | :* | *:)
        echo "entrypoint: PUID and PGID must be numeric (got PUID=$PUID PGID=$PGID)" >&2
        exit 1
        ;;
esac

mkdir -p "$DATA_DIR"
# Only touch entries with the wrong owner: cheap on restarts, fixes old or copied data.
find "$DATA_DIR" \( ! -user "$PUID" -o ! -group "$PGID" \) -exec chown -h "$PUID:$PGID" {} +

exec setpriv --reuid="$PUID" --regid="$PGID" --clear-groups -- cloudit-server "$@"
