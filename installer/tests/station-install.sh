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
:
EOF
chmod 755 "$TEST_ROOT/tools/"*
upload="$TEST_ROOT/data/opt/.goodwin-upload.test01"
mkdir "$upload" "$TEST_ROOT/payload" "$TEST_ROOT/payload/models"
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
    stop) rm -f "$TEST_ROOT/running" ;;
esac
EOF

package() {
    version=$1
    model_version=${2:-$version}
    cat > "$TEST_ROOT/payload/goodwin" <<EOF
#!/bin/sh
case "\$1" in
    wake-check) test -f "\$2" ;;
    version) echo "$version" ;;
    setup) mkdir "\$2" && echo settings > "\$2/config.json" ;;
esac
EOF
    cd "$TEST_ROOT/payload"
    set -- "${3:-bundled}"
    case "$1" in
        bundled)
            for file in standard.onnx micro.onnx LICENSE NOTICE; do
                echo "$model_version" > "models/$file"
            done
            set -- goodwin models/*
            ;;
        catalog)
            printf '%s\n' "$model_version" > wake-model.onnx
            hash=$(sha256sum wake-model.onnx)
            hash=${hash%% *}
            printf '{"id":"%s","wake_word":"Егор","tier":"micro","sha256":"%s"}\n' "$model_version" "$hash" > wake-model.json
            set -- goodwin wake-model.onnx wake-model.json
            ;;
        core) set -- goodwin ;;
    esac
    sha256sum "$@" > SHA256SUMS
    tar -czf "$upload/payload.tar.gz" "$@" SHA256SUMS
    cd "$TEST_ROOT"
}

package initial
sh "$TEST_ROOT/install-test.sh" "$upload" keep | tee "$TEST_ROOT/fresh.out"
grep 'GOODWIN_INSTALL_OK fresh' "$TEST_ROOT/fresh.out"
data=$TEST_ROOT/data/opt/etc/goodwin
test "$(cat "$data/model")" = standard
echo retained > "$data/config.json"
mkdir "$data/plugins"
echo custom > "$data/models/custom.onnx"
package current
sh "$TEST_ROOT/install-test.sh" "$upload" micro | tee "$TEST_ROOT/update.out"
grep 'GOODWIN_INSTALL_OK update' "$TEST_ROOT/update.out"
test "$(cat "$TEST_ROOT/running")" = current
test "$(cat "$data/model")" = micro
install_init=$upload/init.sh
upload="$TEST_ROOT/tmp/.goodwin-upload.test02"
mkdir "$upload"
cp "$install_init" "$upload/init.sh"
models_inode=$(stat -c %i "$data/models")
package compact current
sh "$TEST_ROOT/install-test.sh" "$upload" keep | tee "$TEST_ROOT/compact.out"
grep 'GOODWIN_INSTALL_OK update' "$TEST_ROOT/compact.out"
test "$(cat "$TEST_ROOT/running")" = compact
test "$(stat -c %i "$data/models")" = "$models_inode"
test ! -e "$upload/payload"
for stage in "$TEST_ROOT/data/opt"/.goodwin-stage.*; do test ! -e "$stage"; done
package broken
if sh "$TEST_ROOT/install-test.sh" "$upload" standard > "$TEST_ROOT/failed.out" 2>&1; then
    echo "The update with a failed startup unexpectedly succeeded" >&2
    exit 1
fi
grep 'Restoring the previous state' "$TEST_ROOT/failed.out"
test "$(cat "$TEST_ROOT/running")" = compact
test "$(cat "$data/model")" = micro
test "$(cat "$data/models/standard.onnx")" = current
test "$(cat "$data/models/custom.onnx")" = custom
test "$(cat "$data/config.json")" = retained
test -d "$data/plugins"
test ! -e "$TEST_ROOT/data/opt/.goodwin-install-lock"

package catalog-first egor-micro catalog
sh "$TEST_ROOT/install-test.sh" "$upload" catalog
grep 'egor-micro' "$data/wake-model.json"
test "$(cat "$data/wake-models/$hash.onnx")" = egor-micro
selected_hash=$hash

package catalog-keep '' core
sh "$TEST_ROOT/install-test.sh" "$upload" keep
test "$(cat "$TEST_ROOT/running")" = catalog-keep
test "$(cat "$data/wake-models/$selected_hash.onnx")" = egor-micro

package broken goodwin-micro catalog
if sh "$TEST_ROOT/install-test.sh" "$upload" catalog > "$TEST_ROOT/failed-catalog.out" 2>&1; then
    echo "The wake model change with a failed startup unexpectedly succeeded" >&2
    exit 1
fi
grep 'Restoring the previous state' "$TEST_ROOT/failed-catalog.out"
test "$(cat "$TEST_ROOT/running")" = catalog-keep
grep 'egor-micro' "$data/wake-model.json"
test "$(cat "$data/wake-models/$selected_hash.onnx")" = egor-micro
test ! -e "$data/wake-models/$hash.onnx"
test "$(cat "$data/config.json")" = retained

hook=$TEST_ROOT/data/init.d/S96goodwin
"$hook" stop
mv "$hook" "$TEST_ROOT/previous-hook"
mv "$data" "$TEST_ROOT/previous-data"
mv "$TEST_ROOT/data/opt/bin/goodwin" "$TEST_ROOT/previous-bin"
package fresh-catalog goodwin-micro catalog
sh "$TEST_ROOT/install-test.sh" "$upload" catalog
grep 'goodwin-micro' "$data/wake-model.json"
test "$(cat "$data/wake-models/$hash.onnx")" = goodwin-micro
test ! -e "$data/models"

printf '%s\n' 'Bundled and catalog installs, core-only updates and rollback passed.'
