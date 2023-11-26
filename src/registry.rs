mod www_authenticate;

use std::{str, sync::Arc};

use axum::{
    async_trait,
    body::Body,
    extract::{FromRequestParts, State},
    http::{
        header::{self, LOCATION},
        request::Parts,
        Request, Response, StatusCode,
    },
    response::{Html, IntoResponse},
    routing::{get, post},
    Router,
};
use base64::engine::Engine as _;

#[derive(Debug)]
struct ClientSuppliedCredentials {
    username: String,
    password: String,
}

#[async_trait]
impl<S> FromRequestParts<S> for ClientSuppliedCredentials {
    type Rejection = StatusCode;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        const BASIC_AUTH_PREFIX: &str = "basic";

        if let Some(auth_header) = parts.headers.get(header::AUTHORIZATION) {
            let auth_part = str::from_utf8(auth_header.as_bytes())
                .map_err(|_| StatusCode::BAD_REQUEST)?
                .to_ascii_lowercase();

            todo!();

            let decoded =
                String::from_utf8(
                    dbg!(base64::engine::general_purpose::STANDARD
                        .decode(dbg!(auth_header.as_bytes())))
                    .map_err(|_| StatusCode::BAD_REQUEST)?,
                )
                .map_err(|_| StatusCode::BAD_REQUEST)?;
            dbg!(&decoded);

            let (username, password) = decoded.split_once(':').ok_or(StatusCode::BAD_REQUEST)?;

            Ok(ClientSuppliedCredentials {
                username: username.to_owned(),
                password: password.to_owned(),
            })
        } else {
            Err(StatusCode::UNAUTHORIZED)
        }
    }
}

// TODO: Auth
pub(crate) struct DockerRegistry {
    realm: String,
}

impl DockerRegistry {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(DockerRegistry {
            realm: "TODO REGISTRY".to_string(),
        })
    }

    pub(crate) fn make_router(self: Arc<DockerRegistry>) -> Router {
        Router::new()
            .route("/v2/", get(index_v2))
            .route("/v2/test/blobs/uploads/", post(upload_blob_test))
            .with_state(self)
    }
}

async fn index_v2(
    State(registry): State<Arc<DockerRegistry>>,
    credentials: Option<ClientSuppliedCredentials>,
    request: Request<Body>,
) -> Response<Body> {
    let realm = &registry.realm;

    dbg!(request);
    dbg!(&credentials);
    Response::builder()
        .status(StatusCode::UNAUTHORIZED)
        .header("WWW-Authenticate", format!("Basic realm=\"{realm}\""))
        .body(Body::empty())
        .unwrap()

    // let mut resp = StatusCode::UNAUTHORIZED;

    // resp.map(|_| Default::default())
}

async fn upload_blob_test(request: Request<Body>) -> Response<Body> {
    let mut resp = StatusCode::ACCEPTED.into_response();
    let location = format!("/v2/test/blobs/uploads/asdf123"); // TODO: should be uuid
    resp.headers_mut()
        .append(LOCATION, location.parse().unwrap());

    resp.map(|_| Default::default())
}
