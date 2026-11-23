#!/bin/sh
set -eu
umask 077

bin=/data/opt/bin/goodwin
data=/data/opt/etc/goodwin
pidfile=/tmp/goodwin.pid
log=/tmp/goodwin.log

running() {
    [ -s "$pidfile" ] && start-stop-daemon -K -t -q -p "$pidfile" -x "$bin"
}

start() {
    running && return 0
    model=standard
    if [ -r "$data/model" ]; then read -r model < "$data/model"; fi
    case "$model" in
        standard|micro)
            if [ -r "$data/models/$model.onnx" ]; then
                export GOODWIN_WAKE_MODEL="$data/models/$model.onnx"
            fi
            ;;
    esac
    if [ ! -r "$data/config.json" ] || [ ! -x "$bin" ]; then
        echo "Goodwin executable or settings missing" >&2
        return 1
    fi
    start-stop-daemon -S -b -m -q -p "$pidfile" -x "$bin" -- \
        serve "$data" 0.0.0.0:8787 </dev/null >>"$log" 2>&1
}

stop() {
    if running; then
        start-stop-daemon -K -q -p "$pidfile" -x "$bin"
        for attempt in 1 2 3 4 5; do
            running || break
            sleep 1
        done
        if running; then
            echo "Goodwin is still running" >&2
            return 1
        fi
    fi
    rm -f "$pidfile"
}

case "${1:-start}" in
    start) start ;;
    stop) stop ;;
    restart) stop; start ;;
    status) running ;;
    *) echo "Usage: $0 {start|stop|restart|status}" >&2; exit 2 ;;
esac
