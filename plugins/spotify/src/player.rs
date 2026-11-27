use serde_json::{Value, json};
use url::Url;

use crate::{Outcome, soloist};

const QUEUE_LIMIT: usize = 20;

fn label(value: &Value) -> Option<String> {
    value
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.chars().take(200).collect())
}

fn track(entity: &Value) -> Option<Value> {
    let uri = label(&entity["uri"]);
    let details = &entity["decorations"];
    let title = label(&details["identity"]["name"]);
    if title.is_none() && uri.is_none() {
        return None;
    }
    let artists: Vec<_> = details["creators"]
        .as_array()
        .into_iter()
        .flatten()
        .take(6)
        .filter_map(|creator| label(&creator["entity"]["decorations"]["identity"]["name"]))
        .collect();
    Some(json!({
        "uri": uri, "title": title, "artists": artists,
        "album": label(&details["parent"]["entity"]["decorations"]["identity"]["name"]),
        "duration": details["playback"]["duration_ms"].as_u64(),
    }))
}

fn entries(queue: &Value) -> Vec<Value> {
    queue
        .as_array()
        .into_iter()
        .flatten()
        .take(QUEUE_LIMIT)
        .filter_map(|entry| track(&entry["item"]))
        .collect()
}

pub(super) async fn snapshot(url: &Url) -> Outcome<Value> {
    let mut socket = soloist::connect(url).await?;
    let mut device = None;
    let mut playback = None;
    let mut queue = None;
    loop {
        let event = soloist::event(&mut socket).await?;
        match event["type"].as_str() {
            Some("auth_state") => {
                let name = label(&event["device_name"]);
                match event["logged_in"].as_bool() {
                    Some(false) => return Ok(json!({"connection":"pairing", "device":name})),
                    Some(true) if device.is_none() => {
                        device = Some(name);
                        // Soloist sends the full playback state on login/connection.
                        soloist::send(
                            &mut socket,
                            &json!({"type":"command","command":"get_queue","limit":QUEUE_LIMIT}),
                        )
                        .await?;
                    }
                    Some(true) => {}
                    None => return Err("invalid_soloist_response"),
                }
            }
            Some("playback_state") => {
                let actions = &event["available_actions"];
                let controls: Vec<_> = ["play", "pause", "skip_next", "skip_prev"]
                    .into_iter()
                    .filter(|key| actions.get(*key).is_some())
                    .collect();
                playback = Some(json!({
                    "status": label(&event["status"]), "active": event["is_active"].as_bool(),
                    "track": track(&event["item"]), "artwork": crate::artwork::id(&event["item"]),
                    "position": event["position"]["position_ms"].as_u64(),
                    "timestamp": event["position"]["timestamp_ms"].as_u64(),
                    "speed": event["position"]["speed"].as_f64(), "controls": controls,
                }));
            }
            Some("queue_changed") => {
                queue = Some((entries(&event["upcoming"]), entries(&event["previous"])));
            }
            Some("error") => return Err("soloist_rejected_command"),
            _ => {}
        }
        if let (Some(name), Some(player), Some((upcoming, previous))) = (&device, &playback, &queue)
        {
            return Ok(
                json!({"connection":"ready","device":name,"player":player,"upcoming":upcoming,"previous":previous}),
            );
        }
    }
}
