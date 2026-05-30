use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use utoipa::{OpenApi, ToSchema};

use crate::client::{self, DepError};

pub const SERVICE: &str = "srvcs-percentageof";
pub const CONCERN: &str = "arithmetic: percent% of whole";
pub const DEPENDS_ON: &[&str] = &["srvcs-floatdivide", "srvcs-floatmultiply"];

/// Dependency endpoints, injected as router state so tests can point them at
/// mock services.
#[derive(Clone)]
pub struct Deps {
    pub floatdivide_url: String,
    pub floatmultiply_url: String,
}

#[derive(Serialize, ToSchema)]
pub struct Info {
    pub service: &'static str,
    pub concern: &'static str,
    pub depends_on: Vec<&'static str>,
}

/// `GET /` — service identity (srvcs service standard).
#[utoipa::path(get, path = "/", responses((status = 200, body = Info)))]
pub async fn index() -> Json<Info> {
    Json(Info {
        service: SERVICE,
        concern: CONCERN,
        depends_on: DEPENDS_ON.to_vec(),
    })
}

#[derive(Deserialize, ToSchema)]
pub struct EvalRequest {
    pub percent: f64,
    pub whole: f64,
}

#[derive(Serialize, ToSchema)]
pub struct PercentageOfResponse {
    pub percent: f64,
    pub whole: f64,
    /// `percent% of whole`, as a floating-point number that may be fractional.
    pub result: f64,
}

fn ok(percent: f64, whole: f64, result: f64) -> Response {
    (
        StatusCode::OK,
        Json(json!({ "percent": percent, "whole": whole, "result": result })),
    )
        .into_response()
}

fn degraded(dependency: &str) -> Response {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(json!({ "error": "dependency unavailable", "dependency": dependency })),
    )
        .into_response()
}

fn forward(status: u16, body: Value) -> Response {
    let code = StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_GATEWAY);
    (code, Json(body)).into_response()
}

/// A reachable dependency answered `200` but its body lacked a numeric
/// `result`. That is a contract violation we cannot recover from, so surface a
/// `500` rather than guessing.
fn malformed(dependency: &str) -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(
            json!({ "error": "dependency returned a malformed result", "dependency": dependency }),
        ),
    )
        .into_response()
}

/// Call one dependency at `url` with `body`, mapping its outcome to either the
/// parsed response body (on `200`) or an early-return `Response` the caller
/// should surface verbatim:
///
/// - unreachable / non-`200`/`422` -> `503` degraded
/// - `422` -> forwarded `422` (the dependency rejected the input)
async fn ask(url: &str, body: &Value, dependency: &str) -> Result<Value, Response> {
    match client::call(url, body).await {
        Err(DepError::Unreachable) => Err(degraded(dependency)),
        Ok((200, body)) => Ok(body),
        Ok((422, body)) => Err(forward(422, body)),
        Ok(_) => Err(degraded(dependency)),
    }
}

/// `POST /` — compute `percent% of whole` by composing two float primitives.
///
/// This service owns the *control flow* but delegates every arithmetic step to
/// its dependencies, exactly as specified:
///
/// 1. ask `srvcs-floatdivide` for `frac = percent / 100`;
/// 2. ask `srvcs-floatmultiply` for `result = frac * whole`.
///
/// The constant `100` is a trivial local literal; the headline operations both
/// go through the dependency services. If a dependency is unreachable it reports
/// itself degraded (`503`); if a dependency rejects the input it forwards the
/// `422`.
#[utoipa::path(
    post,
    path = "/",
    request_body = EvalRequest,
    responses(
        (status = 200, body = PercentageOfResponse),
        (status = 422, description = "a dependency rejected the input (forwarded)"),
        (status = 500, description = "a dependency returned a malformed result"),
        (status = 503, description = "a dependency is unavailable")
    )
)]
pub async fn evaluate(State(deps): State<Deps>, Json(req): Json<EvalRequest>) -> Response {
    let (percent, whole) = (req.percent, req.whole);

    // 1. frac = percent / 100
    let divide_body = match ask(
        &deps.floatdivide_url,
        &json!({ "a": percent, "b": 100 }),
        "srvcs-floatdivide",
    )
    .await
    {
        Ok(body) => body,
        Err(resp) => return resp,
    };
    let frac = match divide_body.get("result").and_then(Value::as_f64) {
        Some(frac) => frac,
        None => return malformed("srvcs-floatdivide"),
    };

    // 2. result = frac * whole
    let multiply_body = match ask(
        &deps.floatmultiply_url,
        &json!({ "a": frac, "b": whole }),
        "srvcs-floatmultiply",
    )
    .await
    {
        Ok(body) => body,
        Err(resp) => return resp,
    };
    let result = match multiply_body.get("result").and_then(Value::as_f64) {
        Some(r) => r,
        None => return malformed("srvcs-floatmultiply"),
    };

    ok(percent, whole, result)
}

#[derive(OpenApi)]
#[openapi(
    paths(index, evaluate),
    components(schemas(Info, EvalRequest, PercentageOfResponse))
)]
pub struct ApiDoc;

/// Serve OpenAPI document
pub async fn openapi_json() -> Json<utoipa::openapi::OpenApi> {
    Json(ApiDoc::openapi())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn openapi_documents_routes() {
        let doc = ApiDoc::openapi();
        let root = doc.paths.paths.get("/").expect("path / present");
        assert!(root.get.is_some());
        assert!(root.post.is_some());
    }

    #[tokio::test]
    async fn index_reports_all_dependencies() {
        let Json(info) = index().await;
        assert_eq!(info.service, "srvcs-percentageof");
        assert_eq!(info.concern, "arithmetic: percent% of whole");
        assert_eq!(
            info.depends_on,
            vec!["srvcs-floatdivide", "srvcs-floatmultiply"]
        );
    }
}
