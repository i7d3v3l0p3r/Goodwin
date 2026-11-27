mod artwork;
mod catalog;
#[path = "../../../shared/http_tls.rs"]
mod http_tls;
mod player;
mod soloist;

use std::{sync::Arc, time::Duration};

use goodwin_plugin::{HttpRequest, HttpResponse, Plugin, ToolCall, settings::read_settings};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::{runtime::Runtime, sync::Mutex, task::JoinHandle, time::timeout};

use catalog::Catalog;

type Outcome<T> = Result<T, &'static str>;

#[derive(Default, Deserialize)]
#[serde(default)]
struct Settings {
    client_id: String,
    client_secret: String,
    market: String,
    soloist_url: String,
}

enum Action {
    Search { query: String, kind: String },
    Control(&'static str),
}

impl Action {
    fn parse(call: &ToolCall) -> Outcome<Self> {
        match call.name.as_str() {
            "play" => {
                let query = call.arguments["query"].as_str().unwrap_or_default().trim();
                let kind = call
                    .arguments
                    .get("kind")
                    .map_or(Some("track"), Value::as_str);
                let Some(kind @ ("track" | "album" | "playlist")) = kind else {
                    return Err("invalid_search_type");
                };
                if query.is_empty() || query.len() > 512 {
                    return Err("invalid_search_query");
                }
                Ok(Self::Search {
                    query: query.into(),
                    kind: kind.into(),
                })
            }
            "pause" => Ok(Self::Control("pause")),
            "resume" => Ok(Self::Control("play")),
            "next" => Ok(Self::Control("skip_next")),
            _ => Err("unknown_tool"),
        }
    }
}

async fn execute(catalog: &Mutex<Catalog>, settings: &Settings, action: Action) -> Outcome<()> {
    let url = soloist::address(&settings.soloist_url)?;
    match action {
        Action::Search { query, kind } => {
            let uri = catalog.lock().await.search(settings, &query, &kind).await?;
            soloist::command(&url, "play", Some(&uri)).await
        }
        Action::Control(command) => soloist::command(&url, command, None).await,
    }
}

#[derive(Default)]
struct Requests {
    pending: Option<JoinHandle<Outcome<()>>>,
    last: Option<Outcome<()>>,
}

impl Requests {
    async fn submit(&mut self, work: impl Future<Output = Outcome<()>> + Send + 'static) -> Value {
        if let Some(previous) = self.pending.take() {
            previous.abort();
            let _ = previous.await;
        }
        let mut job = tokio::spawn(async move {
            timeout(Duration::from_secs(15), work)
                .await
                .unwrap_or(Err("spotify_request_timed_out"))
        });
        // Native tool calls have a three-second host deadline. Keep the job
        // alive during a slow search and report its actual outcome on request.
        match timeout(Duration::from_millis(1500), &mut job).await {
            Ok(result) => self.last = Some(result.unwrap_or(Err("spotify_request_failed"))),
            Err(_) => self.pending = Some(job),
        }
        self.status().await
    }

    async fn status(&mut self) -> Value {
        if self.pending.as_ref().is_some_and(JoinHandle::is_finished)
            && let Some(job) = self.pending.take()
        {
            self.last = Some(job.await.unwrap_or(Err("spotify_request_failed")));
        }
        if self.pending.is_some() {
            return json!({"status": "pending"});
        }
        match self.last {
            Some(Ok(())) => json!({"status": "accepted_by_soloist"}),
            Some(Err(error)) => json!({"error": error}),
            None => json!({"status": "no_request"}),
        }
    }
}

struct Spotify {
    runtime: Runtime,
    client: reqwest::Client,
    catalog: Arc<Mutex<Catalog>>,
    requests: Requests,
}

impl Plugin for Spotify {
    fn registration(&self) -> Value {
        let empty = json!({"type": "object", "properties": {}, "required": [], "additionalProperties": false});
        let mut tools = vec![json!({
            "name": "play", "description": "Play music from Spotify when asked. Pass the song title and artist together as the search query. Use album or playlist only when requested. If the result is pending, say the request is in progress.",
            "parameters": {"type": "object", "properties": {
                "query": {"type": "string", "description": "Title and any artist specified by the listener"},
                "kind": {"type": "string", "description": "track (default), album or playlist"}
            }, "required": ["query"], "additionalProperties": false}
        })];
        for (name, description) in [
            ("pause", "Pause Spotify playback when requested."),
            (
                "resume",
                "Resume Spotify playback, including a request to play music without specifying a title.",
            ),
            ("next", "Skip to the next Spotify track when requested."),
            (
                "status",
                "Check whether the most recent Spotify command is pending, accepted or failed. This reports command status, not what is currently playing.",
            ),
        ] {
            tools.push(json!({"name": name, "description": description, "parameters": empty}));
        }
        json!({"apiVersion": 1, "tools": tools,
            "pages": [{"id":"main","path":"","navigation":true,"icon":"headphones","title":{"en":"Spotify","ru":"Spotify"}}],
            "routes": [{"method":"GET","path":"/player"},{"method":"POST","path":"/player/:command"},{"method":"GET","path":"/artwork/:id"}]
        })
    }

    fn http(&mut self, request: HttpRequest) -> goodwin_error::Result<HttpResponse> {
        let settings: Settings = read_settings()?;
        let result = self.runtime.block_on(async {
            let url = soloist::address(&settings.soloist_url)?;
            let work = async {
                match (request.method.as_str(), request.path.as_str()) {
                    ("GET", "/player") => player::snapshot(&url).await,
                    ("GET", path) if path.starts_with("/artwork/") => {
                        artwork::fetch(&self.client, &path["/artwork/".len()..]).await
                    }
                    ("POST", path) => match path.strip_prefix("/player/") {
                        Some(command @ ("play" | "pause" | "skip_next" | "skip_prev")) => {
                            soloist::command(&url, command, None).await?;
                            Ok(json!({"accepted":true}))
                        }
                        _ => Err("not_found"),
                    },
                    _ => Err("not_found"),
                }
            };
            timeout(Duration::from_secs(2), work)
                .await
                .unwrap_or(Err("soloist_timed_out"))
        });
        Ok(match result {
            Ok(value) => HttpResponse::json(200, value),
            Err(error) => {
                let status = match error {
                    "not_found" | "artwork_unavailable" => 404,
                    "select_soloist_in_spotify" => 409,
                    "invalid_soloist_address" => 422,
                    _ => 503,
                };
                HttpResponse::json(status, json!({"error":error}))
            }
        })
    }

    fn tool(&mut self, call: ToolCall) -> goodwin_error::Result<Value> {
        if call.name == "status" {
            return Ok(self.runtime.block_on(self.requests.status()));
        }
        let action = match Action::parse(&call) {
            Ok(action) => action,
            Err(error) => return Ok(json!({"error": error})),
        };
        let settings: Settings = read_settings()?;
        let catalog = self.catalog.clone();
        Ok(self.runtime.block_on(
            self.requests
                .submit(async move { execute(&catalog, &settings, action).await }),
        ))
    }
}

fn main() -> goodwin_error::Result<()> {
    let client = reqwest::Client::builder()
        .tls_backend_preconfigured(http_tls::client_config())
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(4))
        .timeout(Duration::from_secs(6))
        .build()?;
    let catalog = Catalog::new(
        client.clone(),
        "https://accounts.spotify.com/api/token",
        "https://api.spotify.com/v1/search",
    );
    goodwin_plugin::run(Spotify {
        client,
        runtime: tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()?,
        catalog: Arc::new(Mutex::new(catalog)),
        requests: Requests::default(),
    })
}

#[cfg(test)]
mod tests;
