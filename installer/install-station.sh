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
if [ "$upload" != --recover ]; then
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
fi
[ "$(id -u)" = 0 ] || fail 'Goodwin installation requires root.'

bin=/data/opt/bin/goodwin
data=/data/opt/etc/goodwin
hook=/data/init.d/S96goodwin
lock=/data/opt/.goodwin-install-lock
guard=/data/opt/.goodwin-install-guard
guarded_transaction=/data/opt/.goodwin-install-transaction
payload=$upload/payload
kernel_lock=0
mode=fresh
changed=0
was_running=0

exists() {
    [ -e "$1" ] || [ -L "$1" ]
}

use_transaction() {
    transaction=$1
    stage=$transaction/stage
    journal=$transaction/journal
}

rollback() {
    if [ -x "$hook" ] && "$hook" status; then
        "$hook" stop 9>&- || return 1
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
        if exists "$stage/old-hook" && [ "$stage/old-hook" -ef "$hook" ]; then
            rm -f "$stage/old-hook" || return 1
        elif exists "$stage/old-hook"; then
            mv -f "$stage/old-hook" "$hook" || return 1
        fi
        if [ "$choice" = catalog ]; then
            if ! exists "$stage/new-catalog-models"; then
                mv "$data/wake-models" "$stage/new-catalog-models" || return 1
            fi
            if exists "$stage/old-catalog-models"; then
                mv "$stage/old-catalog-models" "$data/wake-models" || return 1
            fi
            if ! exists "$stage/new-catalog-selection"; then
                mv "$data/wake-model.json" "$stage/new-catalog-selection" || return 1
            fi
            if exists "$stage/old-catalog-selection"; then
                mv -f "$stage/old-catalog-selection" "$data/wake-model.json" || return 1
            fi
        fi
        if [ "$was_running" = 1 ]; then "$hook" start 9>&- || return 1; fi
    fi
}

write_journal() {
    printf '%s %s %s %s %s\n' "$1" "$mode" "$choice" "$was_running" "$kernel_lock" > "$transaction/journal.tmp" &&
        mv -f "$transaction/journal.tmp" "$journal"
}

finish_transaction() {
    # Persist the active installation (or restored files) before discarding its backup.
    sync && write_journal done && sync
}

remove_transaction() {
    if exists "$transaction"; then
        [ -d "$transaction" ] && [ ! -L "$transaction" ] &&
            rm -rf "$stage" && rm -f "$journal" "$transaction/journal.tmp" &&
            rmdir "$transaction" || return 1
    fi
    if [ "$transaction" != "$lock" ]; then
        # Keep ownership visible until removing the transaction is durable.
        sync && rm -f "$lock" || return 1
    fi
}

cleanup() {
    status=$?
    trap - 0
    trap '' HUP INT TERM
    if [ "$changed" = 1 ]; then
        echo 'Installation failed. Restoring the previous state.' >&2
        if ! rollback || ! finish_transaction; then
            printf '%s\n' 'GOODWIN_INSTALL_ERROR recovery_failed'
            echo "Automatic recovery failed. Backup: $stage" >&2
            echo "Installation lock retained: $lock" >&2
            exit 1
        fi
        printf '%s\n' 'GOODWIN_INSTALL_ERROR rolled_back'
        if [ "$choice" = core ]; then printf '%s\n' 'GOODWIN_CORE_ROLLED_BACK'; fi
    fi
    remove_transaction || :
    rm -rf "$payload" || :
    exit "$status"
}

locked() {
    printf '%s\n' 'GOODWIN_INSTALL_ERROR locked'
    echo "Another installation is in progress or needs recovery: $lock" >&2
    exit 1
}

recover_transaction() {
    [ "$kernel_lock" = 1 ] || return 1
    if [ ! -L "$lock" ] && [ "$lock" -ef "$guard" ]; then
        use_transaction "$guarded_transaction"
        if exists "$transaction"; then
            [ -d "$transaction" ] && [ ! -L "$transaction" ] || return 1
        fi
        if ! exists "$journal"; then
            # This marker belongs to a flock user. Without a journal, the
            # transaction stopped before starting or after durable completion.
            remove_transaction
            return
        fi
    else
        # A legacy installer may still own the directory lock without flock.
        [ -d "$lock" ] && [ ! -L "$lock" ] || return 1
        use_transaction "$lock"
    fi
    [ -f "$journal" ] && [ ! -L "$journal" ] &&
        read -r phase mode choice was_running previous_kernel_lock < "$journal" || return 1
    [ "$previous_kernel_lock" = 1 ] || return 1
    case "$mode:$choice:$was_running" in
        fresh:catalog:0|update:catalog:[01]|update:keep:[01]|update:core:[01]) ;;
        fresh:keep:0|fresh:core:0) [ "$phase" = preparing ] || return 1 ;;
        *) return 1 ;;
    esac
    case "$phase" in
        mutating)
            [ -d "$stage" ] && [ ! -L "$stage" ] || return 1
            echo 'Recovering an interrupted Goodwin installation…' >&2
            rollback && finish_transaction || return 1
            ;;
        preparing|done) ;;
        *) return 1 ;;
    esac
    remove_transaction
}

# Keep the guard inode across runs so all flock users lock the same file.
if command -v flock >/dev/null 2>&1; then
    exec 9>>"$guard"
    flock -n 9 || locked
    kernel_lock=1
elif command -v busybox >/dev/null 2>&1 && busybox --list | grep -qx flock; then
    exec 9>>"$guard"
    busybox flock -n 9 || locked
    kernel_lock=1
fi

if [ "$upload" = --recover ]; then
    if exists "$lock"; then recover_transaction || locked; fi
    echo 'GOODWIN_INSTALL_RECOVERY_OK'
    exit 0
fi

requested_choice=$choice
if exists "$lock"; then recover_transaction || locked; fi
choice=$requested_choice
mode=fresh
was_running=0
if [ "$kernel_lock" = 1 ]; then
    use_transaction "$guarded_transaction"
    ! exists "$transaction" || locked
    # Old installers use mkdir on this path. A hard link excludes them
    # atomically and identifies the owners that use the same flock guard.
    ln -T "$guard" "$lock" || locked
else
    use_transaction "$lock"
    mkdir "$lock" || locked
fi
trap cleanup 0
trap 'exit 1' HUP INT TERM
if [ "$transaction" != "$lock" ]; then
    sync
    mkdir "$transaction"
fi
mkdir "$stage"
write_journal preparing
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

if [ "$mode" = fresh ]; then
    mkdir -p /data/opt/bin /data/opt/etc /data/init.d
    "$stage/new-bin" setup "$stage/new-data"
    mv "$stage/new-catalog-models" "$stage/new-data/wake-models"
    mv "$stage/new-catalog-selection" "$stage/new-data/wake-model.json"
    write_journal mutating
    changed=1
    sync
    mv "$stage/new-bin" "$bin"
    mv "$stage/new-data" "$data"
    mv "$stage/new-hook" "$hook"
else
    if "$hook" status; then was_running=1; fi
    write_journal mutating
    changed=1
    sync
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
    if [ "$choice" != catalog ]; then
        # Run inference after the old process releases its model. Older
        # packages still support the original catalog check.
        if "$stage/new-bin" --help | grep -q -e 'wake-check --installed-inference '; then
            "$stage/new-bin" wake-check --installed-inference "$data" ||
                fail_model 'The selected Goodwin build could not use the installed wake model.'
        elif [ -f "$data/wake-model.json" ]; then
            "$stage/new-bin" wake-check --installed "$data" ||
                fail_model 'The selected Goodwin build could not use the installed wake model.'
        fi
    fi
    ln "$bin" "$stage/old-bin"
    if [ "$choice" != core ]; then ln "$hook" "$stage/old-hook"; fi
    sync
    mv -f "$stage/new-bin" "$bin"
    if [ "$choice" != core ]; then
        mv -f "$stage/new-hook" "$hook"
    fi
    if [ "$choice" = catalog ]; then
        if exists "$data/wake-models"; then mv "$data/wake-models" "$stage/old-catalog-models"; fi
        if exists "$data/wake-model.json"; then mv "$data/wake-model.json" "$stage/old-catalog-selection"; fi
        sync
        mv "$stage/new-catalog-models" "$data/wake-models"
        mv "$stage/new-catalog-selection" "$data/wake-model.json"
    fi
fi

"$hook" start 9>&-
for attempt in 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15; do
    if "$hook" status &&
        access=$(wget -T 2 -qO- http://127.0.0.1:8787/api/access 2>/dev/null) &&
        printf '%s' "$access" | grep -Eq '"authorized"[[:space:]]*:[[:space:]]*(true|false)' &&
        printf '%s' "$access" | grep -Eq '"protected"[[:space:]]*:[[:space:]]*(true|false)'; then
        finish_transaction
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
