use axum::body::Body;
use axum::extract::Json as AxumJson;
use axum::http::{Request, StatusCode};
use axum::routing::post;
use axum::{Json, Router as AxumRouter};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use srvcs_percentageof::{api::Deps, health, router, telemetry};
use tower::ServiceExt;

const DEAD_URL: &str = "http://127.0.0.1:1";

/// Spawn a *computing* mock `srvcs-floatdivide`: reads `{"a": x, "b": y}` and
/// returns `{"result": x / y}` as an `f64`. The orchestration is genuinely
/// driven by this answer rather than a canned value.
async fn spawn_floatdivide() -> String {
    let app = AxumRouter::new().route(
        "/",
        post(|AxumJson(body): AxumJson<Value>| async move {
            let a = body.get("a").and_then(Value::as_f64).unwrap_or(0.0);
            let b = body.get("b").and_then(Value::as_f64).unwrap_or(1.0);
            Json(json!({ "result": a / b }))
        }),
    );
    serve(app).await
}

/// Spawn a *computing* mock `srvcs-floatmultiply`: reads `{"a": x, "b": y}` and
/// returns `{"result": x * y}` as an `f64` — the real product.
async fn spawn_floatmultiply() -> String {
    let app = AxumRouter::new().route(
        "/",
        post(|AxumJson(body): AxumJson<Value>| async move {
            let a = body.get("a").and_then(Value::as_f64).unwrap_or(0.0);
            let b = body.get("b").and_then(Value::as_f64).unwrap_or(0.0);
            Json(json!({ "result": a * b }))
        }),
    );
    serve(app).await
}

/// Spawn a mock returning a fixed status + body (used for error-path tests).
async fn spawn_fixed(status: StatusCode, body: Value) -> String {
    let app = AxumRouter::new().route(
        "/",
        post(move || {
            let body = body.clone();
            async move { (status, Json(body)) }
        }),
    );
    serve(app).await
}

async fn serve(app: AxumRouter) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}

fn approx(got: f64, expected: f64) {
    assert!(
        (got - expected).abs() < 1e-9,
        "expected {expected}, got {got}"
    );
}

fn app(floatdivide_url: &str, floatmultiply_url: &str) -> axum::Router {
    router(
        telemetry::metrics_handle_for_tests(),
        Deps {
            floatdivide_url: floatdivide_url.to_string(),
            floatmultiply_url: floatmultiply_url.to_string(),
        },
    )
}

async fn percentageof(
    floatdivide_url: &str,
    floatmultiply_url: &str,
    percent: f64,
    whole: f64,
) -> (StatusCode, Value) {
    let res = app(floatdivide_url, floatmultiply_url)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({ "percent": percent, "whole": whole }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = res.status();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

async fn status_of(uri: &str) -> StatusCode {
    app(DEAD_URL, DEAD_URL)
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap()
        .status()
}

// --- Standard endpoints. ---

#[tokio::test]
async fn healthz_ok() {
    assert_eq!(status_of("/healthz").await, StatusCode::OK);
}

#[tokio::test]
async fn readyz_reflects_state() {
    health::set_ready(true);
    assert_eq!(status_of("/readyz").await, StatusCode::OK);
}

#[tokio::test]
async fn metrics_ok() {
    assert_eq!(status_of("/metrics").await, StatusCode::OK);
}

#[tokio::test]
async fn openapi_ok() {
    assert_eq!(status_of("/openapi.json").await, StatusCode::OK);
}

#[tokio::test]
async fn generates_request_id_when_absent() {
    let res = app(DEAD_URL, DEAD_URL)
        .oneshot(
            Request::builder()
                .uri("/healthz")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(
        res.headers().contains_key("x-request-id"),
        "response must carry a generated x-request-id"
    );
}

#[tokio::test]
async fn index_reports_identity() {
    let res = app(DEAD_URL, DEAD_URL)
        .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let body: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body["service"], "srvcs-percentageof");
    assert_eq!(body["concern"], "arithmetic: percent% of whole");
    assert_eq!(
        body["depends_on"],
        json!(["srvcs-floatdivide", "srvcs-floatmultiply"])
    );
}

// --- Correctness cases, against the computing mocks. ---

#[tokio::test]
async fn percentageof_20_of_50_is_10() {
    let (d, m) = (spawn_floatdivide().await, spawn_floatmultiply().await);
    let (status, body) = percentageof(&d, &m, 20.0, 50.0).await;
    assert_eq!(status, StatusCode::OK);
    approx(body["percent"].as_f64().unwrap(), 20.0);
    approx(body["whole"].as_f64().unwrap(), 50.0);
    // floatdivide(20,100)=0.2; floatmultiply(0.2,50)=10.0
    approx(body["result"].as_f64().unwrap(), 10.0);
}

#[tokio::test]
async fn percentageof_50_of_200_is_100() {
    let (d, m) = (spawn_floatdivide().await, spawn_floatmultiply().await);
    let (status, body) = percentageof(&d, &m, 50.0, 200.0).await;
    assert_eq!(status, StatusCode::OK);
    // 0.5 * 200 = 100
    approx(body["result"].as_f64().unwrap(), 100.0);
}

#[tokio::test]
async fn percentageof_fractional_result() {
    let (d, m) = (spawn_floatdivide().await, spawn_floatmultiply().await);
    let (status, body) = percentageof(&d, &m, 12.5, 80.0).await;
    assert_eq!(status, StatusCode::OK);
    // 0.125 * 80 = 10.0
    approx(body["result"].as_f64().unwrap(), 10.0);
}

#[tokio::test]
async fn percentageof_fractional_percent_and_whole() {
    let (d, m) = (spawn_floatdivide().await, spawn_floatmultiply().await);
    let (status, body) = percentageof(&d, &m, 33.0, 12.0).await;
    assert_eq!(status, StatusCode::OK);
    // 0.33 * 12 = 3.96
    approx(body["result"].as_f64().unwrap(), 3.96);
}

#[tokio::test]
async fn percentageof_zero_percent_is_zero() {
    let (d, m) = (spawn_floatdivide().await, spawn_floatmultiply().await);
    let (status, body) = percentageof(&d, &m, 0.0, 999.0).await;
    assert_eq!(status, StatusCode::OK);
    approx(body["result"].as_f64().unwrap(), 0.0);
}

#[tokio::test]
async fn percentageof_negative_percent() {
    let (d, m) = (spawn_floatdivide().await, spawn_floatmultiply().await);
    let (status, body) = percentageof(&d, &m, -10.0, 50.0).await;
    assert_eq!(status, StatusCode::OK);
    // -0.1 * 50 = -5.0
    approx(body["result"].as_f64().unwrap(), -5.0);
}

// --- Error / degraded paths. ---

#[tokio::test]
async fn degrades_when_floatdivide_unreachable() {
    let m = spawn_floatmultiply().await;
    let (status, body) = percentageof(DEAD_URL, &m, 20.0, 50.0).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body["dependency"], "srvcs-floatdivide");
}

#[tokio::test]
async fn degrades_when_floatmultiply_unreachable() {
    // floatdivide reachable, so the pipeline reaches the multiply call.
    let d = spawn_floatdivide().await;
    let (status, body) = percentageof(&d, DEAD_URL, 20.0, 50.0).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body["dependency"], "srvcs-floatmultiply");
}

#[tokio::test]
async fn forwards_422_from_floatdivide() {
    let m = spawn_floatmultiply().await;
    let d = spawn_fixed(
        StatusCode::UNPROCESSABLE_ENTITY,
        json!({ "error": "value is not a number" }),
    )
    .await;
    let (status, _) = percentageof(&d, &m, 20.0, 50.0).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn forwards_422_from_floatmultiply() {
    let d = spawn_floatdivide().await;
    let m = spawn_fixed(
        StatusCode::UNPROCESSABLE_ENTITY,
        json!({ "error": "value is not a number" }),
    )
    .await;
    let (status, _) = percentageof(&d, &m, 20.0, 50.0).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn malformed_floatdivide_result_is_500() {
    // floatdivide answers 200 but with no numeric result -> contract violation.
    let m = spawn_floatmultiply().await;
    let d = spawn_fixed(StatusCode::OK, json!({ "result": "not-a-number" })).await;
    let (status, body) = percentageof(&d, &m, 20.0, 50.0).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(body["dependency"], "srvcs-floatdivide");
}

#[tokio::test]
async fn malformed_floatmultiply_result_is_500() {
    let d = spawn_floatdivide().await;
    let m = spawn_fixed(StatusCode::OK, json!({ "result": "not-a-number" })).await;
    let (status, body) = percentageof(&d, &m, 20.0, 50.0).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(body["dependency"], "srvcs-floatmultiply");
}
