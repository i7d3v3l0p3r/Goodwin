#!/bin/sh
set -eu

root=$(CDPATH= cd -P "$(dirname "$0")" && pwd)
if [ "$#" != 0 ]; then
    echo "Usage: $0" >&2
    exit 2
fi
exec "$root/goodwin" serve \
    "${GOODWIN_DATA_DIR:-$root/goodwin-data}" "${GOODWIN_PANEL_ADDRESS:-127.0.0.1:8787}"
