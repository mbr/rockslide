use std::collections::HashMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize)]
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

#[derive(Debug, Deserialize, Serialize)]
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
