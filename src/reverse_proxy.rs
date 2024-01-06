use std::{
    collections::HashMap,
    fmt::{self, Display},
    mem,
    str::{self, FromStr},
    sync::{Arc, OnceLock},
};

use axum::{
    body::Body,
    extract::{Request, State},
    http::{
        header::{CONTENT_TYPE, HOST},
        uri::{Authority, Parts, PathAndQuery, Scheme},
        Method, StatusCode, Uri,
    },
    response::{IntoResponse, Response},
    RequestExt, Router,
};
use itertools::Itertools;
use tokio::sync::RwLock;
use tracing::{trace, warn};

use crate::{
    container_orchestrator::{ContainerOrchestrator, PublishedContainer},
    registry::{
        storage::ImageLocation, AuthProvider, ManifestReference, Reference, UnverifiedCredentials,
    },
};

pub(crate) struct ReverseProxy {
    auth_provider: Arc<dyn AuthProvider>,
    client: reqwest::Client,
    routing_table: RwLock<RoutingTable>,
    orchestrator: OnceLock<Arc<ContainerOrchestrator>>,
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

#[derive(Debug)]
enum Destination {
    ReverseProxied(Uri),
    Internal(Uri),
    NotFound,
}

impl RoutingTable {
    fn from_containers(containers: impl IntoIterator<Item = PublishedContainer>) -> Self {
        let mut path_maps = HashMap::new();
        let mut domain_maps = HashMap::new();

        for container in containers {
            if let Some(domain) =
                Domain::new(container.manifest_reference().location().repository())
            {
                domain_maps.insert(domain, container.clone());
            }

            path_maps.insert(container.manifest_reference().location().clone(), container);
        }

        Self {
            path_maps,
            domain_maps,
        }
    }

    fn get_destination_uri_from_request(&self, request: &Request) -> Destination {
        let req_uri = request.uri();

        // First, attempt to match a domain.
        let opt_domain = if let Some(host_header) = request
            .headers()
            .get(HOST)
            .and_then(|h| str::from_utf8(h.as_bytes()).ok())
        {
            let candidate = if let Some(colon) = host_header.rfind(':') {
                &host_header[..colon]
            } else {
                host_header
            };

            Domain::new(candidate)
        } else {
            None
        };

        if let Some(pc) = opt_domain.and_then(|domain| self.get_domain_route(&domain)) {
            // We only need to swap the protocol and domain and we're good to go.
            let mut parts = req_uri.clone().into_parts();
            parts.scheme = Some(Scheme::HTTP);
            parts.authority = Some(
                Authority::from_str(&pc.host_addr().to_string())
                    .expect("SocketAddr should never fail to convert to Authority"),
            );
            return Destination::ReverseProxied(
                Uri::from_parts(parts).expect("should not have invalidated Uri"),
            );
        }

        // Matching a domain did not succeed, let's try with a path.
        // First, we attempt to match a special `_rockslide` path:
        if req_uri.path().starts_with("/_rockslide") {
            return Destination::Internal(req_uri.to_owned());
        }

        // Reconstruct image location from path segments, keeping remainder intact.
        if let Some((image_location, remainder)) = split_path_base_url(req_uri) {
            if let Some(pc) = self.get_path_route(&image_location) {
                let container_addr = pc.host_addr();

                let mut dest_path_and_query = remainder;

                if req_uri.path().ends_with('/') {
                    dest_path_and_query.push('/');
                }

                if let Some(query) = req_uri.query() {
                    dest_path_and_query.push('?');
                    dest_path_and_query += query;
                }

                let mut parts = Parts::default();
                parts.scheme = Some(Scheme::HTTP);
                parts.authority = Some(Authority::from_str(&container_addr.to_string()).unwrap());
                parts.path_and_query = Some(PathAndQuery::from_str(&dest_path_and_query).unwrap());

                return Destination::ReverseProxied(Uri::from_parts(parts).unwrap());
            }
        }

        Destination::NotFound
    }
}

#[derive(Debug)]
enum AppError {
    NoSuchContainer,
    InternalUrlInvalid,
    AssertionFailed(&'static str),
    NonUtf8Header,
    AuthFailure(StatusCode),
    Internal(anyhow::Error),
}

impl Display for AppError {
    #[inline(always)]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AppError::NoSuchContainer => f.write_str("no such container"),
            AppError::InternalUrlInvalid => f.write_str("internal url invalid"),
            AppError::AssertionFailed(msg) => f.write_str(msg),
            AppError::NonUtf8Header => f.write_str("a header contained non-utf8 data"),
            AppError::AuthFailure(_) => f.write_str("authentication missing or not present"),
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
            AppError::InternalUrlInvalid => StatusCode::NOT_FOUND.into_response(),
            AppError::AssertionFailed(msg) => (StatusCode::NOT_FOUND, msg).into_response(),
            AppError::NonUtf8Header => StatusCode::BAD_REQUEST.into_response(),
            AppError::AuthFailure(status) => status.into_response(),
            AppError::Internal(err) => {
                (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()).into_response()
            }
        }
    }
}

impl ReverseProxy {
    pub(crate) fn new(auth_provider: Arc<dyn AuthProvider>) -> Arc<Self> {
        Arc::new(ReverseProxy {
            auth_provider,
            client: reqwest::Client::new(),
            routing_table: RwLock::new(Default::default()),
            orchestrator: OnceLock::new(),
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

    pub(crate) fn set_orchestrator(&self, orchestrator: Arc<ContainerOrchestrator>) -> &Self {
        self.orchestrator
            .set(orchestrator)
            .map_err(|_| ())
            .expect("set already set orchestrator");
        self
    }
}

fn split_path_base_url(uri: &Uri) -> Option<(ImageLocation, String)> {
    // Reconstruct image location from path segments, keeping remainder intact.
    let mut segments = uri.path().split('/').filter(|segment| !segment.is_empty());

    let image_location =
        ImageLocation::new(segments.next()?.to_owned(), segments.next()?.to_owned());

    // Now create the path, format is: '' / repository / image / ...
    // Need to skip the first three.
    let remainder = segments.join("/");

    Some((image_location, remainder))
}

async fn route_request(
    State(rp): State<Arc<ReverseProxy>>,
    request: Request,
) -> Result<Response, AppError> {
    let dest_uri = {
        let routing_table = rp.routing_table.read().await;
        routing_table.get_destination_uri_from_request(&request)
    };

    let dest = match dest_uri {
        Destination::ReverseProxied(dest) => dest,
        Destination::Internal(uri) => {
            let method = request.method().clone();
            // Note: The auth functionality has been lifted from `registry`. It may need to be
            //       refactored out because of that.
            let creds = request
                .extract::<UnverifiedCredentials, _>()
                .await
                .map_err(AppError::AuthFailure)?;

            // Any internal URL is subject to requiring auth through the master key.
            if !rp.auth_provider.check_credentials(&creds).await {
                return Response::builder()
                    .status(StatusCode::FORBIDDEN)
                    .body(Body::empty())
                    .map_err(|_| AppError::AssertionFailed("should not fail to build response"));
            }

            let remainder = uri
                .path()
                .strip_prefix("/_rockslide/config/")
                .ok_or(AppError::InternalUrlInvalid)?;

            let parts = remainder.split('/').collect::<Vec<_>>();
            if parts.len() != 3 {
                return Err(AppError::InternalUrlInvalid);
            }

            if parts[2] != "prod" {
                return Err(AppError::InternalUrlInvalid);
            }

            let manifest_reference = ManifestReference::new(
                ImageLocation::new(parts[0].to_owned(), parts[1].to_owned()),
                Reference::new_tag(parts[2]),
            );

            let orchestrator = rp
                .orchestrator
                .get()
                .ok_or_else(|| AppError::AssertionFailed("no orchestrator configured"))?;

            return match method {
                Method::GET => {
                    let config = orchestrator
                        .load_config(&manifest_reference)
                        .await
                        .map_err(AppError::Internal)?;
                    let config_toml = toml::to_string_pretty(&config)
                        .map_err(|err| AppError::Internal(err.into()))?;

                    Response::builder()
                        .status(StatusCode::OK)
                        .header(CONTENT_TYPE, "application/toml")
                        .body(Body::from(config_toml))
                        .map_err(|_| AppError::AssertionFailed("should not fail to build response"))
                }
                Method::PUT => {
                    todo!("handle PUT");
                    Response::builder()
                        .status(StatusCode::OK)
                        // .header(CONTENT_TYPE, "application/toml")
                        .body(Body::from("TODO: Replace"))
                        .map_err(|_| AppError::AssertionFailed("should not fail to build response"))
                }
                _ => Err(AppError::InternalUrlInvalid),
            };
        }
        Destination::NotFound => {
            return Err(AppError::NoSuchContainer);
        }
    };
    trace!(%dest, "reverse proxying");

    // Note: `reqwest` and `axum` currently use different versions of `http`
    let method =
        request.method().to_string().parse().map_err(|_| {
            AppError::AssertionFailed("method http version mismatch workaround failed")
        })?;
    let response = rp.client.request(method, dest.to_string()).send().await;

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
    pub(super) static HOP_BY_HOP: [HeaderName; 8] = [
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
