#!/bin/sh
set -eu
umask 077

fail() {
    printf '%s\n' "$1" >&2
    exit 2
}

fail_model() {
    printf '%s\n' 'GOODWIN_INSTALL_ERROR model'
    fail "$1"
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
case "$choice" in keep|catalog|core) ;; *) fail 'Invalid installation option.' ;; esac
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
        if exists "$stage/old-bin" && [ "$stage/old-bin" -ef "$bin" ]; then
            rm -f "$stage/old-bin" || return 1
        elif exists "$stage/old-bin"; then
            mv -f "$stage/old-bin" "$bin" || return 1
        fi
        if exists "$stage/old-hook"; then mv -f "$stage/old-hook" "$hook" || return 1; fi
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
            printf '%s\n' 'GOODWIN_INSTALL_ERROR recovery_failed'
            echo "Automatic recovery failed. Backup: $stage" >&2
            echo "Installation lock retained: $lock" >&2
            exit 1
        fi
        printf '%s\n' 'GOODWIN_INSTALL_ERROR rolled_back'
        if [ "$choice" = core ]; then printf '%s\n' 'GOODWIN_CORE_ROLLED_BACK'; fi
    fi
    if [ -n "$stage" ]; then rm -rf "$stage" || :; fi
    rm -rf "$payload" || :
    rmdir "$lock" || :
    exit "$status"
}

if ! mkdir "$lock"; then
    printf '%s\n' 'GOODWIN_INSTALL_ERROR locked'
    echo "Another installation is in progress or needs recovery: $lock" >&2
    exit 1
fi
trap cleanup 0
trap 'exit 1' HUP INT TERM
stage=$(mktemp -d /data/opt/.goodwin-stage.XXXXXX)
if [ "$choice" = core ]; then
    # The panel updater has already verified the release and staged its executable.
    [ -f "$upload/goodwin" ] && [ ! -L "$upload/goodwin" ] || fail 'Missing staged executable.'
    mv "$upload/goodwin" "$stage/new-bin"
else
    mkdir "$payload"
    gzip -dc "$upload/payload.tar.gz" | tar -xf - -C "$payload"
    (cd "$payload" && sha256sum -c SHA256SUMS)
    rm -f "$upload/payload.tar.gz"
    chmod 755 "$payload/goodwin"
    mv "$payload/goodwin" "$stage/new-bin"
    cp "$upload/init.sh" "$stage/new-hook"
    sh -n "$stage/new-hook"
    chmod 755 "$stage/new-hook"
fi

if exists "$bin" || exists "$data" || exists "$hook"; then
    if [ ! -f "$bin" ] || [ ! -f "$data/config.json" ] || [ ! -x "$hook" ] ||
        [ -L "$bin" ] || [ -L "$data" ] || [ -L "$hook" ] ||
        [ -L "$data/wake-models" ] || [ -L "$data/wake-model.json" ]; then
        printf '%s\n' 'GOODWIN_INSTALL_ERROR layout'
        echo 'The existing installation does not match the Goodwin service layout.' >&2
        exit 1
    fi
    mode=update
fi

if [ "$choice" = core ]; then
    [ "$mode" = update ] || fail 'The Goodwin service is not installed.'
    [ -n "${3:-}" ] && [ "$("$bin" --version)" = "$3" ] || fail 'The installed Goodwin version changed.'
elif [ "$choice" = catalog ]; then
    echo 'Checking the downloaded wake model…'
    extension=onnx
    if [ -f "$payload/wake-model.tflite" ]; then extension=tflite; fi
    model_file=$payload/wake-model.$extension
    "$stage/new-bin" wake-check "$model_file" "$payload/wake-model.json" ||
        fail_model 'The selected Goodwin build could not load the downloaded wake model.'
    hash=$(sha256sum "$model_file")
    mkdir "$stage/new-catalog-models"
    mv "$model_file" "$stage/new-catalog-models/${hash%% *}.$extension"
    mv "$payload/wake-model.json" "$stage/new-catalog-selection"
elif [ "$mode" = fresh ]; then
    fail_model 'The selected wake model is missing from the installation payload.'
fi

if [ "$mode" = update ] && [ "$choice" != catalog ] && [ -f "$data/wake-model.json" ]; then
    "$stage/new-bin" wake-check --installed "$data" ||
        fail_model 'The selected Goodwin build could not use the installed wake model.'
fi

if [ "$mode" = fresh ]; then
    mkdir -p /data/opt/bin /data/opt/etc /data/init.d
    "$stage/new-bin" setup "$stage/new-data"
    mv "$stage/new-catalog-models" "$stage/new-data/wake-models"
    mv "$stage/new-catalog-selection" "$stage/new-data/wake-model.json"
    changed=1
    mv "$stage/new-bin" "$bin"
    mv "$stage/new-data" "$data"
    mv "$stage/new-hook" "$hook"
else
    if "$hook" status; then was_running=1; fi
    if ! "$hook" stop; then
        [ "$was_running" = 1 ] || fail 'Could not stop Goodwin.'
        # Older builds can wait up to 60 seconds for a provider request after SIGTERM.
        echo 'Waiting for the previous Goodwin process to stop…' >&2
        attempts=65
        while "$hook" status && [ "$attempts" -gt 0 ]; do
            sleep 1
            attempts=$((attempts - 1))
        done
        if "$hook" status; then fail 'The previous Goodwin process did not stop.'; fi
        "$hook" stop
    fi
    changed=1
    if [ "$choice" = core ]; then
        ln "$bin" "$stage/old-bin"
        mv -f "$stage/new-bin" "$bin"
    else
        mv "$bin" "$stage/old-bin"
        mv "$stage/new-bin" "$bin"
        mv "$hook" "$stage/old-hook"
        mv "$stage/new-hook" "$hook"
    fi
    if [ "$choice" = catalog ]; then
        if exists "$data/wake-models"; then mv "$data/wake-models" "$stage/old-catalog-models"; fi
        if exists "$data/wake-model.json"; then mv "$data/wake-model.json" "$stage/old-catalog-selection"; fi
        mv "$stage/new-catalog-models" "$data/wake-models"
        mv "$stage/new-catalog-selection" "$data/wake-model.json"
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
        if version=$("$bin" --version 2>/dev/null); then
            case "$version" in
                ''|*[!0-9A-Za-z.+-]*) ;;
                *) printf 'GOODWIN_INSTALL_VERSION %s\n' "$version" ;;
            esac
        fi
        echo "GOODWIN_INSTALL_OK $mode"
        exit 0
    fi
    sleep 1
done
echo 'Goodwin did not pass its panel startup check.' >&2
tail -15 /tmp/goodwin.log >&2 || :
exit 1
