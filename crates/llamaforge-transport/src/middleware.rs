use axum::{
    extract::Request,
    middleware::Next,
    response::{IntoResponse, Response},
};
use llamaforge_core::config::ConfigReader;
use std::sync::Arc;
use tokio::sync::Semaphore;
use tower_http::cors::{AllowOrigin, CorsLayer};
use uuid::Uuid;

pub(crate) fn build_cors_layer(config_reader: Arc<dyn ConfigReader>) -> CorsLayer {
    CorsLayer::new()
        .allow_origin(AllowOrigin::predicate(move |origin: &axum::http::HeaderValue, _parts| {
            let config = config_reader.snapshot();
            let origins = &config.cors.allowed_origins;
            if origins.iter().any(|o| o == "*") {
                return true;
            }
            if let Ok(origin_str) = origin.to_str() {
                origins.iter().any(|o| o == origin_str)
            } else {
                false
            }
        }))
        .allow_methods([
            axum::http::Method::GET,
            axum::http::Method::POST,
            axum::http::Method::DELETE,
            axum::http::Method::OPTIONS,
        ])
        .allow_headers([axum::http::header::CONTENT_TYPE, axum::http::header::AUTHORIZATION])
        .max_age(std::time::Duration::from_secs(3600))
}

#[derive(Clone)]
pub struct LargeBodyConcurrencyLimit {
    semaphore: Arc<Semaphore>,
    threshold_bytes: u64,
}

impl LargeBodyConcurrencyLimit {
    pub fn new(max_concurrent: usize, threshold_mb: u64) -> Self {
        Self {
            semaphore: Arc::new(Semaphore::new(max_concurrent)),
            threshold_bytes: threshold_mb * 1024 * 1024,
        }
    }
}

pub(crate) async fn large_body_limit_middleware(
    state: axum::extract::State<LargeBodyConcurrencyLimit>,
    req: Request,
    next: Next,
) -> Result<Response, crate::error::ApiError> {
    let content_length = req
        .headers()
        .get(axum::http::header::CONTENT_LENGTH)
        .and_then(|val| val.to_str().ok())
        .and_then(|val| val.parse::<u64>().ok())
        .unwrap_or(0);

    if content_length > state.threshold_bytes {
        let permit = state.semaphore.clone().try_acquire_owned();
        match permit {
            Ok(p) => {
                let mut res = next.run(req).await;
                // Bind permit to response extensions so it's dropped when the response completes
                res.extensions_mut().insert(Arc::new(p));
                Ok(res)
            }
            Err(_) => Err(crate::error::ApiError::QueueFull { retry_after_secs: 30 }),
        }
    } else {
        Ok(next.run(req).await)
    }
}

pub(crate) async fn auth_middleware(
    state: axum::extract::State<Arc<dyn ConfigReader>>,
    req: Request,
    next: Next,
) -> Result<Response, crate::error::ApiError> {
    let config = state.snapshot();

    if let Some(expected_key) = &config.global.api_key {
        if expected_key.is_empty() {
            return Ok(next.run(req).await);
        }

        let auth_header =
            req.headers().get(axum::http::header::AUTHORIZATION).and_then(|val| val.to_str().ok());

        let is_valid = match auth_header {
            Some(header) if header.starts_with("Bearer ") => {
                let token = &header[7..];
                token == expected_key
            }
            _ => false,
        };

        if !is_valid {
            let ip = req
                .extensions()
                .get::<axum::extract::ConnectInfo<std::net::SocketAddr>>()
                .map(|c| c.0.to_string())
                .unwrap_or_else(|| "unknown".to_string());
            let masked_key = auth_header
                .and_then(|h| h.strip_prefix("Bearer "))
                .map(|k| format!("{}...", &k[..std::cmp::min(k.len(), 4)]))
                .unwrap_or_else(|| "none".to_string());

            tracing::warn!(
                ip = %ip,
                provided_key_prefix = %masked_key,
                "transport.auth_failed"
            );
            metrics::counter!("llamaforge_auth_failures_total").increment(1);
            return Err(crate::error::ApiError::AuthFailure);
        }
    }

    Ok(next.run(req).await)
}

pub(crate) async fn request_id_middleware(mut req: Request, next: Next) -> Response {
    let request_id = Uuid::now_v7();
    let request_id_str = request_id.to_string();

    req.extensions_mut().insert(llamaforge_core::types::RequestId::new(request_id));

    let method = req.method().to_string();
    let path = req.uri().path().to_string();
    let content_length = req
        .headers()
        .get(axum::http::header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("0")
        .to_string();

    tracing::debug!(
        method = %method,
        path = %path,
        request_id = %request_id_str,
        content_length = %content_length,
        "transport.request"
    );

    let mut response = next.run(req).await;

    if response.status().is_client_error() || response.status().is_server_error() {
        if response.status() == axum::http::StatusCode::PAYLOAD_TOO_LARGE {
            tracing::warn!(
                received_bytes = %content_length,
                limit = 52428800,
                "transport.body_too_large"
            );
            let body = axum::Json(serde_json::json!({
                "error": "request_body_too_large",
                "message": "Payload too large",
                "max_bytes": 52428800,
                "received_bytes": content_length.parse::<u64>().unwrap_or(0),
                "request_id": request_id_str
            }));
            let mut new_res = body.into_response();
            *new_res.status_mut() = axum::http::StatusCode::PAYLOAD_TOO_LARGE;
            *new_res.headers_mut() = response.headers().clone();
            response = new_res;
        } else {
            let status = response.status();
            let headers = response.headers().clone();
            if let Ok(body_bytes) = axum::body::to_bytes(response.into_body(), 1024 * 1024).await {
                if let Ok(mut json) = serde_json::from_slice::<serde_json::Value>(&body_bytes) {
                    if let Some(obj) = json.as_object_mut() {
                        obj.insert("request_id".to_string(), serde_json::json!(request_id_str));
                        let body = axum::Json(json);
                        let mut new_res = body.into_response();
                        *new_res.status_mut() = status;
                        *new_res.headers_mut() = headers;
                        new_res.headers_mut().insert(
                            axum::http::header::CONTENT_TYPE,
                            axum::http::HeaderValue::from_static("application/json"),
                        );
                        response = new_res;
                    } else {
                        let mut new_res = (status, body_bytes).into_response();
                        *new_res.headers_mut() = headers;
                        response = new_res;
                    }
                } else {
                    let mut new_res = (status, body_bytes).into_response();
                    *new_res.headers_mut() = headers;
                    response = new_res;
                }
            } else {
                let mut new_res = (status, "").into_response();
                *new_res.headers_mut() = headers;
                response = new_res;
            }
        }
    }

    if let Ok(header_val) = axum::http::HeaderValue::from_str(&request_id_str) {
        response.headers_mut().insert("x-request-id", header_val.clone());
        response.headers_mut().insert("x-llamaforge-request-id", header_val);
    }

    response
}

pub(crate) async fn metrics_middleware(req: Request, next: Next) -> Response {
    let method = req.method().to_string();
    let path = if let Some(matched_path) = req.extensions().get::<axum::extract::MatchedPath>() {
        matched_path.as_str().to_string()
    } else {
        "unknown".to_string()
    };

    metrics::gauge!("llamaforge_active_connections").increment(1.0);
    let start = std::time::Instant::now();

    let response = next.run(req).await;

    let duration = start.elapsed().as_secs_f64();
    let status = response.status().as_u16().to_string();

    metrics::gauge!("llamaforge_active_connections").decrement(1.0);
    metrics::counter!("llamaforge_http_requests_total", "method" => method.clone(), "path" => path.clone(), "status" => status).increment(1);
    metrics::histogram!("llamaforge_http_request_duration_seconds", "method" => method, "path" => path).record(duration);

    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use llamaforge_core::config::{
        source::memory::InMemoryConfigSource, AppConfig, ConfigHandle, GlobalConfig,
    };
    use std::sync::Arc;
    use tower::ServiceExt;

    fn setup_mock_config_reader(api_key: Option<String>) -> Arc<ConfigHandle> {
        let global = GlobalConfig { api_key, ..Default::default() };
        let config = AppConfig { global, ..Default::default() };
        let source = Arc::new(InMemoryConfigSource::new(config));
        Arc::new(ConfigHandle::load(source).unwrap())
    }

    #[tokio::test]
    async fn test_auth_middleware_pass_through() {
        let reader = setup_mock_config_reader(None) as Arc<dyn ConfigReader>;
        let req = Request::builder().body(Body::empty()).unwrap();
        let next = axum::middleware::from_fn_with_state(reader, auth_middleware);
        let app = axum::Router::new().route("/", axum::routing::get(|| async { "ok" })).layer(next);
        let res = app.oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_auth_middleware_rejection() {
        let reader = setup_mock_config_reader(Some("secret".to_string())) as Arc<dyn ConfigReader>;
        let req = Request::builder().body(Body::empty()).unwrap();
        let next = axum::middleware::from_fn_with_state(reader, auth_middleware);
        let app = axum::Router::new().route("/", axum::routing::get(|| async { "ok" })).layer(next);
        let res = app.oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_request_id_injection() {
        let next = axum::middleware::from_fn(request_id_middleware);
        let app = axum::Router::new().route("/", axum::routing::get(|| async { "ok" })).layer(next);
        let req = Request::builder().body(Body::empty()).unwrap();
        let res = app.oneshot(req).await.unwrap();
        assert!(res.headers().contains_key("x-request-id"));
        assert!(res.headers().contains_key("x-llamaforge-request-id"));
    }

    #[tokio::test]
    async fn test_large_body_concurrency_limit() {
        let limit = LargeBodyConcurrencyLimit::new(1, 1); // 1MB threshold
        let next = axum::middleware::from_fn_with_state(limit.clone(), large_body_limit_middleware);

        let handler = |_req: Request<Body>| async move {
            // keep the permit alive long enough
            tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
            "ok"
        };

        let app = axum::Router::new().route("/", axum::routing::post(handler)).layer(next);

        let req1 = Request::builder()
            .method("POST")
            .uri("/")
            .header("content-length", "2000000")
            .body(Body::empty())
            .unwrap();
        let req2 = Request::builder()
            .method("POST")
            .uri("/")
            .header("content-length", "2000000")
            .body(Body::empty())
            .unwrap();

        // Spawn first request which will acquire the permit and hold it
        let app_clone1 = app.clone();
        let task1 = tokio::spawn(async move { app_clone1.oneshot(req1).await.unwrap() });

        // Let it start
        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;

        // Second request should be rejected because max_concurrent is 1
        let res2 = app.oneshot(req2).await.unwrap();
        assert_eq!(res2.status(), StatusCode::TOO_MANY_REQUESTS); // QueueFull mapped to 429

        task1.await.unwrap();
    }

    #[tokio::test]
    async fn test_cors_headers() {
        let reader = setup_mock_config_reader(None) as Arc<dyn ConfigReader>;
        let next = build_cors_layer(reader);
        let app = axum::Router::new().route("/", axum::routing::get(|| async { "ok" })).layer(next);

        let req = Request::builder()
            .method("OPTIONS")
            .uri("/")
            .header("origin", "http://example.com")
            .header("access-control-request-method", "POST")
            .body(Body::empty())
            .unwrap();

        let res = app.oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(res.headers().get("access-control-allow-origin").unwrap(), "http://example.com");
        assert!(res.headers().contains_key("access-control-allow-methods"));
    }

    #[tokio::test]
    async fn test_body_limit_rejection_format() {
        let next = axum::middleware::from_fn(request_id_middleware);
        let limit = tower_http::limit::RequestBodyLimitLayer::new(10);
        let app = axum::Router::new()
            .route("/", axum::routing::post(|_req: Request<axum::body::Body>| async { "ok" }))
            .layer(limit)
            .layer(next);

        let body_str = "x".repeat(100);
        let req = Request::builder()
            .method("POST")
            .uri("/")
            .header("content-length", "100")
            .body(Body::from(body_str))
            .unwrap();

        let res = app.oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::PAYLOAD_TOO_LARGE);

        let body_bytes = axum::body::to_bytes(res.into_body(), 1024).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();

        assert_eq!(json["error"], "request_body_too_large");
        assert_eq!(json["max_bytes"], 52428800);
        assert_eq!(json["received_bytes"], 100);
        assert!(json["request_id"].is_string());
    }
}
