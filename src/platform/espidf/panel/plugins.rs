use std::io::{self, Read};

use anyhow::{Result, ensure};
use esp_idf_svc::{http::Method, io::Write};
use goodwin_plugin::HttpRequest;
use serde_json::{Value, json};

use crate::plugins::{Failure, connections, settings::BODY_LIMIT as SETTINGS_BODY_LIMIT};

use super::{CONTENT_SECURITY_POLICY, EspHttpConnection, Request, problem, send, send_json};
use crate::platform::espidf::plugins::{ARCHIVE_LIMIT, Plugins};

type PanelRequest<'a, 'b> = Request<&'a mut EspHttpConnection<'b>>;

pub(super) fn handle(mut request: PanelRequest<'_, '_>, plugins: &Plugins) -> Result<()> {
    let method = request.method();
    let uri = request.uri().to_owned();
    let (path, query) = uri
        .split_once('?')
        .map_or((uri.as_str(), None), |(path, query)| (path, Some(query)));
    let result: Result<(), Failure> = match (method, path) {
        (Method::Get, "/api/plugins") => return send_json(request, &plugins.catalog()),
        (Method::Post, "/api/plugin-packages") => {
            let Ok(body) = Body::new(&mut request, ARCHIVE_LIMIT) else {
                return problem(request, 413, "plugin_archive_too_large");
            };
            plugins.install(body)
        }
        (Method::Delete, path) if path.starts_with("/api/plugin-packages/") => {
            plugins.remove(&path["/api/plugin-packages/".len()..])
        }
        (Method::Get, path) if path.starts_with("/api/plugin-models/") => {
            let Some((provider, capability)) = path["/api/plugin-models/".len()..].split_once('/')
            else {
                return failure(request, Failure::NotFound);
            };
            let Ok(capability) = serde_json::from_value(json!(capability)) else {
                return failure(request, Failure::NotFound);
            };
            let provider = provider.replace("%3A", ":").replace("%3a", ":");
            return match plugins.models(&provider, capability) {
                Ok(models) => send_json(request, &models),
                Err(error) => failure(request, error),
            };
        }
        (Method::Get | Method::Put | Method::Delete, path)
            if path.starts_with("/api/plugin-connections/") =>
        {
            let suffix = &path["/api/plugin-connections/".len()..];
            return match method {
                Method::Get => match plugins.connections(suffix) {
                    Ok(connections) => send_json(request, &connections),
                    Err(error) => failure(request, error),
                },
                Method::Put => {
                    let Ok(update) = super::read_json::<connections::Update>(&mut request) else {
                        return failure(request, Failure::InvalidSettings);
                    };
                    match plugins.save_connection(suffix, update) {
                        Ok((id, connections)) => {
                            send_json(request, &json!({"id": id, "connections": connections}))
                        }
                        Err(error) => failure(request, error),
                    }
                }
                Method::Delete => {
                    let Some((plugin, connection)) = suffix.split_once('/') else {
                        return failure(request, Failure::NotFound);
                    };
                    match plugins.remove_connection(plugin, connection) {
                        Ok(connections) => send_json(request, &connections),
                        Err(error) => failure(request, error),
                    }
                }
                _ => unreachable!(),
            };
        }
        (Method::Get | Method::Patch, path) if path.starts_with("/api/plugin-settings/") => {
            let id = &path["/api/plugin-settings/".len()..];
            let patch = if method == Method::Patch {
                let Ok(data) = read_body(&mut request, SETTINGS_BODY_LIMIT)
                    .and_then(|bytes| Ok(serde_json::from_slice(&bytes)?))
                else {
                    return failure(request, Failure::InvalidSettings);
                };
                Some(data)
            } else {
                None
            };
            return match plugins.settings(id, patch) {
                Ok(settings) => send_json(request, &settings),
                Err(error) => failure(request, error),
            };
        }
        (Method::Get, path) if path.starts_with("/plugin-assets/") => {
            let Some((id, revision, path)) = asset_path(&path["/plugin-assets/".len()..]) else {
                return failure(request, Failure::NotFound);
            };
            let Some(mut file) = plugins.asset(id, revision, path) else {
                return failure(request, Failure::NotFound);
            };
            let Some(mime) = crate::plugins::package::asset_mime(path) else {
                return failure(request, Failure::NotFound);
            };
            let mut response = request.into_response(
                200,
                Some("OK"),
                &[
                    ("Content-Type", mime),
                    ("Cache-Control", "no-store"),
                    ("Content-Security-Policy", CONTENT_SECURITY_POLICY),
                    ("X-Content-Type-Options", "nosniff"),
                ],
            )?;
            let mut buffer = [0; 4096];
            loop {
                let read = file.read(&mut buffer)?;
                if read == 0 {
                    return Ok(());
                }
                response.write_all(&buffer[..read])?;
            }
        }
        (method, path) if path.starts_with("/api/plugins/") => {
            let Some((id, path)) = path["/api/plugins/".len()..].split_once('/') else {
                return failure(request, Failure::NotFound);
            };
            let method = match method {
                Method::Get => "GET",
                Method::Post => "POST",
                Method::Put => "PUT",
                Method::Patch => "PATCH",
                Method::Delete => "DELETE",
                _ => return failure(request, Failure::NotFound),
            };
            let Ok(bytes) = read_body(&mut request, 16 * 1024) else {
                return problem(request, 413, "invalid_plugin_request");
            };
            let body = if bytes.is_empty() {
                Value::Null
            } else {
                let Ok(value) = serde_json::from_slice(&bytes) else {
                    return problem(request, 400, "invalid_plugin_request");
                };
                value
            };
            let response = plugins.http(
                id,
                HttpRequest {
                    method: method.into(),
                    path: format!("/{path}"),
                    query: query.map(str::to_owned),
                    body,
                },
            );
            return match response {
                Ok(response)
                    if (200..300).contains(&response.status)
                        || (400..600).contains(&response.status) =>
                {
                    if let Some(body) = response.body.filter(|_| response.status != 204) {
                        send(
                            request,
                            response.status,
                            "application/json",
                            &serde_json::to_vec(&body)?,
                            &[],
                        )
                    } else {
                        send(request, response.status, "text/plain", &[], &[])
                    }
                }
                Ok(_) => failure(request, Failure::Unavailable),
                Err(error) => failure(request, error),
            };
        }
        _ => Err(Failure::NotFound),
    };
    match result {
        Ok(()) => send(request, 204, "text/plain", &[], &[]),
        Err(error) => failure(request, error),
    }
}

fn failure(request: PanelRequest<'_, '_>, error: Failure) -> Result<()> {
    problem(request, error.status(), error.code())
}

fn asset_path(path: &str) -> Option<(&str, &str, &str)> {
    let mut parts = path.splitn(3, '/');
    Some((parts.next()?, parts.next()?, parts.next()?))
}

fn read_body(request: &mut PanelRequest<'_, '_>, limit: usize) -> Result<Vec<u8>> {
    let mut body = Body::new(request, limit)?;
    let mut bytes = Vec::new();
    body.read_to_end(&mut bytes)?;
    Ok(bytes)
}

struct Body<'r, 'a, 'b> {
    request: &'r mut PanelRequest<'a, 'b>,
    remaining: usize,
}

impl<'r, 'a, 'b> Body<'r, 'a, 'b> {
    fn new(request: &'r mut PanelRequest<'a, 'b>, limit: usize) -> Result<Self> {
        let remaining = request
            .header("Content-Length")
            .map(str::parse)
            .transpose()?
            .unwrap_or(0);
        ensure!(remaining <= limit, "Request is too large");
        Ok(Self { request, remaining })
    }
}

impl Read for Body<'_, '_, '_> {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        let length = bytes.len().min(self.remaining);
        if length == 0 {
            return Ok(0);
        }
        let read = self
            .request
            .read(&mut bytes[..length])
            .map_err(io::Error::other)?;
        if read == 0 {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        self.remaining -= read;
        Ok(read)
    }
}
