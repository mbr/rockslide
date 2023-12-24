use std::sync::Arc;

use axum::{
    body::Body,
    extract::{Path, Request, State},
    response::Response,
    routing::any,
    Router,
};
use itertools::Itertools;
use tracing::warn;

pub(crate) struct ReverseProxy {
    client: reqwest::Client,
}

impl ReverseProxy {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(ReverseProxy {
            client: reqwest::Client::new(),
        })
    }

    pub(crate) fn make_router(self: Arc<Self>) -> Router {
        Router::new()
            .route("/:repository/:image", any(reverse_proxy))
            .route("/:repository/:image/", any(reverse_proxy))
            .route("/:repository/:image/*remainder", any(reverse_proxy))
            .with_state(self)
    }
}

async fn reverse_proxy(State(rp): State<Arc<ReverseProxy>>, request: Request) -> Response<Body> {
    let base_url = "http://echo.free.beeceptor.com"; // TODO

    // Determine rewritten URL.
    let req_uri = request.uri();

    // Format is: '' / repository / image / ...
    // Need to skip the first three.
    let cleaned_path = req_uri
        .path()
        .split('/')
        .filter(|segment| !segment.is_empty())
        .skip(2)
        .join("/");

    let mut dest_path_and_query = cleaned_path;

    if req_uri.path().ends_with('/') {
        dest_path_and_query.push('/');
    }

    if let Some(query) = req_uri.query() {
        dest_path_and_query.push('?');
        dest_path_and_query += query;
    }

    let dest_uri = format!("{base_url}/{dest_path_and_query}");

    // Note: `reqwest` and `axum` currently use different versions of `http`
    let method = request
        .method()
        .to_string()
        .parse()
        .expect("method http version mismatch workaround failed");
    let response = rp.client.request(method, &dest_uri).send().await;

    match response {
        Ok(response) => {
            let mut bld = Response::builder().status(response.status().as_u16());
            for (key, value) in response.headers() {
                if HOP_BY_HOP.contains(key) {
                    continue;
                }

                let key_string = key.to_string();
                let value_str = value.to_str().expect("TODO:Handle");

                bld = bld.header(key_string, value_str);
            }
            bld.body(Body::from(response.bytes().await.expect("TODO: Handle")))
                .expect("should not fail to construct response")
        }
        Err(err) => {
            warn!(%err, %dest_uri, "failed request");
            Response::builder()
                .status(500)
                .body(Body::empty())
                .expect("should not fail to construct error response")
        }
    }
}

/// HTTP/1.1 hop-by-hop headers
mod hop_by_hop {
    use reqwest::header::HeaderName;
    pub(super) const HOP_BY_HOP: [HeaderName; 8] = [
        HeaderName::from_static("keep-alive"),
        HeaderName::from_static("transfer-encoding"),
        HeaderName::from_static("te"),
        HeaderName::from_static("connection"),
        HeaderName::from_static("trailer"),
        HeaderName::from_static("upgrade"),
        HeaderName::from_static("proxy-authorization"),
        HeaderName::from_static("proxy-authenticate"),
    ];
}
use hop_by_hop::HOP_BY_HOP;
