#![cfg_attr(target_arch = "wasm32", no_std)]
#![cfg_attr(target_arch = "wasm32", no_main)]

#[cfg(target_arch = "wasm32")]
extern crate alloc;

mod state;
#[cfg(target_arch = "wasm32")]
mod wasm;

#[cfg(target_arch = "wasm32")]
use alloc::vec::Vec;
#[cfg(not(target_arch = "wasm32"))]
use std::{sync::Arc, thread};

use anyhow::Result;
#[cfg(not(target_arch = "wasm32"))]
use goodwin_plugin::{Host, data_dir};
use goodwin_plugin::{HttpRequest, HttpResponse, Plugin, ToolCall};
use serde_json::{Value, json};
use state::{Clock, Filter, Timer, now};

struct Timers {
    #[cfg(not(target_arch = "wasm32"))]
    clock: Arc<Clock>,
    #[cfg(target_arch = "wasm32")]
    clock: Clock,
    choices: Option<(u64, Vec<u64>)>,
}

impl Plugin for Timers {
    fn registration(&self) -> Value {
        json!({
            "apiVersion": 1,
            "voiceContextTool": "list",
            "pages": [{
                "id": "main", "path": "", "navigation": true, "icon": "timer",
                "title": {"en": "Timers", "ru": "Таймеры"},
            }],
            "routes": [
                {"method": "GET", "path": "/timers"}, {"method": "POST", "path": "/timers"},
                {"method": "DELETE", "path": "/timers/:id"}
            ],
            "tools": [
                {"name": "create", "description": "Start a new countdown timer when the user explicitly asks to set or start one. Convert its duration to whole seconds.", "parameters": {
                    "type": "object", "properties": {"seconds": {"type": "integer"}, "name": {"type": "string", "description": "Optional timer name supplied by the user"}},
                    "required": ["seconds"], "additionalProperties": false
                }},
                {"name": "list", "description": "List running timers and how much time remains when asked about existing timers.", "parameters": {
                    "type": "object", "properties": {}, "required": [], "additionalProperties": false
                }},
                {"name": "cancel", "description": "Cancel a running timer. Match the meaning of the user's reference to an active timer in plugin context, including grammatical case, prepositions and omitted words. Once the target is clear, use the saved name verbatim. Call with {} when the user gives no identifier. The tool cancels the only matching timer or returns numbered choices. Use seconds for the original duration, name for a name, or option for a number explicitly chosen by the user. The tool checks whether a numbered choice is pending. If a name lookup returns active timers, retry with the exact stored name only if one entry clearly refers to the same target. Keep any original duration filter. Ask the user if the intended timer is ambiguous.", "parameters": {
                    "type": "object", "properties": {
                        "name": {"type": "string", "description": "The stored name of the matching timer. Use the user's wording when the list is unavailable."},
                        "seconds": {"type": "integer", "description": "The original duration in seconds, never the remaining time"},
                        "option": {"type": "integer", "description": "The number explicitly chosen by the user after a numbered clarification"}
                    }, "required": [], "additionalProperties": false
                }}
            ]
        })
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn ready(&mut self, host: Host) {
        let clock = self.clock.clone();
        thread::spawn(move || clock.schedule(host));
    }

    fn http(&mut self, request: HttpRequest) -> Result<HttpResponse> {
        match (request.method.as_str(), request.path.as_str()) {
            ("GET", "/timers") => Ok(HttpResponse::json(200, self.clock.list())),
            ("POST", "/timers") => match self.clock.create(&request.body)? {
                Ok(timer) => Ok(HttpResponse::json(201, json!(timer))),
                Err("timer_limit") => Ok(HttpResponse::json(409, json!({"error": "timer_limit"}))),
                Err(error) => Ok(HttpResponse::json(400, json!({"error": error}))),
            },
            ("DELETE", path) => {
                let id = path.strip_prefix("/timers/").and_then(|id| id.parse().ok());
                if let Some(id) = id
                    && self.clock.cancel(id)?.is_some()
                {
                    return Ok(HttpResponse::empty(204));
                }
                Ok(HttpResponse::json(
                    404,
                    json!({"error": "timer_not_active"}),
                ))
            }
            _ => Ok(HttpResponse::json(404, json!({"error": "not_found"}))),
        }
    }

    fn tool(&mut self, call: ToolCall) -> Result<Value> {
        match call.name.as_str() {
            "create" => Ok(match self.clock.create(&call.arguments)? {
                Ok(timer) => json!({"created": timer.voice(now())}),
                Err(error) => json!({"error": error}),
            }),
            "list" => Ok(
                json!({"active": self.clock.matches(&Filter::default()).iter().map(|timer| timer.voice(now())).collect::<Vec<_>>() }),
            ),
            "cancel" => self.cancel(call),
            _ => Ok(json!({"error": "unknown_tool"})),
        }
    }
}

impl Timers {
    fn cancel(&mut self, call: ToolCall) -> Result<Value> {
        let args = &call.arguments;
        if let Some(option) = args.get("option") {
            if args.get("name").is_some() || args.get("seconds").is_some() {
                return Ok(json!({"error": "invalid_timer_choice"}));
            }
            let Some((_, choices)) = self
                .choices
                .as_ref()
                .filter(|(turn, _)| turn.checked_add(1) == Some(call.voice_turn))
            else {
                return Ok(json!({"error": "no_pending_timer_choice"}));
            };
            let id = option
                .as_u64()
                .and_then(|number| number.checked_sub(1))
                .and_then(|index| usize::try_from(index).ok())
                .and_then(|index| choices.get(index))
                .copied();
            self.choices = None;
            return match id {
                Some(id) => self.cancel_id(id),
                None => Ok(json!({"error": "invalid_timer_choice"})),
            };
        }
        if self
            .choices
            .as_ref()
            .is_some_and(|(turn, _)| *turn == call.voice_turn)
        {
            return Ok(json!({"error": "clarification_required"}));
        }
        self.choices = None;
        let Some(filter) = Filter::parse(args) else {
            return Ok(json!({"error": "invalid_timer_filter"}));
        };
        let matches = self.clock.matches(&filter);
        match matches.as_slice() {
            [] if args.get("name").is_some() => {
                let time = now();
                let active: Vec<_> = self
                    .clock
                    .matches(&filter.without_name())
                    .iter()
                    .map(|timer| timer.voice(time))
                    .collect();
                Ok(json!({"error": "timer_not_active", "active": active}))
            }
            [] => Ok(json!({"error": "timer_not_active"})),
            [timer] => self.cancel_id(timer.id),
            _ => {
                self.choices = Some((
                    call.voice_turn,
                    matches.iter().map(|timer| timer.id).collect(),
                ));
                let time = now();
                let options: Vec<_> = matches
                    .iter()
                    .enumerate()
                    .map(|(index, timer)| json!({"option": index + 1, "timer": timer.voice(time)}))
                    .collect();
                Ok(json!({"error": "clarification_required", "options": options}))
            }
        }
    }

    fn cancel_id(&self, id: u64) -> Result<Value> {
        Ok(match self.clock.cancel(id)? {
            Some(timer) => json!({"cancelled": Timer::voice(&timer, now())}),
            None => json!({"error": "timer_not_active"}),
        })
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn main() -> Result<()> {
    let clock = Arc::new(Clock::load(data_dir()?.join("timers.json"))?);
    goodwin_plugin::run(Timers {
        clock,
        choices: None,
    })
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    fn cancel(plugin: &mut Timers, voice_turn: u64, arguments: Value) -> Value {
        plugin
            .tool(ToolCall {
                name: "cancel".into(),
                arguments,
                voice_turn,
            })
            .unwrap()
    }

    #[test]
    fn numbered_cancellation_uses_the_original_timer_ids() {
        let dir = tempfile::tempdir().unwrap();
        let clock = Arc::new(Clock::load(dir.path().join("timers.json")).unwrap());
        let first = clock.create(&json!({"seconds": 60})).unwrap().unwrap();
        let second = clock.create(&json!({"seconds": 60})).unwrap().unwrap();
        let mut plugin = Timers {
            clock,
            choices: None,
        };
        let result = cancel(&mut plugin, 1, json!({}));
        assert_eq!(result["error"], "clarification_required");
        assert_eq!(result["options"][1]["option"], 2);
        assert_eq!(
            cancel(&mut plugin, 1, json!({"option": 2}))["error"],
            "no_pending_timer_choice"
        );
        plugin.clock.cancel(second.id).unwrap();
        assert_eq!(
            cancel(&mut plugin, 2, json!({"option": 2}))["error"],
            "timer_not_active"
        );
        assert_eq!(plugin.clock.matches(&Filter::default())[0].id, first.id);
        assert_eq!(
            cancel(&mut plugin, 3, json!({}))["cancelled"]["seconds"],
            60
        );
        assert!(plugin.clock.matches(&Filter::default()).is_empty());
    }

    #[test]
    fn choices_expire_after_the_next_voice_request() {
        let dir = tempfile::tempdir().unwrap();
        let clock = Arc::new(Clock::load(dir.path().join("timers.json")).unwrap());
        clock.create(&json!({"seconds": 60})).unwrap().unwrap();
        let second = clock.create(&json!({"seconds": 60})).unwrap().unwrap();
        let mut plugin = Timers {
            clock,
            choices: None,
        };
        cancel(&mut plugin, 5, json!({}));
        assert_eq!(
            cancel(&mut plugin, 7, json!({"option": 2}))["error"],
            "no_pending_timer_choice"
        );
        cancel(&mut plugin, 8, json!({"seconds": 60}));
        assert_eq!(
            cancel(&mut plugin, 9, json!({"option": 2}))["cancelled"]["seconds"],
            60
        );
        let remaining = plugin.clock.matches(&Filter::default());
        assert_eq!(remaining.len(), 1);
        assert_ne!(remaining[0].id, second.id);
    }

    #[test]
    fn spoken_filters_cancel_only_the_matching_timer() {
        let dir = tempfile::tempdir().unwrap();
        let clock = Arc::new(Clock::load(dir.path().join("timers.json")).unwrap());
        clock.create(&json!({"seconds": 300})).unwrap().unwrap();
        clock
            .create(&json!({"name": "Чай", "seconds": 120}))
            .unwrap()
            .unwrap();
        let mut plugin = Timers {
            clock,
            choices: None,
        };

        assert_eq!(
            cancel(&mut plugin, 1, json!({"seconds": 300}))["cancelled"]["seconds"],
            300
        );
        assert_eq!(
            cancel(&mut plugin, 2, json!({"name": "чай"}))["cancelled"]["name"],
            "Чай"
        );
        assert!(plugin.clock.matches(&Filter::default()).is_empty());
    }

    #[test]
    fn cancellation_can_retry_with_the_saved_name_and_keeps_other_timers() {
        let dir = tempfile::tempdir().unwrap();
        let clock = Arc::new(Clock::load(dir.path().join("timers.json")).unwrap());
        for (name, seconds) in [("для яиц", 120), ("для картошки", 300)] {
            clock
                .create(&json!({"name": name, "seconds": seconds}))
                .unwrap()
                .unwrap();
        }
        let mut plugin = Timers {
            clock,
            choices: None,
        };

        let miss = cancel(&mut plugin, 1, json!({"name": "картошка", "seconds": 300}));
        assert_eq!(miss["error"], "timer_not_active");
        assert_eq!(miss["active"].as_array().unwrap().len(), 1);
        assert_eq!(miss["active"][0]["name"], "для картошки");
        assert_eq!(plugin.clock.matches(&Filter::default()).len(), 2);

        let result = cancel(
            &mut plugin,
            1,
            json!({"name": "для картошки", "seconds": 300}),
        );
        assert_eq!(result["cancelled"]["name"], "для картошки");
        assert_eq!(
            plugin.clock.matches(&Filter::default())[0].voice(now())["name"],
            "для яиц"
        );
    }

    #[test]
    fn different_saved_names_remain_distinct() {
        let dir = tempfile::tempdir().unwrap();
        let clock = Arc::new(Clock::load(dir.path().join("timers.json")).unwrap());
        for name in ["картошки", "для картошки"] {
            clock
                .create(&json!({"name": name, "seconds": 300}))
                .unwrap()
                .unwrap();
        }
        let mut plugin = Timers {
            clock,
            choices: None,
        };

        let result = cancel(&mut plugin, 1, json!({"name": "Для  картошки"}));
        assert_eq!(result["cancelled"]["name"], "для картошки");
        assert_eq!(
            plugin.clock.matches(&Filter::default())[0].voice(now())["name"],
            "картошки"
        );
    }
}
