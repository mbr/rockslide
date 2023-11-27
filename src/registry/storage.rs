use std::{
    fs, io,
    path::{Path, PathBuf},
};

use axum::{async_trait, http::StatusCode, response::IntoResponse};
use thiserror::Error;
use tokio::io::{AsyncSeekExt, AsyncWrite};
use uuid::Uuid;

#[derive(Debug, Error)]
pub(crate) enum Error {
    #[error("given upload does not exist")]
    UploadDoesNotExit,
    #[error("can not upload any more, out of space")]
    OutOfSpace,
    #[error("digest did not match")]
    DigestMismatch,
    #[error("io error")]
    Io(io::Error),
}

impl IntoResponse for Error {
    #[inline]
    fn into_response(self) -> axum::response::Response {
        match self {
            Error::UploadDoesNotExit => StatusCode::NOT_FOUND.into_response(),
            Error::OutOfSpace | Error::DigestMismatch | Error::Io(_) => {
                StatusCode::INTERNAL_SERVER_ERROR.into_response()
            }
        }
    }
}

struct Digest;

struct Reference;

struct Namespace;

#[async_trait]
pub(crate) trait RegistryStorage: Send + Sync {
    async fn begin_new_upload(&self) -> Result<Uuid, Error>;

    async fn get_upload_progress(&self, upload: Uuid) -> Result<usize, Error>;

    async fn get_allocated_size(&self, upload: Uuid) -> Result<usize, Error>;

    async fn allocate_upload(&self, upload: Uuid) -> Result<usize, Error>;

    async fn get_writer(
        &self,
        start_at: u64,
        upload: Uuid,
    ) -> Result<Box<dyn AsyncWrite + Send + Unpin>, Error>;

    async fn finalize_upload(&self, upload: Uuid, hash: ()) -> Result<Digest, Error>;

    async fn cancel_upload(&self, upload: Uuid) -> Result<(), Error>;

    async fn get_manifest(
        &self,
        namespace: &Namespace,
        reference: &Reference,
    ) -> Result<Option<Vec<u8>>, Error>;
}

#[derive(Debug)]
pub(crate) struct FilesystemStorage {
    // root: PathBuf,
    uploads: PathBuf,
}

impl FilesystemStorage {
    pub(crate) fn new<P: AsRef<Path>>(root: P) -> Result<Self, io::Error> {
        let root = root.as_ref().canonicalize()?;

        let uploads = root.join("uploads");

        // Create necessary subpaths.
        if !uploads.exists() {
            fs::create_dir(&uploads)?;
        }

        Ok(FilesystemStorage {
            //root,
            uploads,
        })
    }

    fn upload_path(&self, upload: Uuid) -> PathBuf {
        self.uploads.join(format!("{}.partial", upload))
    }
}

#[async_trait]
impl RegistryStorage for FilesystemStorage {
    async fn begin_new_upload(&self) -> Result<Uuid, Error> {
        let upload = Uuid::new_v4();
        let out_path = self.upload_path(upload);
        // Write zero-sized file.
        let file = tokio::fs::File::create(out_path).await.map_err(Error::Io)?;

        Ok(upload)
    }

    async fn get_upload_progress(&self, upload: Uuid) -> Result<usize, Error> {
        todo!()
    }

    async fn get_allocated_size(&self, upload: Uuid) -> Result<usize, Error> {
        todo!()
    }

    async fn allocate_upload(&self, upload: Uuid) -> Result<usize, Error> {
        todo!()
    }

    async fn get_writer(
        &self,
        start_at: u64,
        upload: Uuid,
    ) -> Result<Box<dyn AsyncWrite + Send + Unpin>, Error> {
        let location = self.upload_path(upload);
        let mut file = tokio::fs::OpenOptions::new()
            .append(true)
            .truncate(false)
            .open(location)
            .await
            .map_err(Error::Io)?;

        file.seek(io::SeekFrom::Start(start_at))
            .await
            .map_err(Error::Io)?;

        Ok(Box::new(file))
    }

    async fn finalize_upload(&self, upload: Uuid, hash: ()) -> Result<Digest, Error> {
        todo!()
    }

    async fn cancel_upload(&self, upload: Uuid) -> Result<(), Error> {
        todo!()
    }

    async fn get_manifest(
        &self,
        namespace: &Namespace,
        reference: &Reference,
    ) -> Result<Option<Vec<u8>>, Error> {
        todo!()
    }
}
