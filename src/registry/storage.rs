use std::{
    collections::HashMap,
    fmt::Display,
    fs,
    io::{self, Read},
    path::{Path, PathBuf},
};

use axum::{async_trait, http::StatusCode, response::IntoResponse};
use serde::Deserialize;
use sha2::Digest as Sha2Digest;
use thiserror::Error;
use tokio::io::{AsyncRead, AsyncSeekExt, AsyncWrite};
use tracing_subscriber::Layer;
use uuid::Uuid;

use super::types::ImageManifest;

const SHA256_LEN: usize = 32;

const BUFFER_SIZE: usize = 1024 * 1024 * 1024; // 1 MiB

#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord, Hash)]
pub(crate) struct Digest([u8; SHA256_LEN]);

impl Digest {
    pub(crate) fn new(bytes: [u8; SHA256_LEN]) -> Self {
        Self(bytes)
    }
}

impl Display for Digest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&hex::encode(&self.0[..]))
    }
}

#[derive(Debug, Deserialize)]
struct LayerManifest {
    #[serde(rename = "camelCase")]
    blob_sum: String,
}

// TODO: Remove `Deserialize`, should not leak into `storage` module.
#[derive(Debug, Deserialize)]
pub(crate) struct ImageLocation {
    repository: String,
    image: String,
}

impl ImageLocation {
    #[inline(always)]
    pub(crate) fn repository(&self) -> &str {
        self.repository.as_ref()
    }

    #[inline(always)]
    pub(crate) fn image(&self) -> &str {
        self.image.as_ref()
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct Reference(String);

impl Reference {
    fn name(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Error)]
pub(crate) enum Error {
    #[error("given upload does not exist")]
    UploadDoesNotExit,
    #[error("digest did not match")]
    DigestMismatch,

    // Not great to have a catch-all IO error, to be replaced later.
    #[error("io error")]
    Io(io::Error),
    #[error("background task panicked")]
    BackgroundTaskPanicked(#[source] tokio::task::JoinError),
}

impl IntoResponse for Error {
    #[inline]
    fn into_response(self) -> axum::response::Response {
        match self {
            Error::UploadDoesNotExit => StatusCode::NOT_FOUND.into_response(),
            Error::DigestMismatch | Error::Io(_) | Error::BackgroundTaskPanicked(_) => {
                StatusCode::INTERNAL_SERVER_ERROR.into_response()
            }
        }
    }
}

#[derive(Debug)]
pub(crate) struct BlobMetadata {
    digest: Digest,
    size: u64,
}

impl BlobMetadata {
    pub(crate) fn digest(&self) -> Digest {
        self.digest
    }

    pub(crate) fn size(&self) -> u64 {
        self.size
    }
}

#[async_trait]
pub(crate) trait RegistryStorage: Send + Sync {
    async fn begin_new_upload(&self) -> Result<Uuid, Error>;

    async fn get_upload_progress(&self, upload: Uuid) -> Result<usize, Error>;

    async fn get_allocated_size(&self, upload: Uuid) -> Result<usize, Error>;

    async fn get_blob_reader(
        &self,
        digest: Digest,
    ) -> Result<Option<Box<dyn AsyncRead + Send + Unpin>>, Error>;

    async fn get_blob_metadata(&self, digest: Digest) -> Result<Option<BlobMetadata>, Error>;

    async fn allocate_upload(&self, upload: Uuid) -> Result<usize, Error>;

    async fn get_writer(
        &self,
        start_at: u64,
        upload: Uuid,
    ) -> Result<Box<dyn AsyncWrite + Send + Unpin>, Error>;

    async fn finalize_upload(&self, upload: Uuid, hash: Digest) -> Result<(), Error>;

    async fn cancel_upload(&self, upload: Uuid) -> Result<(), Error>;

    async fn get_manifest(&self, location: &ImageLocation) -> Result<Option<Vec<u8>>, Error>;

    async fn put_manifest(
        &self,
        location: &ImageLocation,
        reference: &Reference,
        manifest: &ImageManifest,
    ) -> Result<(), Error>;
}

#[derive(Debug)]
pub(crate) struct FilesystemStorage {
    uploads: PathBuf,
    blobs: PathBuf,
    manifests: PathBuf,
}

impl FilesystemStorage {
    pub(crate) fn new<P: AsRef<Path>>(root: P) -> Result<Self, io::Error> {
        let root = root.as_ref().canonicalize()?;

        let uploads = root.join("uploads");
        let blobs = root.join("blobs");
        let manifests = root.join("manifests");

        // Create necessary subpaths.
        if !uploads.exists() {
            fs::create_dir(&uploads)?;
        }

        if !blobs.exists() {
            fs::create_dir(&blobs)?;
        }

        if !manifests.exists() {
            fs::create_dir(&manifests)?;
        }

        Ok(FilesystemStorage {
            uploads,
            blobs,
            manifests,
        })
    }
    fn blob_path(&self, digest: Digest) -> PathBuf {
        self.blobs.join(format!("{}", digest))
    }
    fn upload_path(&self, upload: Uuid) -> PathBuf {
        self.uploads.join(format!("{}.partial", upload))
    }

    fn manifest_path(&self, location: &ImageLocation, reference: &Reference) -> PathBuf {
        self.manifests
            .join(location.repository())
            .join(location.image())
            .join(reference.name())
    }
}

#[async_trait]
impl RegistryStorage for FilesystemStorage {
    async fn begin_new_upload(&self) -> Result<Uuid, Error> {
        let upload = Uuid::new_v4();
        let out_path = self.upload_path(upload);

        // Write zero-sized file.
        let _file = tokio::fs::File::create(out_path).await.map_err(Error::Io)?;

        Ok(upload)
    }
    async fn get_upload_progress(&self, _upload: Uuid) -> Result<usize, Error> {
        todo!()
    }

    async fn get_allocated_size(&self, _upload: Uuid) -> Result<usize, Error> {
        todo!()
    }

    async fn get_blob_metadata(&self, digest: Digest) -> Result<Option<BlobMetadata>, Error> {
        let blob_path = self.blob_path(digest);

        if !blob_path.exists() {
            return Ok(None);
        }

        let metadata = tokio::fs::metadata(blob_path).await.map_err(Error::Io)?;

        Ok(Some(BlobMetadata {
            digest,
            size: metadata.len(),
        }))
    }

    async fn get_blob_reader(
        &self,
        digest: Digest,
    ) -> Result<Option<Box<dyn AsyncRead + Send + Unpin>>, Error> {
        let blob_path = self.blob_path(digest);

        if !blob_path.exists() {
            return Ok(None);
        }

        let reader = tokio::fs::File::open(blob_path).await.map_err(Error::Io)?;

        Ok(Some(Box::new(reader)))
    }

    async fn allocate_upload(&self, _upload: Uuid) -> Result<usize, Error> {
        todo!()
    }

    async fn get_writer(
        &self,
        start_at: u64,
        upload: Uuid,
    ) -> Result<Box<dyn AsyncWrite + Send + Unpin>, Error> {
        let location = self.upload_path(upload);

        if !location.exists() {
            return Err(Error::UploadDoesNotExit);
        }

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

    async fn finalize_upload(&self, upload: Uuid, digest: Digest) -> Result<(), Error> {
        // We are to validate the uploaded partial, then move it into the proper store.
        // TODO: Lock in place so that the hash cannot be corrupted/attacked.

        let upload_path = self.upload_path(upload);

        if !upload_path.exists() {
            return Err(Error::UploadDoesNotExit);
        }

        // We offload hashing to a blocking thread.
        let actual = {
            let upload_path = upload_path.clone();
            tokio::task::spawn_blocking::<_, Result<Digest, Error>>(move || {
                let mut src = fs::File::open(upload_path).map_err(Error::Io)?;

                // Uses `vec!` instead of `Box`, as initializing the latter blows the stack:
                let mut buf = vec![0; BUFFER_SIZE];
                let mut hasher = sha2::Sha256::new();

                loop {
                    let read = src.read(buf.as_mut()).map_err(Error::Io)?;
                    if read == 0 {
                        break;
                    }
                    hasher.update(&buf[..read]);
                }

                let actual = hasher.finalize();
                Ok(Digest::new(actual.into()))
            })
        }
        .await
        .map_err(Error::BackgroundTaskPanicked)??;

        if actual != digest {
            return Err(Error::DigestMismatch);
        }

        // The uploaded file matches, we can rename it now.
        let dest = self.blob_path(digest);
        tokio::fs::rename(upload_path, dest)
            .await
            .map_err(Error::Io)?;

        // All good.
        Ok(())
    }

    async fn cancel_upload(&self, _upload: Uuid) -> Result<(), Error> {
        todo!()
    }

    async fn get_manifest(&self, _location: &ImageLocation) -> Result<Option<Vec<u8>>, Error> {
        todo!()
    }

    async fn put_manifest(
        &self,
        location: &ImageLocation,
        reference: &Reference,
        manifest: &ImageManifest,
    ) -> Result<(), Error> {
        // TODO: Validate all blobs are completely uploaded.

        let dest = self.manifest_path(&location, &reference);
        let parent = dest.parent().expect("must have parent");

        if !parent.exists() {
            tokio::fs::create_dir_all(parent).await.map_err(Error::Io)?;
        }

        let contents =
            serde_json::to_string_pretty(manifest).expect("manifest should always be serializable");

        tokio::fs::write(dest, contents).await.map_err(Error::Io)
    }
}
