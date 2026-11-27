use std::time::{Duration, Instant};

use reqwest::{Client, Response, StatusCode};
use serde::Deserialize;
use serde_json::Value;

use crate::{Outcome, Settings};

struct Token {
    client_id: String,
    client_secret: String,
    value: String,
    expires: Instant,
}

pub(crate) struct Catalog {
    client: Client,
    token_url: String,
    search_url: String,
    token: Option<Token>,
}

impl Catalog {
    pub(crate) fn new(client: Client, token_url: &str, search_url: &str) -> Self {
        Self {
            client,
            token_url: token_url.into(),
            search_url: search_url.into(),
            token: None,
        }
    }

    async fn authorize(&mut self, settings: &Settings) -> Outcome<&str> {
        if !self.token.as_ref().is_some_and(|token| {
            token.client_id == settings.client_id
                && token.client_secret == settings.client_secret
                && token.expires > Instant::now()
        }) {
            self.token = None;
            let response = self
                .client
                .post(&self.token_url)
                .basic_auth(&settings.client_id, Some(&settings.client_secret))
                .header(
                    reqwest::header::CONTENT_TYPE,
                    "application/x-www-form-urlencoded",
                )
                .body("grant_type=client_credentials")
                .send()
                .await
                .map_err(|_| "spotify_unreachable")?;
            if matches!(
                response.status(),
                StatusCode::BAD_REQUEST | StatusCode::UNAUTHORIZED
            ) {
                return Err("invalid_spotify_app_credentials");
            }
            #[derive(Deserialize)]
            struct Grant {
                access_token: String,
                expires_in: u64,
            }
            let grant: Grant = serde_json::from_slice(&body(response).await?)
                .map_err(|_| "invalid_spotify_response")?;
            if grant.access_token.is_empty() || grant.expires_in == 0 {
                return Err("invalid_spotify_response");
            }
            self.token = Some(Token {
                client_id: settings.client_id.clone(),
                client_secret: settings.client_secret.clone(),
                value: grant.access_token,
                expires: Instant::now()
                    + Duration::from_secs(grant.expires_in.min(86_400).saturating_sub(20)),
            });
        }
        self.token
            .as_ref()
            .map(|t| t.value.as_str())
            .ok_or("invalid_spotify_response")
    }

    pub(crate) async fn search(
        &mut self,
        settings: &Settings,
        query: &str,
        kind: &str,
    ) -> Outcome<String> {
        if settings.client_id.trim().is_empty() || settings.client_secret.is_empty() {
            return Err("configure_spotify_app_in_plugin_settings");
        }
        let market = settings.market.trim().to_ascii_uppercase();
        if market.len() != 2 || !market.bytes().all(|c| c.is_ascii_uppercase()) {
            return Err("configure_account_country_in_plugin_settings");
        }
        for attempt in 0..2 {
            let request = self.client.get(&self.search_url).query(&[
                ("q", query),
                ("type", kind),
                ("market", market.as_str()),
                ("limit", "5"),
            ]);
            let response = request
                .bearer_auth(self.authorize(settings).await?)
                .send()
                .await
                .map_err(|_| "spotify_unreachable")?;
            if response.status() == StatusCode::UNAUTHORIZED && attempt == 0 {
                self.token = None;
                continue;
            }
            let result: Value = serde_json::from_slice(&body(response).await?)
                .map_err(|_| "invalid_spotify_response")?;
            let items = result[format!("{kind}s")]["items"]
                .as_array()
                .ok_or("invalid_spotify_response")?;
            let prefix = format!("spotify:{kind}:");
            return items
                .iter()
                .filter(|item| {
                    item["is_playable"] != false && item["restrictions"]["reason"].is_null()
                })
                .filter_map(|item| item["uri"].as_str())
                .find(|uri| {
                    uri.strip_prefix(&prefix).is_some_and(|id| {
                        id.len() == 22 && id.bytes().all(|b| b.is_ascii_alphanumeric())
                    })
                })
                .map(str::to_owned)
                .ok_or("no_playable_match");
        }
        Err("invalid_spotify_app_credentials")
    }
}

async fn body(mut response: Response) -> Outcome<Vec<u8>> {
    match response.status() {
        StatusCode::OK => {}
        StatusCode::UNAUTHORIZED => return Err("invalid_spotify_app_credentials"),
        StatusCode::FORBIDDEN => return Err("spotify_app_access_denied"),
        StatusCode::TOO_MANY_REQUESTS => return Err("spotify_rate_limited"),
        _ => return Err("spotify_api_failed"),
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| "spotify_unreachable")? {
        if chunk.len() > 256 * 1024 - bytes.len() {
            return Err("invalid_spotify_response");
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}
