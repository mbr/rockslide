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
        header::HOST,
        uri::{Authority, Parts, PathAndQuery, Scheme},
        Method, StatusCode, Uri,
    },
    response::{IntoResponse, Response},
    RequestExt, Router,
};
use tokio::sync::RwLock;
use tracing::{info, trace, warn};

use crate::{
    container_orchestrator::{ContainerOrchestrator, PublishedContainer, RuntimeConfig},
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

impl Display for RoutingTable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut first = true;

        for (location, container) in &self.path_maps {
            if !first {
                f.write_str(", ")?;
            }

            write!(f, "/{} -> {}", location, container)?;
            first = false;
        }

        for (domain, container) in &self.domain_maps {
            if !first {
                f.write_str(", ")?;
            }

            write!(f, "{} -> {}", domain, container)?;
            first = false;
        }

        Ok(())
    }
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

impl Display for Domain {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        <String as Display>::fmt(&self.0, f)
    }
}

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
    ReverseProxied {
        uri: Uri,
        script_name: Option<String>,
        config: Arc<RuntimeConfig>,
    },
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
            return Destination::ReverseProxied {
                uri: Uri::from_parts(parts).expect("should not have invalidated Uri"),
                script_name: None,
                config: pc.config().clone(),
            };
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

                return Destination::ReverseProxied {
                    uri: Uri::from_parts(parts).unwrap(),
                    script_name: Some(format!("/{}", image_location)),
                    config: pc.config().clone(),
                };
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
    AuthFailure {
        realm: &'static str,
        status: StatusCode,
    },
    InvalidPayload,
    BodyReadError(axum::Error),
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
            AppError::AuthFailure { .. } => f.write_str("authentication missing or not present"),
            AppError::InvalidPayload => f.write_str("invalid payload"),
            AppError::BodyReadError(err) => write!(f, "could not read body: {}", err),
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
            AppError::AuthFailure { realm, status } => Response::builder()
                .status(status)
                .header("WWW-Authenticate", format!("basic realm={realm}"))
                .body(Body::empty())
                .expect("should never fail to build auth failure response"),
            AppError::InvalidPayload => StatusCode::BAD_REQUEST.into_response(),
            // TODO: Could probably be more specific here instead of just `BAD_REQUEST`:
            AppError::BodyReadError(_) => StatusCode::BAD_REQUEST.into_response(),
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
        info!(
            routing_table = %*guard,
            "reverse proxy updated routing table"
        )
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
    let mut remainder = String::new();

    segments.for_each(|segment| {
        remainder.push('/');
        remainder.push_str(segment);
    });

    Some((image_location, remainder))
}

async fn route_request(
    State(rp): State<Arc<ReverseProxy>>,
    mut request: Request,
) -> Result<Response, AppError> {
    let dest_uri = {
        let routing_table = rp.routing_table.read().await;
        routing_table.get_destination_uri_from_request(&request)
    };

    match dest_uri {
        Destination::ReverseProxied {
            uri: dest,
            script_name,
            config,
        } => {
            trace!(%dest, "reverse proxying");

            // First, check if http authentication is enabled.
            if let Some(ref http_access) = config.http.access {
                let creds = request
                    .extract_parts::<UnverifiedCredentials>()
                    .await
                    .map_err(|status| AppError::AuthFailure {
                        // TODO: Output container name?
                        realm: "password protected container",
                        status,
                    })?;

                if !http_access.check_credentials(&creds).await {
                    return Err(AppError::AuthFailure {
                        realm: "password protected container",
                        status: StatusCode::UNAUTHORIZED,
                    });
                }
            }

            // Note: `reqwest` and `axum` currently use different versions of `http`
            let method = request.method().to_string().parse().map_err(|_| {
                AppError::AssertionFailed("method http version mismatch workaround failed")
            })?;

            let mut req = rp.client.request(method, dest.to_string());

            for (name, value) in request.headers() {
                let name: reqwest::header::HeaderName = if let Ok(name) = name.as_str().parse() {
                    name
                } else {
                    continue;
                };

                if !BLACKLISTED.contains(&name) && !HOP_BY_HOP.contains(&name) {
                    if let Ok(value) = value.to_str() {
                        req = req.header(name, value);
                    } else {
                        continue;
                    }
                }
            }

            // Attach script name.
            if let Some(script_name) = script_name {
                req = req.header("X-Script-Name", script_name);
            };

            // Retrieve body.
            let request_body = axum::body::to_bytes(
                request.into_limited_body(),
                1024 * 1024, // See #43.
            )
            .await
            .map_err(AppError::BodyReadError)?;
            req = req.body(request_body);

            let response = req.send().await;

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

                    let body = response.bytes().await?;
                    Ok(bld.body(Body::from(body)).map_err(|_| {
                        AppError::AssertionFailed("should not fail to construct response")
                    })?)
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
        Destination::Internal(uri) => {
            let method = request.method().clone();
            // Note: The auth functionality has been lifted from `registry`. It may need to be
            //       refactored out because of that.
            let creds: UnverifiedCredentials =
                request
                    .extract_parts()
                    .await
                    .map_err(|status| AppError::AuthFailure {
                        realm: "internal",
                        status,
                    })?;

            let opt_body = request
                .extract::<Option<String>, _>()
                .await
                .expect("infallible");

            // Any internal URL is subject to requiring auth through the master key.
            if !rp.auth_provider.check_credentials(&creds).await {
                return Err(AppError::AuthFailure {
                    realm: "internal",
                    status: StatusCode::UNAUTHORIZED,
                });
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

            match method {
                Method::GET => {
                    let config = orchestrator
                        .load_config(&manifest_reference)
                        .await
                        .map_err(AppError::Internal)?;

                    Ok(config.into_response())
                }
                Method::PUT => {
                    let raw = opt_body.ok_or(AppError::InvalidPayload)?;
                    let new_config: RuntimeConfig =
                        toml::from_str(&raw).map_err(|_| AppError::InvalidPayload)?;
                    let stored = orchestrator
                        .save_config(&manifest_reference, &new_config)
                        .await
                        .map_err(AppError::Internal)?;

                    // Update containers.
                    orchestrator.updated_published_set().await;

                    Ok(stored.into_response())
                }
                _ => Err(AppError::InternalUrlInvalid),
            }
        }
        Destination::NotFound => Err(AppError::NoSuchContainer),
    }
}

/// HTTP/1.1 hop-by-hop headers
mod known_headers {
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
    pub(super) static BLACKLISTED: [HeaderName; 1] = [HeaderName::from_static("x-script-name")];
}
use known_headers::BLACKLISTED;
use known_headers::HOP_BY_HOP;
