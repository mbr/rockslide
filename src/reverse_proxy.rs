use std::{
    collections::HashMap,
    fmt::{self, Display},
    mem,
    net::SocketAddr,
    str::FromStr,
    sync::Arc,
};

use axum::{
    body::Body,
    extract::{Request, State},
    http::{
        uri::{Authority, Scheme},
        StatusCode, Uri,
    },
    response::{IntoResponse, Response},
    Router,
};
use itertools::Itertools;
use tokio::sync::RwLock;
use tracing::{trace, warn};

use crate::registry::storage::ImageLocation;

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

    #[inline(always)]
    fn get_domain_route(&self, domain: &Domain) -> Option<&PublishedContainer> {
        self.domain_maps.get(domain)
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
        Router::new().fallback(route_request).with_state(self)
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

async fn route_request(
    State(rp): State<Arc<ReverseProxy>>,
    request: Request,
) -> Result<Response, AppError> {
    let req_uri = request.uri();

    let opt_domain = req_uri.host().and_then(Domain::new);
    let routing_table = rp.routing_table.read().await;

    let dest_uri = if let Some(pc) =
        opt_domain.and_then(|domain| routing_table.get_domain_route(&domain))
    {
        // We only need to swap the protocol and domain and we're good to go.
        let mut parts = req_uri.clone().into_parts();
        parts.scheme = Some(Scheme::HTTP);
        parts.authority = Some(Authority::from_str(&pc.host_addr.to_string()).map_err(|_| {
            anyhow::anyhow!("failed to convert container address into `Authority`")
        })?);
        Some(
            Uri::from_parts(parts)
                .map_err(|_| anyhow::anyhow!("did not expect invalid uri parts"))?
                .to_string(),
        )
    } else {
        // Reconstruct image location from path segments, keeping remainder intact.
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

        if let Some(pc) = routing_table.get_path_route(&image_location) {
            let container_addr = pc.host_addr;

            // Now create the path, format is: '' / repository / image / ...
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

            // Reassemble
            Some(format!("http://{container_addr}/{dest_path_and_query}"))
        } else {
            None
        }
    };

    // Release lock.
    drop(routing_table);

    // TODO: Return better error (404?).
    let dest = dest_uri.ok_or(AppError::NoSuchContainer)?;
    trace!(%dest, "reverse proxying");

    // Note: `reqwest` and `axum` currently use different versions of `http`
    let method =
        request.method().to_string().parse().map_err(|_| {
            AppError::AssertionFailed("method http version mismatch workaround failed")
        })?;
    let response = rp.client.request(method, &dest).send().await;

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
            warn!(%err, %dest, "failed request");
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
