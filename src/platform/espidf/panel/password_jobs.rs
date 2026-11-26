use std::sync::Arc;

use super::{
    Auth, Panel,
    jobs::{Outcome, Problem},
};
use crate::password;

pub(super) enum Operation {
    Setup(String),
    Login(String),
}

enum Work {
    Setup(String),
    Verify { password: String, hash: String },
}

pub(super) fn submit(panel: &Arc<Panel>, operation: Operation) -> Outcome<String> {
    panel
        .password_jobs
        .submit("goodwin-password", 12 * 1024, (429, "login_retry"), || {
            let work = match operation {
                Operation::Setup(password) => Work::Setup(password),
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
        })
}

fn process(panel: &Panel, work: Work) -> Outcome<String> {
    match work {
        Work::Setup(password) => {
            let hash = password::hash(&password).map_err(|_| (500, "password_save_failed"))?;
            let mut credentials = panel.credentials.lock().expect("panel credentials");
            if !credentials.0.access(None).setup_required {
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
