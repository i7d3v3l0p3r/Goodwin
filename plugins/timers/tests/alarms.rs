use std::{
    io::{BufRead, BufReader, Write},
    path::Path,
    process::{Command, Stdio},
    time::{SystemTime, UNIX_EPOCH},
};

use serde_json::{Value, json};

fn poll(directory: &Path, played: Option<u64>) -> Value {
    let mut child = Command::new(env!("CARGO_BIN_EXE_goodwin-timers"))
        .env("GOODWIN_PLUGIN_DATA", directory)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    let output = BufReader::new(child.stdout.take().unwrap());
    writeln!(
        input,
        "{}",
        json!({"jsonrpc":"2.0","id":0,"method":"initialize",
        "params":{"apiVersion":1,"timerAlarms":true}})
    )
    .unwrap();
    writeln!(
        input,
        "{}",
        json!({"jsonrpc":"2.0","id":1,"method":"timer.poll",
        "params":{"played":played}})
    )
    .unwrap();
    drop(input);
    let mut replies = output
        .lines()
        .map(|line| serde_json::from_str::<Value>(&line.unwrap()).unwrap());
    assert_eq!(replies.next().unwrap()["result"]["timerAlarms"], true);
    let reply = replies.next().unwrap();
    assert_eq!(reply["id"], 1);
    assert!(child.wait().unwrap().success());
    reply["result"].clone()
}

#[test]
fn native_timer_replays_an_alarm_until_the_host_confirms_playback() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("timers.json");
    let expired = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
        - 1000;
    goodwin_plugin::save_json(
        &path,
        &json!({"next_id":1,"timers":[
            {"id":1,"seconds":1,"expiresAt":expired,"completed":false}
        ]}),
    )
    .unwrap();
    assert_eq!(poll(directory.path(), None)["alarm"], 1);
    assert_eq!(poll(directory.path(), None)["alarm"], 1);
    assert!(poll(directory.path(), Some(1))["alarm"].is_null());
    assert!(poll(directory.path(), None)["alarm"].is_null());
}
