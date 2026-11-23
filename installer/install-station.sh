#!/bin/sh
set -eu
umask 077

fail() {
    printf '%s\n' "$1" >&2
    exit 2
}

upload=${1:-}
choice=${2:-}
case "${upload%/*}" in /data/opt|/tmp) ;; *) fail 'Invalid upload directory.' ;; esac
case "${upload##*/}" in
    .goodwin-upload.[a-zA-Z0-9][a-zA-Z0-9][a-zA-Z0-9][a-zA-Z0-9][a-zA-Z0-9][a-zA-Z0-9]) ;;
    *) fail 'Invalid upload directory.' ;;
esac
[ ! -L "$upload" ] || fail 'The upload directory cannot be a symbolic link.'
upload_parent=$(cd "${upload%/*}" && pwd -P) || fail 'Could not resolve the upload parent directory.'
upload_root=$(cd "$upload" && pwd -P) || fail 'Could not open the upload directory.'
# Some firmware mounts the persistent data partition at /bkupgrade/data.
[ "$upload_root" = "$upload_parent/${upload##*/}" ] || fail 'The upload directory is outside its expected parent.'
case "$choice" in keep|standard|micro|catalog) ;; *) fail 'Invalid model selection.' ;; esac
[ "$(id -u)" = 0 ] || fail 'Goodwin installation requires root.'

bin=/data/opt/bin/goodwin
data=/data/opt/etc/goodwin
hook=/data/init.d/S96goodwin
lock=/data/opt/.goodwin-install-lock
payload=$upload/payload
stage=
mode=fresh
changed=0
start_attempted=0
was_running=0
replace_models=0

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
        if exists "$stage/old-hook"; then mv -f "$stage/old-hook" "$hook" || return 1; fi
        if exists "$stage/old-models"; then
            if exists "$data/models"; then mv "$data/models" "$stage/failed-models" || return 1; fi
            mv "$stage/old-models" "$data/models" || return 1
        elif [ "$replace_models" = 1 ] && ! exists "$stage/new-models"; then
            mv "$data/models" "$stage/failed-models" || return 1
        fi
        if exists "$stage/old-model"; then
            mv -f "$stage/old-model" "$data/model" || return 1
        elif { [ "$choice" = standard ] || [ "$choice" = micro ]; } && ! exists "$stage/new-model"; then
            rm -f "$data/model" || return 1
        fi
        if [ "$choice" = catalog ]; then
            if ! exists "$stage/new-catalog-models"; then
                mv "$data/wake-models" "$stage/new-catalog-models" || return 1
            fi
            if exists "$stage/old-catalog-models"; then
                mv "$stage/old-catalog-models" "$data/wake-models" || return 1
            fi
            if exists "$stage/old-catalog-selection"; then
                mv -f "$stage/old-catalog-selection" "$data/wake-model.json" || return 1
            elif ! exists "$stage/new-catalog-selection"; then
                rm -f "$data/wake-model.json" || return 1
            fi
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
    rm -rf "$payload" || :
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
mkdir "$payload"
gzip -dc "$upload/payload.tar.gz" | tar -xf - -C "$payload"
(cd "$payload" && sha256sum -c SHA256SUMS)
rm -f "$upload/payload.tar.gz"
chmod 755 "$payload/goodwin"

if exists "$bin" || exists "$data" || exists "$hook"; then
    if [ ! -f "$bin" ] || [ ! -f "$data/config.json" ] || [ ! -x "$hook" ] ||
        [ -L "$bin" ] || [ -L "$data" ] || [ -L "$hook" ] ||
        [ -L "$data/models" ] || [ -L "$data/model" ] ||
        [ -L "$data/wake-models" ] || [ -L "$data/wake-model.json" ]; then
        echo 'The existing installation does not match the Goodwin service layout.' >&2
        exit 1
    fi
    mode=update
fi

mv "$payload/goodwin" "$stage/new-bin"
cp "$upload/init.sh" "$stage/new-hook"
sh -n "$stage/new-hook"
chmod 755 "$stage/new-hook"
if [ "$choice" = catalog ]; then
    echo 'Checking the downloaded wake model…'
    "$stage/new-bin" wake-check "$payload/wake-model.onnx"
    hash=$(sha256sum "$payload/wake-model.onnx")
    mkdir "$stage/new-catalog-models"
    mv "$payload/wake-model.onnx" "$stage/new-catalog-models/${hash%% *}.onnx"
    mv "$payload/wake-model.json" "$stage/new-catalog-selection"
elif [ -d "$payload/models" ]; then
    model=standard
    if [ "$choice" != keep ]; then
        model=$choice
    elif [ "$mode" = update ] && exists "$data/model"; then
        read -r model < "$data/model"
    fi
    case "$model" in standard|micro) ;; *) fail 'Invalid bundled model selection.' ;; esac
    echo "Checking the $model wake model…"
    "$stage/new-bin" wake-check "$payload/models/$model.onnx"
    printf '%s\n' "$model" > "$stage/new-model"
elif [ "$choice" != keep ] || [ "$mode" = fresh ]; then
    fail 'The selected wake model is missing from the installation payload.'
fi

if [ "$mode" = fresh ]; then
    mkdir -p /data/opt/bin /data/opt/etc /data/init.d
    "$stage/new-bin" setup "$stage/new-data"
    if [ "$choice" = catalog ]; then
        mv "$stage/new-catalog-models" "$stage/new-data/wake-models"
        mv "$stage/new-catalog-selection" "$stage/new-data/wake-model.json"
    else
        mv "$payload/models" "$stage/new-data/models"
        mv "$stage/new-model" "$stage/new-data/model"
    fi
    changed=1
    mv "$stage/new-bin" "$bin"
    mv "$stage/new-data" "$data"
    mv "$stage/new-hook" "$hook"
else
    if [ -d "$payload/models" ]; then
        for name in standard.onnx micro.onnx LICENSE NOTICE; do
            if ! cmp -s "$payload/models/$name" "$data/models/$name"; then
                replace_models=1
                break
            fi
        done
    fi
    if [ "$replace_models" = 1 ]; then
        mkdir "$stage/new-models"
        if [ -d "$data/models" ]; then cp -pR "$data/models/." "$stage/new-models/"; fi
        for name in standard.onnx micro.onnx LICENSE NOTICE; do
            rm -f "$stage/new-models/$name"
            mv "$payload/models/$name" "$stage/new-models/$name"
        done
    fi
    if "$hook" status; then was_running=1; fi
    "$hook" stop
    changed=1
    mv "$bin" "$stage/old-bin"
    mv "$stage/new-bin" "$bin"
    mv "$hook" "$stage/old-hook"
    mv "$stage/new-hook" "$hook"
    if [ "$replace_models" = 1 ]; then
        if exists "$data/models"; then mv "$data/models" "$stage/old-models"; fi
        mv "$stage/new-models" "$data/models"
    fi
    if [ "$choice" = catalog ]; then
        if exists "$data/wake-models"; then mv "$data/wake-models" "$stage/old-catalog-models"; fi
        if exists "$data/wake-model.json"; then mv "$data/wake-model.json" "$stage/old-catalog-selection"; fi
        mv "$stage/new-catalog-models" "$data/wake-models"
        mv "$stage/new-catalog-selection" "$data/wake-model.json"
    elif [ "$choice" != keep ]; then
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
