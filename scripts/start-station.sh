#!/bin/sh
set -eu

root=$(CDPATH= cd -P "$(dirname "$0")" && pwd)
case "${1:-standard}" in
    standard|micro) model="${1:-standard}" ;;
    *) echo "Usage: $0 [standard|micro]" >&2; exit 2 ;;
esac

GOODWIN_WAKE_MODEL="$root/models/$model.onnx" exec "$root/goodwin" serve \
    "${GOODWIN_DATA_DIR:-$root/goodwin-data}" "${GOODWIN_PANEL_ADDRESS:-127.0.0.1:8787}"
