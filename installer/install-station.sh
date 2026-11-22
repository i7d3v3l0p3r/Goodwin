#!/bin/sh
set -eu
umask 077

fail() {
    printf '%s\n' "$1" >&2
    exit 2
}

upload=${1:-}
choice=${2:-}
case "$upload" in
    /data/opt/.goodwin-upload.[a-zA-Z0-9][a-zA-Z0-9][a-zA-Z0-9][a-zA-Z0-9][a-zA-Z0-9][a-zA-Z0-9]) ;;
    *) fail 'Invalid upload directory.' ;;
esac
[ ! -L "$upload" ] || fail 'The upload directory cannot be a symbolic link.'
opt_root=$(cd /data/opt && pwd -P) || fail 'Could not resolve the Station installation directory.'
upload_root=$(cd "$upload" && pwd -P) || fail 'Could not open the upload directory.'
# Some firmware mounts the persistent data partition at /bkupgrade/data.
[ "$upload_root" = "$opt_root/${upload##*/}" ] || fail 'The upload directory is outside the installation directory.'
case "$choice" in keep|standard|micro) ;; *) fail 'Invalid model selection.' ;; esac
[ "$(id -u)" = 0 ] || fail 'Goodwin installation requires root.'

bin=/data/opt/bin/goodwin
data=/data/opt/etc/goodwin
hook=/data/init.d/S96goodwin
lock=/data/opt/.goodwin-install-lock
stage=
mode=fresh
changed=0
start_attempted=0
was_running=0

exists() {
    [ -e "$1" ] || [ -L "$1" ]
}

rollback() {
    if [ "$start_attempted" = 1 ]; then
        "$hook" stop || return 1
    fi
    if [ "$mode" = fresh ]; then
        if ! exists "$stage/new-hook"; then mv "$hook" "$stage/new-hook" || return 1; fi
        if ! exists "$stage/new-data"; then mv "$data" "$stage/new-data" || return 1; fi
        if ! exists "$stage/new-bin"; then mv "$bin" "$stage/new-bin" || return 1; fi
    else
        if exists "$stage/old-bin"; then mv -f "$stage/old-bin" "$bin" || return 1; fi
        if exists "$stage/old-models"; then
            if exists "$data/models"; then mv "$data/models" "$stage/failed-models" || return 1; fi
            mv "$stage/old-models" "$data/models" || return 1
        fi
        if exists "$stage/old-model"; then
            mv -f "$stage/old-model" "$data/model" || return 1
        elif [ "$choice" != keep ] && ! exists "$stage/new-model"; then
            rm -f "$data/model" || return 1
        fi
        if [ "$was_running" = 1 ]; then "$hook" start || return 1; fi
    fi
}

cleanup() {
    status=$?
    trap - 0
    trap '' HUP INT TERM
    if [ "$changed" = 1 ]; then
        echo 'Installation failed. Restoring the previous state.' >&2
        if ! rollback; then
            echo "Automatic recovery failed. Backup: $stage" >&2
            echo "Installation lock retained: $lock" >&2
            exit 1
        fi
    fi
    if [ -n "$stage" ]; then rm -rf "$stage" || :; fi
    rmdir "$lock" || :
    exit "$status"
}

if ! mkdir "$lock"; then
    echo "Another installation is in progress or needs recovery: $lock" >&2
    exit 1
fi
trap cleanup 0
trap 'exit 1' HUP INT TERM
stage=$(mktemp -d /data/opt/.goodwin-stage.XXXXXX)
mkdir "$stage/payload"
gzip -dc "$upload/payload.tar.gz" | tar -xf - -C "$stage/payload"
(cd "$stage/payload" && sha256sum -c SHA256SUMS)
chmod 755 "$stage/payload/goodwin"

if exists "$bin" || exists "$data" || exists "$hook"; then
    if [ ! -f "$bin" ] || [ ! -f "$data/config.json" ] || [ ! -x "$hook" ] ||
        [ ! -d "$data/models" ] || [ -L "$bin" ] || [ -L "$data" ] || [ -L "$hook" ] ||
        [ -L "$data/models" ] || [ -L "$data/model" ]; then
        echo 'The existing installation does not match the Goodwin service layout.' >&2
        exit 1
    fi
    mode=update
fi

model=standard
if [ "$choice" != keep ]; then
    model=$choice
elif [ "$mode" = update ] && exists "$data/model"; then
    read -r model < "$data/model"
fi
case "$model" in standard|micro) ;; *) echo 'Invalid model selection.' >&2; exit 1 ;; esac

echo "Checking the $model wake model…"
"$stage/payload/goodwin" wake-check "$stage/payload/models/$model.onnx"
mv "$stage/payload/goodwin" "$stage/new-bin"
printf '%s\n' "$model" > "$stage/new-model"

if [ "$mode" = fresh ]; then
    mkdir -p /data/opt/bin /data/opt/etc /data/init.d
    "$stage/new-bin" setup "$stage/new-data"
    mv "$stage/payload/models" "$stage/new-data/models"
    mv "$stage/new-model" "$stage/new-data/model"
    cp "$upload/init.sh" "$stage/new-hook"
    sh -n "$stage/new-hook"
    chmod 755 "$stage/new-hook"
    changed=1
    mv "$stage/new-bin" "$bin"
    mv "$stage/new-data" "$data"
    mv "$stage/new-hook" "$hook"
else
    mkdir "$stage/new-models"
    cp -pR "$data/models/." "$stage/new-models/"
    for name in standard.onnx micro.onnx LICENSE NOTICE; do
        rm -f "$stage/new-models/$name"
        mv "$stage/payload/models/$name" "$stage/new-models/$name"
    done
    if "$hook" status; then was_running=1; fi
    "$hook" stop
    changed=1
    mv "$bin" "$stage/old-bin"
    mv "$stage/new-bin" "$bin"
    mv "$data/models" "$stage/old-models"
    mv "$stage/new-models" "$data/models"
    if [ "$choice" != keep ]; then
        if exists "$data/model"; then mv "$data/model" "$stage/old-model"; fi
        mv "$stage/new-model" "$data/model"
    fi
fi

start_attempted=1
"$hook" start
for attempt in 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15; do
    if "$hook" status &&
        access=$(wget -T 2 -qO- http://127.0.0.1:8787/api/access 2>/dev/null) &&
        printf '%s' "$access" | grep -Eq '"authorized"[[:space:]]*:[[:space:]]*(true|false)' &&
        printf '%s' "$access" | grep -Eq '"protected"[[:space:]]*:[[:space:]]*(true|false)'; then
        changed=0
        echo "GOODWIN_INSTALL_OK $mode"
        exit 0
    fi
    sleep 1
done
echo 'Goodwin did not pass its panel startup check.' >&2
tail -15 /tmp/goodwin.log >&2 || :
exit 1
