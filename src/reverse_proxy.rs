use std::{
    collections::HashMap,
    fmt::{self, Display},
    mem,
    net::SocketAddr,
    sync::Arc,
};

use axum::{
    body::Body,
    extract::{Request, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::any,
    Router,
};
use itertools::Itertools;
use tokio::sync::RwLock;
use tracing::{trace, warn};

pub(crate) struct ReverseProxy {
    client: reqwest::Client,
    routing_table: RwLock<RoutingTable>,
}

#[derive(Clone, Debug)]
pub(crate) struct PublishedContainer {
    host_addr: SocketAddr,
    image_location: ImageLocation,
}

#[derive(Debug, Default)]
pub(crate) struct RoutingTable {
    path_maps: HashMap<ImageLocation, PublishedContainer>,
    domain_maps: HashMap<Domain, PublishedContainer>,
}

impl RoutingTable {
    #[inline(always)]
    fn get_path_route(&self, image_location: &ImageLocation) -> Option<&PublishedContainer> {
        self.path_maps.get(image_location)
    }
}

#[derive(Debug, Hash, Eq, PartialEq)]
struct Domain(String);

impl Domain {
    fn new(raw: &str) -> Option<Self> {
        let domain_name = raw.to_lowercase();
        if !domain_name.contains('.') {
            return None;
        }

        Some(Self(domain_name))
    }
}

impl PartialEq<String> for Domain {
    fn eq(&self, other: &String) -> bool {
        other.to_lowercase() == self.0
    }
}

impl RoutingTable {
    fn from_containers(containers: impl IntoIterator<Item = PublishedContainer>) -> Self {
        let mut path_maps = HashMap::new();
        let mut domain_maps = HashMap::new();

        for container in containers {
            if let Some(domain) = Domain::new(&container.image_location.repository()) {
                domain_maps.insert(domain, container.clone());
            }

            path_maps.insert(container.image_location.clone(), container);
        }

        Self {
            path_maps,
            domain_maps,
        }
    }
}

#[derive(Debug)]
enum AppError {
    NoSuchContainer,
    AssertionFailed(&'static str),
    NonUtf8Header,
    Internal(anyhow::Error),
}

impl Display for AppError {
    #[inline(always)]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AppError::NoSuchContainer => f.write_str("no such container"),
            AppError::AssertionFailed(msg) => f.write_str(msg),
            AppError::NonUtf8Header => f.write_str("a header contained non-utf8 data"),
            AppError::Internal(err) => Display::fmt(err, f),
        }
    }
}

impl<E> From<E> for AppError
where
    E: Into<anyhow::Error>,
{
    #[inline(always)]
    fn from(err: E) -> Self {
        AppError::Internal(err.into())
    }
}

impl IntoResponse for AppError {
    #[inline(always)]
    fn into_response(self) -> Response {
        match self {
            AppError::NoSuchContainer => StatusCode::NOT_FOUND.into_response(),
            AppError::AssertionFailed(msg) => (StatusCode::NOT_FOUND, msg).into_response(),
            AppError::NonUtf8Header => StatusCode::BAD_REQUEST.into_response(),
            AppError::Internal(err) => {
                (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()).into_response()
            }
        }
    }
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
            routing_table: RwLock::new(Default::default()),
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
        let mut routing_table = RoutingTable::from_containers(containers);

        let mut guard = self.routing_table.write().await;
        mem::swap(&mut *guard, &mut routing_table);
    }
}

async fn reverse_proxy(
    State(rp): State<Arc<ReverseProxy>>,
    request: Request,
) -> Result<Response<Body>, AppError> {
    // Determine rewritten URL.
    let req_uri = request.uri();

    let mut segments = req_uri
        .path()
        .split('/')
        .filter(|segment| !segment.is_empty());

    let image_location = ImageLocation::new(
        segments
            .next()
            .ok_or(AppError::AssertionFailed("repository segment disappeared"))?
            .to_owned(),
        segments
            .next()
            .ok_or(AppError::AssertionFailed("image segment disappeared"))?
            .to_owned(),
    );

    // TODO: Return better error (404?).
    let dest_addr = rp
        .routing_table
        .read()
        .await
        .get_path_route(&image_location)
        .ok_or(AppError::NoSuchContainer)?
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
    let method =
        request.method().to_string().parse().map_err(|_| {
            AppError::AssertionFailed("method http version mismatch workaround failed")
        })?;
    let response = rp.client.request(method, &dest_uri).send().await;

    match response {
        Ok(response) => {
            let mut bld = Response::builder().status(response.status().as_u16());
            for (key, value) in response.headers() {
                if HOP_BY_HOP.contains(key) {
                    continue;
                }

                let key_string = key.to_string();
                let value_str = value.to_str().map_err(|_| AppError::NonUtf8Header)?;

                bld = bld.header(key_string, value_str);
            }
            Ok(bld
                .body(Body::from(response.bytes().await?))
                .map_err(|_| AppError::AssertionFailed("should not fail to construct response"))?)
        }
        Err(err) => {
            warn!(%err, %dest_uri, "failed request");
            Ok(Response::builder()
                .status(500)
                .body(Body::empty())
                .map_err(|_| {
                    AppError::AssertionFailed("should not fail to construct error response")
                })?)
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
