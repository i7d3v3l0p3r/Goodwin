#!/bin/sh
set -eu
TEST_ROOT=$(mktemp -d /tmp/goodwin-installer-check.XXXXXX)
case "$TEST_ROOT" in /tmp/goodwin-installer-check.??????) ;; *) exit 2 ;; esac
trap 'rm -rf "$TEST_ROOT"' 0
export TEST_ROOT
mkdir -p "$TEST_ROOT/tools" "$TEST_ROOT/bkupgrade/data/opt" "$TEST_ROOT/bkupgrade/data/init.d"
ln -s bkupgrade/data "$TEST_ROOT/data"
PATH="$TEST_ROOT/tools:$PATH"
export PATH
sed "s,/data/opt,$TEST_ROOT/data/opt,g; s,/data/init.d,$TEST_ROOT/data/init.d,g; s,/tmp/goodwin.log,$TEST_ROOT/goodwin.log,g" installer/install-station.sh > "$TEST_ROOT/install-test.sh"
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
    cat > "$TEST_ROOT/payload/goodwin" <<EOF
#!/bin/sh
case "\$1" in
    wake-check) test -f "\$2" ;;
    version) echo "$version" ;;
    setup) mkdir "\$2" && echo settings > "\$2/config.json" ;;
esac
EOF
    cd "$TEST_ROOT/payload"
    for file in standard.onnx micro.onnx LICENSE NOTICE; do
        echo "$version" > "models/$file"
    done
    sha256sum goodwin models/* > SHA256SUMS
    tar -czf "$upload/payload.tar.gz" goodwin models/* SHA256SUMS
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
package broken
if sh "$TEST_ROOT/install-test.sh" "$upload" standard > "$TEST_ROOT/failed.out" 2>&1; then
    echo "The update with a failed startup unexpectedly succeeded" >&2
    exit 1
fi
grep 'Restoring the previous state' "$TEST_ROOT/failed.out"
test "$(cat "$TEST_ROOT/running")" = current
test "$(cat "$data/model")" = micro
test "$(cat "$data/models/standard.onnx")" = current
test "$(cat "$data/models/custom.onnx")" = custom
test "$(cat "$data/config.json")" = retained
test -d "$data/plugins"
test ! -e "$TEST_ROOT/data/opt/.goodwin-install-lock"
printf '%s\n' 'Fresh installation, update and failed-update rollback passed.'
