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
    http::{
        header::{CONTENT_LENGTH, LOCATION, RANGE},
        StatusCode,
    },
    response::{IntoResponse, Response},
    routing::{get, patch, post},
    Router,
};
use futures::stream::StreamExt;
use tokio::io::AsyncWriteExt;
use uuid::Uuid;

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
            .route("/v2/:namespace/:image/blobs/uploads/", post(upload_new))
            .route(
                "/v2/:namespace/:image/uploads/:upload_uuid",
                patch(upload_add_chunk),
            )
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

async fn upload_new(
    State(registry): State<Arc<DockerRegistry>>,
    Path((namespace, image)): Path<(String, String)>,
    _auth: ValidUser,
) -> Result<UploadState, AppError> {
    // Initiate a new upload
    let upload = registry.storage.begin_new_upload().await?;

    Ok(UploadState {
        namespace,
        image,
        completed: None,
        upload,
    })
}

fn mk_upload_location(namespace: &str, image: &str, uuid: Uuid) -> String {
    format!("/v2/{namespace}/{image}/uploads/{uuid}")
}

#[derive(Debug)]
struct UploadState {
    namespace: String,
    image: String,
    completed: Option<u64>,
    upload: Uuid,
}

impl IntoResponse for UploadState {
    fn into_response(self) -> Response {
        let mut builder = Response::builder()
            .header(
                LOCATION,
                mk_upload_location(&self.namespace, &self.image, self.upload),
            )
            .header(CONTENT_LENGTH, 0)
            .header("Docker-Upload-UUID", self.upload.to_string());

        if let Some(completed) = self.completed {
            builder = builder
                .header(RANGE, format!("bytes=0-{}", completed))
                .status(StatusCode::NO_CONTENT)
        } else {
            builder = builder
                .header(CONTENT_LENGTH, 0)
                .status(StatusCode::ACCEPTED);
            // The spec says to use `CREATED`, but only `ACCEPTED` works?
        }

        builder.body(Body::empty()).unwrap()
    }
}

async fn upload_add_chunk(
    State(registry): State<Arc<DockerRegistry>>,
    // TODO: Extract UUID with correct type
    Path((namespace, image, upload)): Path<(String, String, Uuid)>,
    _auth: ValidUser,
    request: axum::extract::Request,
) -> Result<UploadState, AppError> {
    // Check if we have a range - if so, its an unsupported feature, namely monolit uploads.
    if request.headers().contains_key(RANGE) {
        return Err(anyhow::anyhow!("unsupport feature: chunked uploads").into());
    }

    let mut writer = registry.storage.get_writer(0, upload).await?;

    // We'll get the entire file in one go, no range header == monolithic uploads.
    let mut body = request.into_body().into_data_stream();

    let mut completed: u64 = 0;
    while let Some(result) = body.next().await {
        let chunk = result?;
        completed += chunk.len() as u64;
        writer.write_all(chunk.as_ref()).await?;
    }

    writer.flush().await?;

    Ok(UploadState {
        namespace,
        image,
        completed: Some(completed),
        upload,
    })
}
