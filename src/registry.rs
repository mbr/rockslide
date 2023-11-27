mod auth;
mod storage;
mod www_authenticate;

use std::{
    fmt::{self, Display},
    sync::Arc,
};

use self::{
    auth::{AuthProvider, UnverifiedCredentials, ValidUser},
    storage::{FilesystemStorage, RegistryStorage},
};
use axum::{
    body::Body,
    extract::{Path, State},
    http::{header::LOCATION, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Router,
};

#[derive(Debug)]
struct AppError(anyhow::Error);

impl Display for AppError {
    #[inline(always)]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        Display::fmt(&self.0, f)
    }
}

impl<E> From<E> for AppError
where
    E: Into<anyhow::Error>,
{
    #[inline(always)]
    fn from(err: E) -> Self {
        AppError(err.into())
    }
}

impl IntoResponse for AppError {
    #[inline(always)]
    fn into_response(self) -> Response {
        (StatusCode::INTERNAL_SERVER_ERROR, self.0.to_string()).into_response()
    }
}

// TODO: Auth
pub(crate) struct DockerRegistry {
    realm: String,
    auth_provider: Box<dyn AuthProvider>,
    storage: Box<dyn RegistryStorage>,
}

impl DockerRegistry {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(DockerRegistry {
            realm: "TODO REGISTRY".to_string(),
            auth_provider: Box::new(()),
            storage: Box::new(
                FilesystemStorage::new("./rockslide-storage").expect("inaccessible storage"),
            ),
        })
    }

    pub(crate) fn make_router(self: Arc<DockerRegistry>) -> Router {
        Router::new()
            .route("/v2/", get(index_v2))
            // TODO: HEAD to look for blobs
            .route("/v2/:namespace/:image/blobs/uploads/", post(new_upload))
            .with_state(self)
    }
}

async fn index_v2(
    State(registry): State<Arc<DockerRegistry>>,
    credentials: Option<UnverifiedCredentials>,
) -> Response<Body> {
    let realm = &registry.realm;

    if let Some(creds) = credentials {
        if registry.auth_provider.check_credentials(&creds).await {
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

async fn new_upload(
    State(registry): State<Arc<DockerRegistry>>,
    Path((namespace, image)): Path<(String, String)>,
    _auth: ValidUser,
) -> Result<Response<Body>, AppError> {
    // Initiate a new upload
    let upload_uuid = registry.storage.begin_new_upload().await?;
    let location = format!("/v2/{namespace}/{image}/uploads/{upload_uuid}");

    Ok(Response::builder()
        .status(StatusCode::ACCEPTED)
        .header(LOCATION, location)
        .body(Body::empty())?)
}
