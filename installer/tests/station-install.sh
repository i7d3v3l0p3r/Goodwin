#!/bin/sh
set -eu
TEST_ROOT=$(mktemp -d /tmp/goodwin-installer-check.XXXXXX)
case "$TEST_ROOT" in /tmp/goodwin-installer-check.??????) ;; *) exit 2 ;; esac
trap 'rm -rf "$TEST_ROOT"' 0
export TEST_ROOT
mkdir -p "$TEST_ROOT/tools" "$TEST_ROOT/bkupgrade/data/opt" "$TEST_ROOT/bkupgrade/data/init.d" "$TEST_ROOT/tmp"
ln -s bkupgrade/data "$TEST_ROOT/data"
PATH="$TEST_ROOT/tools:$PATH"
export PATH
sed "s,/data/opt,$TEST_ROOT/data/opt,g; s,/data/init.d,$TEST_ROOT/data/init.d,g; s,/tmp/goodwin.log,$TEST_ROOT/goodwin.log,g; s,|/tmp),|$TEST_ROOT/tmp),g" installer/install-station.sh > "$TEST_ROOT/install-test.sh"
cat > "$TEST_ROOT/tools/id" <<'EOF'
#!/bin/sh
echo 0
EOF
cat > "$TEST_ROOT/tools/wget" <<'EOF'
#!/bin/sh
if [ -s "$TEST_ROOT/running" ] && [ "$(cat "$TEST_ROOT/running")" != broken ]; then
    printf '%s' '{"authorized":false,"protected":true}'
else
    exit 1
fi
EOF
cat > "$TEST_ROOT/tools/sleep" <<'EOF'
#!/bin/sh
if [ -f "$TEST_ROOT/stopping" ]; then
    rm -f "$TEST_ROOT/running" "$TEST_ROOT/stopping"
fi
EOF
chmod 755 "$TEST_ROOT/tools/"*
upload="$TEST_ROOT/data/opt/.goodwin-upload.test01"
mkdir "$upload" "$TEST_ROOT/payload"
linked_upload="$TEST_ROOT/data/opt/.goodwin-upload.badlnk"
ln -s "$upload" "$linked_upload"
if sh "$TEST_ROOT/install-test.sh" "$linked_upload" keep > "$TEST_ROOT/linked.out" 2>&1; then
    echo "The upload directory symlink unexpectedly succeeded" >&2
    exit 1
fi
grep 'The upload directory cannot be a symbolic link' "$TEST_ROOT/linked.out"
cat > "$upload/init.sh" <<'EOF'
#!/bin/sh
case "$1" in
    status) test -s "$TEST_ROOT/running" ;;
    start) "$TEST_ROOT/data/opt/bin/goodwin" version > "$TEST_ROOT/running" ;;
    stop)
        if [ -f "$TEST_ROOT/slow-stop" ] && [ -s "$TEST_ROOT/running" ]; then
            touch "$TEST_ROOT/stopping"
            exit 1
        fi
        rm -f "$TEST_ROOT/running"
        ;;
esac
EOF

package() {
    version=$1
    model_version=${2:-$version}
    cat > "$TEST_ROOT/payload/goodwin" <<EOF
#!/bin/sh
case "\$1" in
    wake-check)
        case "\$2" in
            --installed)
                test -f "\$3/wake-model.json" || exit 1
                if grep -q 'livekit-v2-micro-tflite' "\$3/wake-model.json"; then
                    test "$version" != without-int8
                elif [ "$version" = reject-installed-model ]; then
                    exit 1
                else
                    grep -q 'livekit-v2-micro-onnx' "\$3/wake-model.json"
                fi
                ;;
            *.tflite) test "$version" != without-int8 && test -f "\$2" && test -f "\$3" ;;
            *) test -f "\$2" && grep -q 'livekit-v2-micro-onnx' "\$3" ;;
        esac
        ;;
    version|--version) echo "$version" ;;
    setup) mkdir "\$2" && echo settings > "\$2/config.json" ;;
esac
EOF
    cd "$TEST_ROOT/payload"
    set -- "${3:-core}"
    case "$1" in
        catalog)
            printf '%s\n' "$model_version" > wake-model.onnx
            hash=$(sha256sum wake-model.onnx)
            hash=${hash%% *}
            printf '{"id":"%s","wake_word":"Егор","tier":"micro","sha256":"%s","runtime":"livekit-v2-micro-onnx"}\n' "$model_version" "$hash" > wake-model.json
            set -- goodwin wake-model.onnx wake-model.json
            ;;
        int8)
            printf '%s\n' "$model_version" > wake-model.tflite
            hash=$(sha256sum wake-model.tflite)
            hash=${hash%% *}
            printf '{"id":"%s","wake_word":"Гудвин","tier":"micro","sha256":"%s","runtime":"livekit-v2-micro-tflite","score_threshold":0.390625}\n' "$model_version" "$hash" > wake-model.json
            set -- goodwin wake-model.tflite wake-model.json
            ;;
        core) set -- goodwin ;;
    esac
    sha256sum "$@" > SHA256SUMS
    tar -czf "$upload/payload.tar.gz" "$@" SHA256SUMS
    cd "$TEST_ROOT"
}

package initial egor-micro catalog
sh "$TEST_ROOT/install-test.sh" "$upload" catalog | tee "$TEST_ROOT/fresh.out"
grep 'GOODWIN_INSTALL_OK fresh' "$TEST_ROOT/fresh.out"
grep 'GOODWIN_INSTALL_VERSION initial' "$TEST_ROOT/fresh.out"
data=$TEST_ROOT/data/opt/etc/goodwin
grep 'egor-micro' "$data/wake-model.json"
test "$(cat "$data/wake-models/$hash.onnx")" = egor-micro
selected_hash=$hash
echo retained > "$data/config.json"
mkdir "$data/plugins"

install_init=$upload/init.sh
upload="$TEST_ROOT/tmp/.goodwin-upload.test02"
mkdir "$upload"
cp "$install_init" "$upload/init.sh"
models_inode=$(stat -c %i "$data/wake-models")
package catalog-keep
sh "$TEST_ROOT/install-test.sh" "$upload" keep | tee "$TEST_ROOT/update.out"
grep 'GOODWIN_INSTALL_OK update' "$TEST_ROOT/update.out"
grep 'GOODWIN_INSTALL_VERSION catalog-keep' "$TEST_ROOT/update.out"
test "$(cat "$TEST_ROOT/running")" = catalog-keep
test "$(stat -c %i "$data/wake-models")" = "$models_inode"
test "$(cat "$data/wake-models/$selected_hash.onnx")" = egor-micro
test ! -e "$upload/payload"
for stage in "$TEST_ROOT/data/opt"/.goodwin-stage.*; do test ! -e "$stage"; done

package reject-installed-model
if sh "$TEST_ROOT/install-test.sh" "$upload" keep > "$TEST_ROOT/rejected-model.out" 2>&1; then
    echo "An update that rejects the installed ONNX model succeeded" >&2
    exit 1
fi
grep 'could not use the installed wake model' "$TEST_ROOT/rejected-model.out"
test "$(cat "$TEST_ROOT/running")" = catalog-keep

package broken goodwin-micro catalog
if sh "$TEST_ROOT/install-test.sh" "$upload" catalog > "$TEST_ROOT/failed-catalog.out" 2>&1; then
    echo "The wake model change with a failed startup unexpectedly succeeded" >&2
    exit 1
fi
grep 'Restoring the previous state' "$TEST_ROOT/failed-catalog.out"
grep 'GOODWIN_INSTALL_ERROR rolled_back' "$TEST_ROOT/failed-catalog.out"
test "$(cat "$TEST_ROOT/running")" = catalog-keep
grep 'egor-micro' "$data/wake-model.json"
test "$(cat "$data/wake-models/$selected_hash.onnx")" = egor-micro
test ! -e "$data/wake-models/$hash.onnx"
test "$(cat "$data/config.json")" = retained
test -d "$data/plugins"
test ! -e "$TEST_ROOT/data/opt/.goodwin-install-lock"

hook=$TEST_ROOT/data/init.d/S96goodwin
web_upload="$TEST_ROOT/data/opt/.goodwin-upload.web001"
mkdir "$web_upload"
hook_inode=$(stat -c %i "$hook")
package web-release '' core
cp "$TEST_ROOT/payload/goodwin" "$web_upload/goodwin"
chmod 755 "$web_upload/goodwin"
if sh "$TEST_ROOT/install-test.sh" "$web_upload" core stale > "$TEST_ROOT/stale.out" 2>&1; then
    echo "A web update over an unexpected installed version succeeded" >&2
    exit 1
fi
grep 'The installed Goodwin version changed' "$TEST_ROOT/stale.out"
test "$(cat "$TEST_ROOT/running")" = catalog-keep
cp "$TEST_ROOT/payload/goodwin" "$web_upload/goodwin"
chmod 755 "$web_upload/goodwin"
sh "$TEST_ROOT/install-test.sh" "$web_upload" core catalog-keep
test "$(cat "$TEST_ROOT/running")" = web-release
test "$(stat -c %i "$hook")" = "$hook_inode"
test "$(cat "$data/wake-models/$selected_hash.onnx")" = egor-micro

package broken '' core
cp "$TEST_ROOT/payload/goodwin" "$web_upload/goodwin"
chmod 755 "$web_upload/goodwin"
if sh "$TEST_ROOT/install-test.sh" "$web_upload" core web-release > "$TEST_ROOT/failed-web.out" 2>&1; then
    echo "The web update with a failed startup unexpectedly succeeded" >&2
    exit 1
fi
grep 'GOODWIN_CORE_ROLLED_BACK' "$TEST_ROOT/failed-web.out"
test "$(cat "$TEST_ROOT/running")" = web-release
test "$(stat -c %i "$hook")" = "$hook_inode"
test "$(cat "$data/config.json")" = retained
test ! -e "$TEST_ROOT/data/opt/.goodwin-install-lock"

touch "$TEST_ROOT/slow-stop"
package slow-stop-update '' core
cp "$TEST_ROOT/payload/goodwin" "$web_upload/goodwin"
chmod 755 "$web_upload/goodwin"
sh "$TEST_ROOT/install-test.sh" "$web_upload" core web-release > "$TEST_ROOT/slow-stop.out" 2>&1
grep 'Waiting for the previous Goodwin process to stop' "$TEST_ROOT/slow-stop.out"
grep 'GOODWIN_INSTALL_OK update' "$TEST_ROOT/slow-stop.out"
test "$(cat "$TEST_ROOT/running")" = slow-stop-update
rm "$TEST_ROOT/slow-stop"

"$hook" stop
mv "$hook" "$TEST_ROOT/previous-hook"
mv "$data" "$TEST_ROOT/previous-data"
mv "$TEST_ROOT/data/opt/bin/goodwin" "$TEST_ROOT/previous-bin"
package fresh-catalog goodwin-micro catalog
sh "$TEST_ROOT/install-test.sh" "$upload" catalog
grep 'goodwin-micro' "$data/wake-model.json"
test "$(cat "$data/wake-models/$hash.onnx")" = goodwin-micro
test ! -e "$data/models"

package without-int8 goodwin-micro-int8 int8
if sh "$TEST_ROOT/install-test.sh" "$upload" catalog > "$TEST_ROOT/unsupported-int8.out" 2>&1; then
    echo 'An executable without INT8 support accepted a TFLite model' >&2
    exit 1
fi
grep 'could not load the downloaded wake model' "$TEST_ROOT/unsupported-int8.out"
test "$(cat "$TEST_ROOT/running")" = fresh-catalog
package with-int8 goodwin-micro-int8 int8
sh "$TEST_ROOT/install-test.sh" "$upload" catalog
test "$(cat "$data/wake-models/$hash.tflite")" = goodwin-micro-int8

package without-int8 '' core
if sh "$TEST_ROOT/install-test.sh" "$upload" keep > "$TEST_ROOT/unsupported-keep.out" 2>&1; then
    echo 'An executable without INT8 support replaced an installation using TFLite' >&2
    exit 1
fi
grep 'could not use the installed wake model' "$TEST_ROOT/unsupported-keep.out"
test "$(cat "$TEST_ROOT/running")" = with-int8

printf '%s\n' 'Catalog installs, web updates and rollback passed.'
