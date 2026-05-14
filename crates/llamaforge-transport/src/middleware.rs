use axum::{
    extract::Request,
    middleware::Next,
    response::Response,
};
use llamaforge_config::ConfigReader;
use std::sync::Arc;
use tokio::sync::Semaphore;
use uuid::Uuid;

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
            Err(_) => Err(crate::error::ApiError::QueueFull),
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

        let auth_header = req
            .headers()
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|val| val.to_str().ok());

        let is_valid = match auth_header {
            Some(header) if header.starts_with("Bearer ") => {
                let token = &header[7..];
                token == expected_key
            }
            _ => false,
        };

        if !is_valid {
            metrics::counter!("llamaforge_auth_failures_total").increment(1);
            return Err(crate::error::ApiError::AuthFailure);
        }
    }

    Ok(next.run(req).await)
}

pub(crate) async fn request_id_middleware(
    mut req: Request,
    next: Next,
) -> Response {
    let request_id = Uuid::now_v7();
    let request_id_str = request_id.to_string();

    req.extensions_mut().insert(llamaforge_types::RequestId::new(request_id));

    let mut response = next.run(req).await;
    
    if let Ok(header_val) = axum::http::HeaderValue::from_str(&request_id_str) {
        response.headers_mut().insert("x-request-id", header_val.clone());
        response.headers_mut().insert("x-llamaforge-request-id", header_val);
    }

    response
}

pub(crate) async fn metrics_middleware(
    req: Request,
    next: Next,
) -> Response {
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
    use axum::{body::Body, http::{Request, StatusCode}};
    use llamaforge_config::{
        AppConfig, GlobalConfig, ConfigHandle,
        source::memory::InMemoryConfigSource,
    };
    use std::sync::Arc;
    use tower::ServiceExt;

    fn setup_mock_config_reader(api_key: Option<String>) -> Arc<ConfigHandle> {
        let mut global = GlobalConfig::default();
        global.api_key = api_key;
        let config = AppConfig {
            global,
            servers: Default::default(),
            grpc: Default::default(),
            prometheus: Default::default(),
            routing: Default::default(),
            priority_rules: Default::default(),
            web_ui: Default::default(),
        };
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
        
        let req1 = Request::builder().method("POST").uri("/").header("content-length", "2000000").body(Body::empty()).unwrap();
        let req2 = Request::builder().method("POST").uri("/").header("content-length", "2000000").body(Body::empty()).unwrap();
        
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
}