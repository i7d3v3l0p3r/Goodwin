mod collector;
mod files;
mod store;

use std::{
    ops::Bound::{Excluded, Unbounded},
    path::{Path, PathBuf},
};

use base64::{Engine, engine::general_purpose::STANDARD};
use goodwin_error::{Result, ensure};
use goodwin_plugin::{HttpRequest, HttpResponse, Plugin, wake::Observation};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use collector::Collector;
use store::{Clip, DEFAULT_STORAGE_MIB, LIMIT, Label, MAX_STORAGE_MIB, Store};

const PAGE_SIZE: usize = 20;

#[derive(Deserialize, Serialize)]
struct CaptureSettings {
    minimum_score: f64,
    #[serde(default = "default_storage_mib")]
    storage_mib: usize,
}

fn default_storage_mib() -> usize {
    DEFAULT_STORAGE_MIB
}

impl Default for CaptureSettings {
    fn default() -> Self {
        Self {
            minimum_score: 0.1,
            storage_mib: DEFAULT_STORAGE_MIB,
        }
    }
}

struct Dataset {
    store: Store,
    collector: Collector,
    settings: CaptureSettings,
    settings_path: PathBuf,
    active: bool,
    session: u64,
    storage_failed: bool,
}

impl Dataset {
    fn open(root: &Path) -> Result<Self> {
        let mut store = Store::open(&root.join("clips"), files::storage_limit())?;
        let settings_path = root.join("capture.json");
        let bytes = files::read(&settings_path, 0, 8192, true)?;
        let settings: CaptureSettings = if bytes.is_empty() {
            Default::default()
        } else {
            goodwin_plugin::json_codec::from_slice(&bytes)?
        };
        ensure!(
            (0.0..=1.0).contains(&settings.minimum_score),
            "Invalid minimum wake score"
        );
        ensure!(
            (1..=MAX_STORAGE_MIB).contains(&settings.storage_mib),
            "Invalid wake dataset storage limit"
        );
        store.set_storage_mib(settings.storage_mib, bytes.len());
        Ok(Self {
            store,
            collector: Collector::default(),
            settings,
            settings_path,
            active: false,
            session: 0,
            storage_failed: false,
        })
    }

    fn status(&self) -> Value {
        let mut pending = 0;
        let mut positive = 0;
        let mut negative = 0;
        for clip in self.store.clips.values() {
            match clip.label {
                Label::Pending => pending += 1,
                Label::Positive => positive += 1,
                Label::Negative => negative += 1,
            }
        }
        json!({
            "collecting": self.active,
            "storageFailed": self.storage_failed,
            "storageFull": self.store.full(),
            "storageMiB": self.settings.storage_mib,
            "maxStorageMiB": self.store.max_storage_mib(),
            "storageLimitBytes": self.store.byte_limit(),
            "usedBytes": self.store.used_bytes,
            "limit": LIMIT,
            "minimumScore": self.settings.minimum_score,
            "pending": pending,
            "positive": positive,
            "negative": negative,
        })
    }

    fn list(&self, query: &str) -> Option<Value> {
        let mut filter = "pending";
        let mut after = 0;
        for item in query.split('&').filter(|item| !item.is_empty()) {
            match item.split_once('=')? {
                ("filter", value) => filter = value,
                ("after", value) => after = value.parse().ok()?,
                _ => return None,
            }
        }
        let matches: fn(&&Clip) -> bool = match filter {
            "pending" => |clip: &&Clip| clip.label == Label::Pending,
            "positive" => |clip: &&Clip| clip.label == Label::Positive,
            "negative" => |clip: &&Clip| clip.label == Label::Negative,
            "labelled" => |clip: &&Clip| clip.label != Label::Pending,
            _ => return None,
        };
        let mut clips = self
            .store
            .clips
            .range((Excluded(after), Unbounded))
            .map(|(_, clip)| clip)
            .filter(matches);
        let page: Vec<_> = clips.by_ref().take(PAGE_SIZE).collect();
        let next = clips.next().and_then(|_| page.last().map(|clip| clip.id));
        Some(json!({"clips": page, "next": next, "status": self.status()}))
    }

    fn collect(&mut self, body: &Value) -> Result<HttpResponse> {
        let Some(enabled) = body["enabled"].as_bool() else {
            return Ok(invalid());
        };
        if enabled && self.store.full() {
            return Ok(HttpResponse::json(409, json!({"error": "dataset_full"})));
        }
        if enabled && !self.active {
            self.session += 1;
            self.storage_failed = false;
        }
        if !enabled && self.active {
            self.active = false;
            let result = self
                .collector
                .flush(&mut self.store, self.settings.minimum_score as f32);
            self.record_result(result)?;
        }
        self.active = enabled;
        Ok(HttpResponse::json(200, self.status()))
    }

    fn set_capture_settings(&mut self, body: &Value) -> Result<HttpResponse> {
        if body.get("minimumScore").is_none() && body.get("storageMiB").is_none() {
            return Ok(invalid());
        }
        let minimum_score = match body.get("minimumScore") {
            Some(value) => {
                let Some(score) = value.as_f64().filter(|score| (0.0..=1.0).contains(score)) else {
                    return Ok(invalid());
                };
                score
            }
            None => self.settings.minimum_score,
        };
        let storage_mib = match body.get("storageMiB") {
            Some(value) => {
                let Some(limit) = value
                    .as_u64()
                    .and_then(|limit| usize::try_from(limit).ok())
                    .filter(|limit| (1..=self.store.max_storage_mib()).contains(limit))
                else {
                    return Ok(invalid());
                };
                limit
            }
            None => self.settings.storage_mib,
        };
        let settings = CaptureSettings {
            minimum_score,
            storage_mib,
        };
        let encoded = goodwin_plugin::json_codec::to_vec(&settings)?;
        files::write(&self.settings_path, &encoded, true)?;
        self.store.set_storage_mib(storage_mib, encoded.len());
        self.settings = settings;
        self.record_result(Ok(()))?;
        Ok(HttpResponse::empty(204))
    }

    fn delete_all(&mut self) -> Result<HttpResponse> {
        self.active = false;
        self.collector = Collector::default();
        self.store.remove_all()?;
        self.storage_failed = false;
        Ok(HttpResponse::empty(204))
    }

    fn record_result(&mut self, result: Result<()>) -> Result<()> {
        if result.is_err() {
            self.storage_failed = true;
        }
        if self.storage_failed || self.store.full() {
            self.active = false;
            self.collector = Collector::default();
        }
        result
    }
}

impl Plugin for Dataset {
    fn registration(&self) -> Value {
        json!({
            "apiVersion": 1,
            "pages": [{
                "id": "dataset", "path": "", "navigation": true, "icon": "audio-lines",
                "title": {"en": "Wake-word dataset", "ru": "Датасет слова активации"},
            }],
            "routes": [
                {"method": "GET", "path": "/clips", "password_required": true},
                {"method": "DELETE", "path": "/clips", "password_required": true},
                {"method": "GET", "path": "/clips/:id/audio", "password_required": true},
                {"method": "PUT", "path": "/clips/:id", "password_required": true},
                {"method": "DELETE", "path": "/clips/:id", "password_required": true},
                {"method": "PUT", "path": "/capture", "password_required": true},
                {"method": "PUT", "path": "/capture-settings", "password_required": true},
            ],
        })
    }

    fn supports_wake_events(&self) -> bool {
        true
    }

    fn wake_subscription(&self) -> u64 {
        if self.active { self.session } else { 0 }
    }

    fn wake_event(&mut self, observation: Observation) -> Result<()> {
        if !self.active || observation.subscription != self.session {
            return Ok(());
        }
        let result = self.collector.observe(
            &mut self.store,
            observation.event,
            self.settings.minimum_score as f32,
        );
        self.record_result(result)
    }

    fn http(&mut self, request: HttpRequest) -> Result<HttpResponse> {
        match (request.method.as_str(), request.path.as_str()) {
            ("GET", "/clips") => {
                return Ok(self
                    .list(request.query.as_deref().unwrap_or_default())
                    .map_or_else(invalid, |page| HttpResponse::json(200, page)));
            }
            ("DELETE", "/clips") => return self.delete_all(),
            ("PUT", "/capture") => return self.collect(&request.body),
            ("PUT", "/capture-settings") => return self.set_capture_settings(&request.body),
            _ => {}
        }
        let Some(path) = request.path.strip_prefix("/clips/") else {
            return Ok(missing());
        };
        let (id, audio) = path
            .strip_suffix("/audio")
            .map_or((path, false), |id| (id, true));
        let Some(clip) = id
            .parse::<u64>()
            .ok()
            .and_then(|id| self.store.clips.get(&id))
        else {
            return Ok(missing());
        };
        match (request.method.as_str(), audio) {
            ("GET", true) => Ok(HttpResponse::json(
                200,
                json!({
                    "wav": STANDARD.encode(self.store.wav(clip)?),
                }),
            )),
            ("PUT", false) => {
                let Ok(label) = serde_json::from_value(request.body["label"].clone()) else {
                    return Ok(invalid());
                };
                self.store.label(clip.id, label)?;
                Ok(HttpResponse::empty(204))
            }
            ("DELETE", false) => {
                self.store.remove(clip.id)?;
                Ok(HttpResponse::empty(204))
            }
            _ => Ok(missing()),
        }
    }
}

fn invalid() -> HttpResponse {
    HttpResponse::json(400, json!({"error": "invalid_dataset_request"}))
}

fn missing() -> HttpResponse {
    HttpResponse::json(404, json!({"error": "wake_sample_not_found"}))
}

#[cfg(not(target_arch = "wasm32"))]
fn main() -> Result<()> {
    goodwin_plugin::run(Dataset::open(&goodwin_plugin::data_dir()?)?)
}

#[cfg(target_arch = "wasm32")]
fn open() -> Result<Dataset> {
    Dataset::open(Path::new(""))
}

#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(target_arch = "wasm32")]
goodwin_plugin::wasm_plugin!(Dataset, open);

#[cfg(test)]
mod tests {
    use goodwin_plugin::wake::{Event, Frame, Model};

    use super::*;

    fn dataset(root: &Path) -> Dataset {
        Dataset::open(root).unwrap()
    }

    fn frame(subscription: u64, index: u64, sample: i16, score: f32) -> Observation {
        Observation {
            subscription,
            event: Event::Frame(Frame {
                stream: 1,
                index,
                model: Model {
                    id: "fixture".into(),
                    wake_word: "Goodwin".into(),
                    kind: "LiveKit Standard".into(),
                    sha256: "0".repeat(64),
                },
                ended_at_ms: 1000 + index * 80,
                sample_rate: 16_000,
                gain: 2.0,
                threshold: 0.5,
                score,
                pcm: STANDARD.encode(sample.to_le_bytes().repeat(1280)),
            }),
        }
    }

    fn request(dataset: &mut Dataset, method: &str, path: &str, body: Value) -> Value {
        serde_json::to_value(
            dataset
                .http(HttpRequest {
                    method: method.into(),
                    path: path.into(),
                    query: None,
                    body,
                })
                .unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn captures_both_sides_of_the_threshold_and_reopens_manually_labelled_audio() {
        let dir = tempfile::tempdir().unwrap();
        let mut collector = dataset(dir.path());
        collector.wake_event(frame(0, 0, 100, 1.0)).unwrap();
        assert!(collector.store.clips.is_empty());
        collector.collect(&json!({"enabled": true})).unwrap();
        let subscription = collector.wake_subscription();
        for i in 0..32 {
            collector
                .wake_event(frame(subscription, i, (i + 100) as i16, 0.125))
                .unwrap();
        }
        collector
            .wake_event(frame(subscription, 32, 20_000, 0.875))
            .unwrap();
        collector
            .wake_event(Observation {
                subscription,
                event: Event::End { stream: 1 },
            })
            .unwrap();
        collector.collect(&json!({"enabled": false})).unwrap();
        assert_eq!(collector.wake_subscription(), 0);
        collector
            .wake_event(frame(subscription, 33, 300, 0.25))
            .unwrap();
        let pending = collector.list("").unwrap();
        assert_eq!(pending["status"]["pending"], 2);
        assert_eq!(pending["clips"][0]["peak"], 0.125);
        assert_eq!(pending["clips"][1]["peak"], 0.875);
        let ids: Vec<_> = pending["clips"]
            .as_array()
            .unwrap()
            .iter()
            .map(|clip| clip["id"].as_u64().unwrap())
            .collect();

        // Deliberately label the low score positive and high score negative.
        for (id, label) in ids.iter().zip(["positive", "negative"]) {
            assert_eq!(
                request(
                    &mut collector,
                    "PUT",
                    &format!("/clips/{id}"),
                    json!({"label": label})
                )["status"],
                204
            );
        }
        drop(collector);
        let mut collector = dataset(dir.path());
        assert_eq!(collector.wake_subscription(), 0);
        let labelled = collector.list("filter=labelled&after=0").unwrap();
        assert_eq!(labelled["clips"][0]["label"], "positive");
        assert_eq!(labelled["clips"][1]["label"], "negative");
        assert_eq!(labelled["clips"][0]["info"]["gain"], 2.0);
        assert_eq!(labelled["clips"][0]["info"]["gainApplied"], true);
        let [first, second] = ids[..] else {
            panic!("expected two clips")
        };
        let mut wav = |id| {
            STANDARD
                .decode(
                    request(
                        &mut collector,
                        "GET",
                        &format!("/clips/{id}/audio"),
                        Value::Null,
                    )["body"]["wav"]
                        .as_str()
                        .unwrap(),
                )
                .unwrap()
        };
        let low = wav(first);
        let high = wav(second);
        let frame_bytes = 1280 * 2;
        assert_eq!(&low[..4], b"RIFF");
        assert_eq!(low.len(), 44 + 32 * frame_bytes);
        assert_eq!(high.len(), 44 + 17 * frame_bytes);
        assert_eq!(
            &low[44..44 + frame_bytes],
            &200_i16.to_le_bytes().repeat(1280)
        );
        assert_eq!(
            &low[44 + 16 * frame_bytes..],
            &high[44..44 + 16 * frame_bytes]
        );
        assert_eq!(
            &high[high.len() - frame_bytes..],
            &i16::MAX.to_le_bytes().repeat(1280)
        );
        assert_eq!(
            request(
                &mut collector,
                "DELETE",
                &format!("/clips/{first}"),
                Value::Null
            )["status"],
            204
        );
        assert_eq!(
            collector.list("filter=labelled&after=0").unwrap()["status"]["positive"],
            0
        );
    }

    #[test]
    fn a_gap_ends_the_clip_and_stopping_saves_the_remaining_frames() {
        let dir = tempfile::tempdir().unwrap();
        let mut collector = dataset(dir.path());
        collector.collect(&json!({"enabled": true})).unwrap();
        let subscription = collector.wake_subscription();
        collector
            .wake_event(frame(subscription, 5, 100, 0.1))
            .unwrap();
        collector
            .wake_event(frame(subscription, 7, 200, 0.2))
            .unwrap();
        assert_eq!(collector.store.clips.len(), 1);
        collector.collect(&json!({"enabled": false})).unwrap();
        assert_eq!(collector.store.clips.len(), 2);
        let scores: Vec<_> = collector
            .store
            .clips
            .values()
            .map(|clip| clip.info.scores.as_slice())
            .collect();
        assert_eq!(scores, [&[0.1], &[0.2]]);
    }

    #[test]
    fn only_new_scores_at_or_above_the_minimum_save_a_clip() {
        let dir = tempfile::tempdir().unwrap();
        let mut collector = dataset(dir.path());
        collector.collect(&json!({"enabled": true})).unwrap();
        let subscription = collector.wake_subscription();
        for index in 0..68 {
            let score = if index == 36 { 0.1 } else { 0.05 };
            collector
                .wake_event(frame(subscription, index, 100, score))
                .unwrap();
            if index == 31 {
                assert!(collector.store.clips.is_empty());
            }
        }
        collector.collect(&json!({"enabled": false})).unwrap();
        let clips = &collector.store.clips;
        assert_eq!(clips.len(), 1);
        let clip = clips.values().next().unwrap();
        assert_eq!(clip.peak, 0.1);
        assert_eq!(clip.info.scores[..20], [0.05; 20]);
        assert_eq!(clip.info.scores[20], 0.1);
    }

    #[test]
    fn deleting_all_stops_collection_and_preserves_the_saved_minimum() {
        let dir = tempfile::tempdir().unwrap();
        let mut collector = dataset(dir.path());
        assert_eq!(
            request(
                &mut collector,
                "PUT",
                "/capture-settings",
                json!({"minimumScore": -0.1})
            )["status"],
            400
        );
        assert_eq!(
            request(
                &mut collector,
                "PUT",
                "/capture-settings",
                json!({"minimumScore": 0})
            )["status"],
            204
        );
        collector.collect(&json!({"enabled": true})).unwrap();
        let subscription = collector.wake_subscription();
        for index in 0..3 {
            collector
                .wake_event(frame(subscription, index, 100, 0.0))
                .unwrap();
            collector
                .wake_event(Observation {
                    subscription,
                    event: Event::End { stream: 1 },
                })
                .unwrap();
        }
        assert_eq!(collector.store.clips.len(), 3);
        let ids: Vec<_> = collector.store.clips.keys().copied().collect();
        collector.store.label(ids[0], Label::Positive).unwrap();
        collector.store.label(ids[1], Label::Negative).unwrap();
        collector
            .wake_event(frame(subscription, 4, 100, 0.9))
            .unwrap();

        assert_eq!(
            request(&mut collector, "DELETE", "/clips", Value::Null)["status"],
            204
        );
        assert_eq!(collector.wake_subscription(), 0);
        collector
            .wake_event(frame(subscription, 5, 100, 0.9))
            .unwrap();
        assert!(collector.store.clips.is_empty());
        drop(collector);

        let collector = dataset(dir.path());
        assert!(collector.store.clips.is_empty());
        assert_eq!(collector.settings.minimum_score, 0.0);
    }

    #[test]
    fn loads_legacy_settings_and_saves_the_quota_independently() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("capture.json"),
            br#"{"minimum_score":0.42}"#,
        )
        .unwrap();
        let mut collector = dataset(dir.path());
        assert_eq!(collector.status()["storageMiB"], DEFAULT_STORAGE_MIB);
        assert_eq!(collector.status()["maxStorageMiB"], MAX_STORAGE_MIB);
        assert_eq!(
            request(
                &mut collector,
                "PUT",
                "/capture-settings",
                json!({"storageMiB": 6})
            )["status"],
            204
        );
        assert_eq!(
            request(
                &mut collector,
                "PUT",
                "/capture-settings",
                json!({"storageMiB": 0})
            )["status"],
            400
        );
        let reopened = dataset(dir.path());
        assert_eq!(reopened.status()["storageMiB"], 6);
        assert_eq!(reopened.status()["storageLimitBytes"], 6 * store::MIB);
        assert_eq!(reopened.status()["minimumScore"], 0.42);
    }
}
