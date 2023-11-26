mod auth;
mod www_authenticate;

use std::{str, sync::Arc};

use axum::{
    body::Body,
    extract::State,
    http::{header::LOCATION, Request, Response, StatusCode},
    response::IntoResponse,
    routing::{get, post},
    Router,
};

use self::auth::{AuthProvider, UnverifiedCredentials};

// TODO: Auth
pub(crate) struct DockerRegistry {
    realm: String,
    auth_provider: Box<dyn AuthProvider>,
}

impl DockerRegistry {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(DockerRegistry {
            realm: "TODO REGISTRY".to_string(),
            auth_provider: Box::new(()),
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
    credentials: Option<UnverifiedCredentials>,
) -> Response<Body> {
    let realm = &registry.realm;

    if let Some(creds) = credentials {
        if registry.auth_provider.check_credentials(&creds) {
            return Response::builder()
                .status(StatusCode::OK)
                .header("WWW-Authenticate", format!("Basic realm=\"{realm}\""))
                .body(Body::empty())
                .unwrap();
        }
    }

    // Return `UNAUTHORIZED`, since we want the client to supply credentials.
    Response::builder()
        .status(StatusCode::UNAUTHORIZED)
        .header("WWW-Authenticate", format!("Basic realm=\"{realm}\""))
        .body(Body::empty())
        .unwrap()
}

async fn upload_blob_test(request: Request<Body>) -> Response<Body> {
    let mut resp = StatusCode::ACCEPTED.into_response();
    let location = format!("/v2/test/blobs/uploads/asdf123"); // TODO: should be uuid
    resp.headers_mut()
        .append(LOCATION, location.parse().unwrap());

    resp.map(|_| Default::default())
}
