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
    response::IntoResponse,
    routing::{get, post},
    Router,
};
use sec::Secret;

#[derive(Debug)]
struct ClientSuppliedCredentials {
    username: String,
    password: Secret<String>,
}

#[async_trait]
impl<S> FromRequestParts<S> for ClientSuppliedCredentials {
    type Rejection = StatusCode;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        if let Some(auth_header) = parts.headers.get(header::AUTHORIZATION) {
            let (_unparsed, basic) = www_authenticate::basic_auth_response(auth_header.as_bytes())
                .map_err(|_| StatusCode::BAD_REQUEST)?;

            Ok(ClientSuppliedCredentials {
                username: str::from_utf8(&basic.username)
                    .map_err(|_| StatusCode::BAD_REQUEST)?
                    .to_owned(),
                password: Secret::new(
                    str::from_utf8(&basic.password)
                        .map_err(|_| StatusCode::BAD_REQUEST)?
                        .to_owned(),
                ),
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
) -> Response<Body> {
    let realm = &registry.realm;

    if credentials.is_none() {
        // Return `UNAUTHORIZED`, since we want the client to supply credentials.
        Response::builder()
            .status(StatusCode::UNAUTHORIZED)
            .header("WWW-Authenticate", format!("Basic realm=\"{realm}\""))
            .body(Body::empty())
            .unwrap()
    } else {
        // TODO: Validate credentials.
        Response::builder()
            .status(StatusCode::OK)
            .header("WWW-Authenticate", format!("Basic realm=\"{realm}\""))
            .body(Body::empty())
            .unwrap()
    }
}

async fn upload_blob_test(request: Request<Body>) -> Response<Body> {
    let mut resp = StatusCode::ACCEPTED.into_response();
    let location = format!("/v2/test/blobs/uploads/asdf123"); // TODO: should be uuid
    resp.headers_mut()
        .append(LOCATION, location.parse().unwrap());

    resp.map(|_| Default::default())
}
