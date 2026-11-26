use std::sync::{Arc, atomic::Ordering, mpsc};

use serde::{Deserialize, Serialize};

use super::{
    Command, EspHttpConnection, Method, Panel, Request, Result, State, Ticket, jobs::Outcome,
    problem, read_json, send_json, send_pending, send_ticket,
};
use crate::{
    platform::espidf::wake_models,
    wake_catalog::{Selection, valid_model_id},
};

#[derive(Deserialize)]
struct Install {
    id: String,
    tag: String,
}

#[derive(Serialize)]
#[serde(untagged)]
pub(super) enum Completion {
    Catalog { tag: String, models: Vec<Selection> },
    Installed(Selection),
}

pub(super) fn handle(
    mut request: Request<&mut EspHttpConnection<'_>>,
    panel: &Arc<Panel>,
) -> Result<()> {
    if !panel.wake_catalog {
        return problem(request, 404, "not_found");
    }
    if !panel.wifi_connected() {
        return problem(request, 409, "wifi_unavailable");
    }
    let install = if request.method() == Method::Put {
        match read_json::<Install>(&mut request) {
            Ok(install) if valid_model_id(&install.id) && install.tag.len() <= 33 => Some(install),
            _ => return problem(request, 400, "wake_model_invalid"),
        }
    } else {
        None
    };
    let ticket = panel.wake_jobs.submit(
        "goodwin-wake-model",
        20 * 1024,
        (409, "wake_model_busy"),
        || {
            let panel = Arc::clone(panel);
            Ok(move || process(&panel, install))
        },
    );
    send_ticket(request, ticket)
}

pub(super) fn result(
    mut request: Request<&mut EspHttpConnection<'_>>,
    panel: &Panel,
) -> Result<()> {
    let Ok(ticket) = read_json::<Ticket>(&mut request) else {
        return problem(request, 400, "invalid_request");
    };
    match panel.wake_jobs.take(&ticket.operation, "wake_model_failed") {
        Ok(None) => send_pending(request),
        Ok(Some(completion)) => send_json(request, &completion),
        Err((status, code)) => problem(request, status, code),
    }
}

fn process(panel: &Panel, install: Option<Install>) -> Outcome<Completion> {
    let catalog = wake_models::catalog().map_err(|error| {
        log::warn!("Fetch wake model catalog: {error:#}");
        (502, "wake_catalog_unavailable")
    })?;
    let Some(install) = install else {
        return Ok(Completion::Catalog {
            tag: catalog.tag,
            models: catalog
                .models
                .into_iter()
                .map(|model| model.selection)
                .collect(),
        });
    };
    if catalog.tag != install.tag {
        return Err((409, "wake_catalog_changed"));
    }
    let candidate = match wake_models::download(&catalog, &install.id) {
        Ok(Some(candidate)) => candidate,
        Ok(None) => return Err((404, "wake_model_invalid")),
        Err(error) => {
            log::warn!("Download wake model: {error:#}");
            return Err((502, "wake_model_failed"));
        }
    };
    let state = State::lock(&panel.state);
    if state.busy() || panel.firmware_busy() || panel.wake_model_busy.swap(true, Ordering::AcqRel) {
        return Err((409, "wake_model_busy"));
    }
    let (reply, receive) = mpsc::channel();
    let sent = panel
        .command
        .try_send(Command::UseWakeModel(candidate, reply));
    drop(state);
    if sent.is_err() {
        panel.finish_wake_model_change();
        return Err((409, "wake_model_busy"));
    }
    match receive.recv() {
        Ok(Ok(selection)) => Ok(Completion::Installed(selection)),
        Ok(Err(error)) => {
            log::warn!("Activate wake model: {error:#}");
            Err((500, "wake_model_failed"))
        }
        Err(error) => {
            log::warn!("Await wake model activation: {error}");
            Err((503, "wake_model_failed"))
        }
    }
}
