use std::{collections::HashMap, mem, net::SocketAddr, sync::Arc};

use axum::{
    body::Body,
    extract::{Request, State},
    response::Response,
    routing::any,
    Router,
};
use itertools::Itertools;
use tokio::sync::RwLock;
use tracing::{trace, warn};

pub(crate) struct ReverseProxy {
    client: reqwest::Client,
    containers: RwLock<HashMap<ImageLocation, PublishedContainer>>,
}

#[derive(Debug)]
pub(crate) struct PublishedContainer {
    host_addr: SocketAddr,
    image_location: ImageLocation,
}

impl PublishedContainer {
    pub(crate) fn new(host_addr: SocketAddr, image_location: ImageLocation) -> Self {
        Self {
            host_addr,
            image_location,
        }
    }
}

impl ReverseProxy {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(ReverseProxy {
            client: reqwest::Client::new(),
            containers: RwLock::new(HashMap::new()),
        })
    }

    pub(crate) fn make_router(self: Arc<Self>) -> Router {
        Router::new()
            .route("/:repository/:image", any(reverse_proxy))
            .route("/:repository/:image/", any(reverse_proxy))
            .route("/:repository/:image/*remainder", any(reverse_proxy))
            .with_state(self)
    }

    pub(crate) async fn update_containers(
        &self,
        containers: impl Iterator<Item = PublishedContainer>,
    ) {
        let mut new_mapping = containers
            .map(|pc| (pc.image_location.clone(), pc))
            .collect();

        let mut guard = self.containers.write().await;
        mem::swap(&mut *guard, &mut new_mapping);
    }
}

async fn reverse_proxy(State(rp): State<Arc<ReverseProxy>>, request: Request) -> Response<Body> {
    // TODO: This needs a proper AppError and return a `Result`, similar to `Registry`.

    // Determine rewritten URL.
    let req_uri = request.uri();

    let mut segments = req_uri
        .path()
        .split('/')
        .filter(|segment| !segment.is_empty());

    let image_location = ImageLocation::new(
        segments.next().expect("TODO").to_owned(),
        segments.next().expect("TODO").to_owned(),
    );

    // TODO: Return better error (404?).
    let dest_addr = rp
        .containers
        .read()
        .await
        .get(&image_location)
        .expect("TODO")
        .host_addr;
    let base_url = format!("http://{dest_addr}");

    // Format is: '' / repository / image / ...
    // Need to skip the first three.
    let cleaned_path = segments.join("/");

    let mut dest_path_and_query = cleaned_path;

    if req_uri.path().ends_with('/') {
        dest_path_and_query.push('/');
    }

    if let Some(query) = req_uri.query() {
        dest_path_and_query.push('?');
        dest_path_and_query += query;
    }

    let dest_uri = format!("{base_url}/{dest_path_and_query}");
    trace!(%dest_uri, "reverse proxying");

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

use crate::registry::storage::ImageLocation;
