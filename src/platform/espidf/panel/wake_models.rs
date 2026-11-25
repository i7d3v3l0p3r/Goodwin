use std::{
    sync::{atomic::Ordering, mpsc},
    time::Duration,
};

use super::{
    Command, EspHttpConnection, Method, Panel, Request, Result, State, json, problem, read_json,
    send_json,
};
use crate::{platform::espidf::wake_models, wake_catalog::valid_model_id};

#[derive(serde::Deserialize)]
struct Install {
    id: String,
    tag: String,
}

pub(super) fn handle(
    mut request: Request<&mut EspHttpConnection<'_>>,
    panel: &Panel,
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
    let catalog = match wake_models::catalog() {
        Ok(catalog) => catalog,
        Err(error) => {
            log::warn!("Fetch wake model catalog: {error:#}");
            return problem(request, 502, "wake_catalog_unavailable");
        }
    };
    let Some(install) = install else {
        let models: Vec<_> = catalog
            .models
            .into_iter()
            .map(|model| model.selection)
            .collect();
        return send_json(request, &json!({"tag": catalog.tag, "models": models}));
    };
    if catalog.tag != install.tag {
        return problem(request, 409, "wake_catalog_changed");
    }
    let candidate = match wake_models::download(&catalog, &install.id) {
        Ok(Some(candidate)) => candidate,
        Ok(None) => return problem(request, 404, "wake_model_invalid"),
        Err(error) => {
            log::warn!("Download wake model: {error:#}");
            return problem(request, 502, "wake_model_failed");
        }
    };
    let state = State::lock(&panel.state);
    if state.busy() || panel.firmware_busy() || panel.wake_model_busy.swap(true, Ordering::AcqRel) {
        return problem(request, 409, "wake_model_busy");
    }
    let (reply, receive) = mpsc::channel();
    let sent = panel
        .command
        .try_send(Command::UseWakeModel(candidate, reply));
    drop(state);
    if sent.is_err() {
        panel.finish_wake_model_change();
        return problem(request, 409, "wake_model_busy");
    }
    match receive.recv_timeout(Duration::from_secs(25)) {
        Ok(Ok(selection)) => send_json(request, &selection),
        Ok(Err(error)) => {
            log::warn!("Activate wake model: {error:#}");
            problem(request, 500, "wake_model_failed")
        }
        Err(error) => {
            log::warn!("Await wake model activation: {error}");
            problem(request, 503, "wake_model_failed")
        }
    }
}
