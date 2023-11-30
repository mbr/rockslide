use std::collections::HashMap;

use axum::{
    async_trait,
    extract::{rejection::StringRejection, FromRequest, Request},
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use thiserror::Error;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ContentDescriptor {
    media_type: String,
    digest: String, // TODO: Use digest type
    size: u64,
    urls: Option<Vec<String>>,
    annotations: Option<HashMap<String, String>>,
    data: Option<String>,
    artifact_type: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ImageManifest {
    schema_version: u32,

    media_type: String,
    annotations: Option<HashMap<String, String>>,
    artifact_type: Option<String>,

    config: ContentDescriptor,
    layers: Vec<ContentDescriptor>,
    subject: Option<ContentDescriptor>,
}

#[async_trait]
impl<S> FromRequest<S> for ImageManifest
where
    S: Send + Sync,
{
    /// If the extractor fails it'll use this "rejection" type. A rejection is
    /// a kind of error that can be converted into a response.
    type Rejection = JsonContentRejection;

    /// Perform the extraction.
    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        let body = String::from_request(req, state)
            .await
            .map_err(IntoResponse::into_response)?;
    }
}

#[derive(Debug, Error)]
pub(crate) enum JsonContentRejection {
    #[error("could retrieve body")]
    BodyFailure(StringRejection),
}

impl IntoResponse for JsonContentRejection {
    fn into_response(self) -> Response {
        todo!()
    }
}

#[cfg(test)]
mod tests {
    use super::ImageManifest;

    #[test]
    fn simple_example_schema_parse() {
        let raw = r#"{
            "schemaVersion": 2,
            "mediaType": "application/vnd.docker.distribution.manifest.v2+json",
            "config": {
               "mediaType": "application/vnd.docker.container.image.v1+json",
               "size": 2298,
               "digest": "sha256:e4c58958181a5925816faa528ce959e487632f4cfd192f8132f71b32df2744b4"
            },
            "layers": [
               {
                  "mediaType": "application/vnd.docker.image.rootfs.diff.tar.gzip",
                  "size": 30439111,
                  "digest": "sha256:43f89b94cd7df92a2f7e565b8fb1b7f502eff2cd225508cbd7ea2d36a9a3a601"
               }
            ]
        }"#;

        let manifest: ImageManifest = serde_json::from_str(raw).expect("could not parse manifest");
    }
}
