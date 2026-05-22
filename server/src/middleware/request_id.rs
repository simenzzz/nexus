use axum::body::Body;
use axum::http::{Request, Response};
use axum::middleware::Next;

const X_REQUEST_ID: &str = "x-request-id";

pub async fn request_id_middleware(mut req: Request<Body>, next: Next) -> Response<Body> {
    let request_id = req
        .headers()
        .get(X_REQUEST_ID)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

    // Insert into request extensions for downstream use
    req.extensions_mut().insert(RequestId(request_id.clone()));

    let mut response = next.run(req).await;

    // Echo in response header
    if let Ok(val) = axum::http::HeaderValue::from_str(&request_id) {
        response.headers_mut().insert(X_REQUEST_ID, val);
    }

    response
}

#[derive(Clone)]
pub struct RequestId(pub String);
