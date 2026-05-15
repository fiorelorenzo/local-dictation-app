use axum::{
    body::Body,
    http::{header::ACCEPT, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wire {
    Json,
    MsgPack,
}

impl Wire {
    pub fn from_accept(headers: &HeaderMap) -> Self {
        headers
            .get(ACCEPT)
            .and_then(|v| v.to_str().ok())
            .map(|s| {
                if s.contains("application/msgpack") {
                    Wire::MsgPack
                } else {
                    Wire::Json
                }
            })
            .unwrap_or(Wire::Json)
    }
}

pub struct WireResponse<T: Serialize> {
    pub wire: Wire,
    pub status: StatusCode,
    pub body: T,
}

impl<T: Serialize> WireResponse<T> {
    pub fn ok(wire: Wire, body: T) -> Self {
        Self { wire, status: StatusCode::OK, body }
    }
    pub fn with_status(wire: Wire, status: StatusCode, body: T) -> Self {
        Self { wire, status, body }
    }
}

impl<T: Serialize> IntoResponse for WireResponse<T> {
    fn into_response(self) -> Response {
        match self.wire {
            Wire::Json => {
                let mut resp = Json(self.body).into_response();
                *resp.status_mut() = self.status;
                resp
            }
            Wire::MsgPack => match rmp_serde::to_vec_named(&self.body) {
                Ok(bytes) => {
                    let mut resp = Response::new(Body::from(bytes));
                    *resp.status_mut() = self.status;
                    resp.headers_mut().insert(
                        axum::http::header::CONTENT_TYPE,
                        HeaderValue::from_static("application/msgpack"),
                    );
                    resp
                }
                Err(e) => {
                    let body = serde_json::json!({
                        "error": "internal",
                        "reason": format!("msgpack encode failed: {e}")
                    });
                    let mut resp = Json(body).into_response();
                    *resp.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
                    resp
                }
            },
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ErrorBody {
    pub error: &'static str,
    pub reason: String,
}

pub fn error_response(wire: Wire, status: StatusCode, error: &'static str, reason: impl Into<String>) -> WireResponse<ErrorBody> {
    WireResponse::with_status(wire, status, ErrorBody { error, reason: reason.into() })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Serialize, Deserialize, Debug, PartialEq)]
    struct Sample {
        s: String,
        n: i32,
    }

    #[test]
    fn accept_msgpack_is_detected() {
        let mut h = HeaderMap::new();
        h.insert(ACCEPT, HeaderValue::from_static("application/msgpack"));
        assert_eq!(Wire::from_accept(&h), Wire::MsgPack);
    }

    #[test]
    fn accept_anything_else_defaults_to_json() {
        let mut h = HeaderMap::new();
        h.insert(ACCEPT, HeaderValue::from_static("text/html"));
        assert_eq!(Wire::from_accept(&h), Wire::Json);
        let h2 = HeaderMap::new();
        assert_eq!(Wire::from_accept(&h2), Wire::Json);
    }

    #[test]
    fn msgpack_roundtrip_named_fields() {
        let s = Sample { s: "hello".into(), n: 42 };
        let bytes = rmp_serde::to_vec_named(&s).unwrap();
        let decoded: Sample = rmp_serde::from_slice(&bytes).unwrap();
        assert_eq!(decoded, s);
    }
}
