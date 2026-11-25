use axum::{
    body::Bytes,
    extract::{FromRequest, Request, rejection::BytesRejection},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use serde::{Serialize, de::DeserializeOwned};

pub(super) struct Json<T>(pub T);
pub(super) struct JsonDocument;

impl<S: Send + Sync, T: DeserializeOwned> FromRequest<S> for Json<T> {
    type Rejection = Response;

    async fn from_request(request: Request, state: &S) -> Result<Self, Self::Rejection> {
        let bytes = json_bytes(request, state)
            .await
            .map_err(IntoResponse::into_response)?;
        match goodwin_json::from_slice(&bytes) {
            Ok(value) => Ok(Self(value)),
            Err(error) if error.requires_untyped_values() => {
                serde_json::from_slice(&bytes).map(Self).map_err(|error| {
                    (StatusCode::UNPROCESSABLE_ENTITY, error.to_string()).into_response()
                })
            }
            Err(error) => Err(rejection(error)),
        }
    }
}

impl<S: Send + Sync> FromRequest<S> for JsonDocument {
    type Rejection = Response;

    async fn from_request(request: Request, state: &S) -> Result<Self, Self::Rejection> {
        let bytes = json_bytes(request, state)
            .await
            .map_err(IntoResponse::into_response)?;
        goodwin_json::validate(&bytes).map_err(rejection)?;
        Ok(Self)
    }
}

enum JsonBytesRejection {
    ContentType,
    Body(BytesRejection),
}

impl IntoResponse for JsonBytesRejection {
    fn into_response(self) -> Response {
        match self {
            Self::ContentType => (
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                "Expected request with `Content-Type: application/json`",
            )
                .into_response(),
            Self::Body(error) => error.into_response(),
        }
    }
}

async fn json_bytes<S: Send + Sync>(
    request: Request,
    state: &S,
) -> Result<Bytes, JsonBytesRejection> {
    let is_json = request
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<mime::Mime>().ok())
        .is_some_and(|value| {
            value.type_() == mime::APPLICATION
                && (value.subtype() == mime::JSON || value.suffix() == Some(mime::JSON))
        });
    if !is_json {
        return Err(JsonBytesRejection::ContentType);
    }
    Bytes::from_request(request, state)
        .await
        .map_err(JsonBytesRejection::Body)
}

fn rejection(error: goodwin_json::DecodeError) -> Response {
    let status = match error {
        goodwin_json::DecodeError::Syntax => StatusCode::BAD_REQUEST,
        goodwin_json::DecodeError::Data(_) => StatusCode::UNPROCESSABLE_ENTITY,
    };
    (status, error.to_string()).into_response()
}

impl<T: Serialize> IntoResponse for Json<T> {
    fn into_response(self) -> Response {
        match goodwin_json::to_vec(&self.0) {
            Ok(bytes) => ([(header::CONTENT_TYPE, "application/json")], bytes).into_response(),
            Err(error) => (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response(),
        }
    }
}
