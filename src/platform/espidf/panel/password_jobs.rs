use std::sync::Arc;

use esp_idf_svc::hal::{cpu::Core, task::thread::ThreadSpawnConfiguration};

use super::{
    Auth, Panel,
    jobs::{Outcome, Problem},
};
use crate::{panel_core::AccessState, password};

pub(super) enum Operation {
    Setup(String, AccessState),
    Login(String),
}

enum Work {
    Setup(String, AccessState),
    Verify { password: String, hash: String },
}

pub(super) fn submit(panel: &Arc<Panel>, operation: Operation) -> Outcome<String> {
    // Match the core-0 capture task's priority so it can drain I²S between slices.
    // The inference worker can use the other core while passwords are processed.
    let mut config = ThreadSpawnConfiguration::get().unwrap_or_default();
    let previous_core = config.pin_to_core.replace(Core::Core0);
    let previous_priority = core::mem::replace(&mut config.priority, 1);
    config.set().map_err(|error| {
        log::warn!("Configure panel password worker: {error}");
        (503, "busy")
    })?;
    let result =
        panel
            .password_jobs
            .submit("goodwin-password", 12 * 1024, (429, "login_retry"), || {
                let work = match operation {
                    Operation::Setup(password, previous) => Work::Setup(password, previous),
                    Operation::Login(password) => Work::Verify {
                        password,
                        hash: panel
                            .credentials
                            .lock()
                            .expect("panel credentials")
                            .0
                            .reserve_login()
                            .map_err(login_problem)?,
                    },
                };
                let panel = Arc::clone(panel);
                Ok(move || process(&panel, work))
            });
    config.pin_to_core = previous_core;
    config.priority = previous_priority;
    if let Err(error) = config.set() {
        log::warn!("Restore panel worker configuration: {error}");
    }
    result
}

fn process(panel: &Panel, work: Work) -> Outcome<String> {
    match work {
        Work::Setup(password, previous) => {
            let hash = password::hash(&password).map_err(|_| (500, "password_save_failed"))?;
            let mut credentials = panel.credentials.lock().expect("panel credentials");
            if credentials.0.access(None) != previous {
                return Err((409, "setup_complete"));
            }
            credentials
                .1
                .save_password(&hash)
                .map_err(|_| (500, "password_save_failed"))?;
            credentials.0.set_password(hash.clone());
            Ok(hash)
        }
        Work::Verify { password, hash } => {
            Auth::verify_login(&hash, &password).map_err(login_problem)?;
            Ok(hash)
        }
    }
}

fn login_problem(code: &'static str) -> Problem {
    let status = match code {
        "setup_required" | "password_reprovision_required" => 409,
        "login_retry" => 429,
        "wrong_password" => 401,
        _ => 500,
    };
    (status, code)
}
