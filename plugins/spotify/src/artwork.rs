use base64::{Engine, engine::general_purpose::STANDARD};
use reqwest::Client;
use serde_json::{Value, json};

use crate::Outcome;

const CDN: &str = "https://i.scdn.co/image/";
// Leave room for Base64 and JSON in the host's 128 KiB RPC frame.
const MAX_BYTES: usize = 72 * 1024;

fn valid_id(id: &str) -> bool {
    (32..=64).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_hexdigit())
}

pub(super) fn id(entity: &Value) -> Option<&str> {
    [entity, &entity["decorations"]["parent"]["entity"]]
        .into_iter()
        .find_map(|item| {
            item.pointer("/decorations/visual_identity/cover")?
                .as_array()?
                .iter()
                .filter_map(|cover| {
                    let id = cover["url"].as_str()?.strip_prefix(CDN)?;
                    let rank = match cover["size"].as_str() {
                        Some("default") => 0,
                        Some("small") => 1,
                        _ => 2,
                    };
                    valid_id(id).then_some((rank, id))
                })
                .min_by_key(|(rank, _)| *rank)
                .map(|(_, id)| id)
        })
}

pub(super) async fn fetch(client: &Client, id: &str) -> Outcome<Value> {
    if !valid_id(id) {
        return Err("not_found");
    }
    let mut response = client
        .get(format!("{CDN}{id}"))
        .send()
        .await
        .map_err(|_| "artwork_unavailable")?;
    if response.status() != reqwest::StatusCode::OK
        || response
            .content_length()
            .is_some_and(|n| n > MAX_BYTES as u64)
    {
        return Err("artwork_unavailable");
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| "artwork_unavailable")? {
        if chunk.len() > MAX_BYTES - bytes.len() {
            return Err("artwork_unavailable");
        }
        bytes.extend_from_slice(&chunk);
    }
    let mime = if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        "image/jpeg"
    } else if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        "image/png"
    } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        "image/webp"
    } else {
        return Err("artwork_unavailable");
    };
    Ok(json!({"image": format!("data:{mime};base64,{}", STANDARD.encode(bytes))}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn displayed_art_uses_a_spotify_thumbnail_and_falls_back_to_the_album() {
        let default = "abcd".repeat(10);
        let small = "1234".repeat(10);
        let covers = json!([
            {"size":"default","url":format!("https://other.invalid/image/{default}")},
            {"size":"small","url":format!("{CDN}{small}")},
            {"size":"default","url":format!("{CDN}{default}")},
        ]);
        let album = json!({"decorations":{"visual_identity":{"cover":covers}}});
        let item = json!({"decorations":{"parent":{"entity":album}}});
        assert_eq!(id(&item), Some(default.as_str()));
        assert!(!valid_id(&format!("{small}?q=1")));
    }
}
