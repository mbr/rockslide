use std::collections::HashMap;
use std::fs;
use std::net::Ipv4Addr;
use std::path::{Component, PathBuf};
use std::str::FromStr;
use std::{net::SocketAddr, path::Path, sync::Arc};

use crate::podman::podman_is_remote;
use crate::{
    podman::Podman,
    registry::{storage::ImageLocation, ManifestReference, Reference, RegistryHooks},
    reverse_proxy::ReverseProxy,
};

use anyhow::Context;
use axum::async_trait;
use axum::body::Body;
use axum::http::header::CONTENT_TYPE;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use sec::Secret;
use serde::{Deserialize, Deserializer, Serialize};
use tracing::{debug, error, info};

macro_rules! try_quiet {
    ($ex:expr, $msg:expr) => {
        match $ex {
            Ok(v) => v,
            Err(err) => {
                error!(%err, $msg);
                return;
            }
        }
    };
}

pub(crate) struct ContainerOrchestrator {
    podman: Podman,
    reverse_proxy: Arc<ReverseProxy>,
    local_addr: SocketAddr,
    registry_credentials: (String, Secret<String>),
    configs_dir: PathBuf,
    volumes_dir: PathBuf,
}

#[derive(Clone, Debug)]
pub(crate) struct PublishedContainer {
    host_addr: SocketAddr,
    manifest_reference: ManifestReference,
    config: Arc<RuntimeConfig>,
}

impl PublishedContainer {
    pub(crate) fn manifest_reference(&self) -> &ManifestReference {
        &self.manifest_reference
    }

    pub(crate) fn host_addr(&self) -> SocketAddr {
        self.host_addr
    }

    pub(crate) fn config(&self) -> &Arc<RuntimeConfig> {
        &self.config
    }
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub(crate) struct RuntimeConfig {
    #[serde(default)]
    pub(crate) http: Http,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub(crate) struct Http {
    #[serde(default)]
    pub(crate) access: Option<HashMap<String, Secret<String>>>,
}

impl IntoResponse for RuntimeConfig {
    fn into_response(self) -> axum::response::Response {
        toml::to_string_pretty(&self)
            .ok()
            .and_then(|config_toml| {
                Response::builder()
                    .status(StatusCode::OK)
                    .header(CONTENT_TYPE, "application/toml")
                    .body(Body::from(config_toml))
                    .ok()
            })
            .unwrap_or_else(|| StatusCode::INTERNAL_SERVER_ERROR.into_response())
    }
}

impl ContainerOrchestrator {
    pub(crate) fn new<P: AsRef<Path>, Q: AsRef<Path>>(
        podman_path: P,
        reverse_proxy: Arc<ReverseProxy>,
        local_addr: SocketAddr,
        registry_credentials: (String, Secret<String>),
        runtime_dir: Q,
    ) -> anyhow::Result<Self> {
        let podman = Podman::new(podman_path, podman_is_remote());

        let configs_dir = runtime_dir
            .as_ref()
            .canonicalize()
            .context("could not canonicalize runtime config dir")?
            .join("configs");

        if !configs_dir.exists() {
            fs::create_dir(&configs_dir).context("could not create config dir")?;
        }

        let volumes_dir = runtime_dir
            .as_ref()
            .canonicalize()
            .context("could not canonicalize runtime volumes dir")?
            .join("volumes");

        if !volumes_dir.exists() {
            fs::create_dir(&volumes_dir).context("could not create volumes dir")?;
        }

        Ok(Self {
            podman,
            reverse_proxy,
            local_addr,
            registry_credentials,
            configs_dir,
            volumes_dir,
        })
    }

    fn config_path(&self, manifest_reference: &ManifestReference) -> PathBuf {
        manifest_reference.namespaced_dir(&self.configs_dir)
    }

    pub(crate) async fn load_config(
        &self,
        manifest_reference: &ManifestReference,
    ) -> anyhow::Result<RuntimeConfig> {
        let config_path = self.config_path(manifest_reference);

        if !config_path.exists() {
            return Ok(Default::default());
        }

        let raw = tokio::fs::read_to_string(config_path)
            .await
            .context("could not read config")?;

        toml::from_str(&raw).context("could not parse configuration")
    }

    pub(crate) async fn save_config(
        &self,
        manifest_reference: &ManifestReference,
        config: &RuntimeConfig,
    ) -> anyhow::Result<RuntimeConfig> {
        let config_path = self.config_path(manifest_reference);
        let parent_dir = config_path
            .parent()
            .context("could not determine parent path")?;

        if !parent_dir.exists() {
            tokio::fs::create_dir_all(parent_dir)
                .await
                .context("could not create parent path")?;
        }

        let toml = toml::to_string_pretty(config).context("could not serialize new config")?;

        // TODO: Do atomic replace.
        tokio::fs::write(config_path, toml)
            .await
            .context("failed to write new toml config")?;

        // Read back to verify.
        self.load_config(manifest_reference).await
    }

    async fn fetch_managed_containers(&self, all: bool) -> anyhow::Result<Vec<PublishedContainer>> {
        debug!("refreshing running containers");

        let value = self.podman.ps(all).await?;
        let all_containers: Vec<ContainerJson> = serde_json::from_value(value)?;

        debug!(?all_containers, "fetched containers");

        let mut rv = Vec::new();
        for container in all_containers {
            // TODO: Just log error instead of returning.
            if let Some(pc) = self.load_managed_container(container).await? {
                rv.push(pc);
            }
        }
        Ok(rv)
    }

    async fn load_managed_container(
        &self,
        container_json: ContainerJson,
    ) -> anyhow::Result<Option<PublishedContainer>> {
        let manifest_reference = if let Some(val) = container_json.manifest_reference() {
            val
        } else {
            return Ok(None);
        };

        let port_mapping = if let Some(val) = container_json.active_published_port() {
            val
        } else {
            return Ok(None);
        };

        let config = Arc::new(self.load_config(&manifest_reference).await?);

        Ok(Some(PublishedContainer {
            host_addr: port_mapping
                .get_host_listening_addr()
                .context("could not get host listening address")?,
            manifest_reference,
            config,
        }))
    }

    pub(crate) async fn updated_published_set(&self) {
        let running: Vec<_> = try_quiet!(
            self.fetch_managed_containers(false).await,
            "could not fetch running containers"
        );

        info!(?running, "updating running container set");
        self.reverse_proxy
            .update_containers(running.into_iter())
            .await;
    }

    async fn synchronize_container_state(&self, manifest_reference: &ManifestReference) {
        // TODO: Make configurable?
        let production_tag = "prod";

        if matches!(manifest_reference.reference(), Reference::Tag(tag) if tag == production_tag) {
            let image_json_raw = try_quiet!(
                self.podman
                    .inspect("image", &manifest_reference.to_string())
                    .await,
                "failed to fetch image information via inspect"
            );
            let image_json: Vec<ImageJson> = try_quiet!(
                serde_json::from_value(image_json_raw),
                "failed to deserialize image information"
            );
            let volumes = try_quiet!(image_json.get(0).ok_or(""), "no information via inspect")
                .config
                .volume_iter();

            let location = manifest_reference.location();
            let name = format!("rockslide-{}-{}", location.repository(), location.image());

            info!(%name, "removing (potentially nonexistant) container");
            try_quiet!(
                self.podman.rm(&name, true).await,
                "failed to remove container"
            );

            let image_url = format!(
                "{}/{}/{}:{}",
                self.local_addr,
                location.repository(),
                location.image(),
                production_tag
            );

            info!(%name, "loggging in");
            try_quiet!(
                self.podman
                    .login(
                        &self.registry_credentials.0,
                        self.registry_credentials.1.as_str(),
                        self.local_addr.to_string().as_ref(),
                        false
                    )
                    .await,
                "failed to login to local registry"
            );

            // We always pull the container to ensure we have the latest version.
            info!(%name, "pulling container");
            try_quiet!(
                self.podman.pull(&image_url).await,
                "failed to pull container"
            );

            // Prepare volumes.
            let volume_base = manifest_reference.namespaced_dir(&self.volumes_dir);

            let mut podman_run = self.podman.run(&image_url);

            for vol_desc in volumes {
                let host_path = volume_base.join(&vol_desc);

                let mut container_path = PathBuf::from("/");
                container_path.push(vol_desc.as_ref());

                if !host_path.exists() {
                    try_quiet!(
                        tokio::fs::create_dir_all(&host_path).await,
                        "could not create volume path"
                    );
                }

                podman_run.bind_volume(host_path, container_path);
            }

            info!(%name, "starting container");
            try_quiet!(
                podman_run
                    .rm()
                    .rmi()
                    .name(name)
                    .tls_verify(false)
                    .publish("127.0.0.1::8000")
                    .env("PORT", "8000")
                    .execute()
                    .await,
                "failed to launch container"
            );

            info!(?manifest_reference, "new production image running");
        }
    }

    pub(crate) async fn synchronize_all(&self) -> anyhow::Result<()> {
        for container in self.fetch_managed_containers(true).await? {
            self.synchronize_container_state(container.manifest_reference())
                .await;
        }

        Ok(())
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
#[allow(dead_code)]
struct ContainerJson {
    id: String,
    image: String,
    names: Vec<String>,
    #[serde(deserialize_with = "nullable_array")]
    ports: Vec<PortMapping>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct ImageJson {
    config: ImageConfigJson,
}

// See: https://github.com/opencontainers/image-spec/blob/main/config.md
#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct ImageConfigJson {
    #[serde(default)]
    volumes: HashMap<PathBuf, EmptyGoStruct>,
}

#[derive(Debug)]
struct VolumeDesc(PathBuf);

impl VolumeDesc {
    fn from_path<P: AsRef<Path>>(path: P) -> Option<VolumeDesc> {
        let mut path = path.as_ref();
        if !path.is_relative() {
            path = path.strip_prefix("/").ok()?;
        }

        let mut parts = PathBuf::new();
        for component in path.components() {
            match component {
                Component::Prefix(_)
                | Component::RootDir
                | Component::CurDir
                | Component::ParentDir => {
                    // These are all illegal.
                    return None;
                }
                Component::Normal(os_str) => parts.push(os_str),
            }
        }

        Some(VolumeDesc(parts))
    }
}

impl AsRef<Path> for VolumeDesc {
    #[inline(always)]
    fn as_ref(&self) -> &Path {
        self.0.as_ref()
    }
}

impl ImageConfigJson {
    fn volume_iter(&self) -> Vec<VolumeDesc> {
        self.volumes
            .keys()
            .filter_map(VolumeDesc::from_path)
            .collect()
    }
}

#[derive(Debug)]
struct EmptyGoStruct;

impl Serialize for EmptyGoStruct {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        HashMap::<(), ()>::new().serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for EmptyGoStruct {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let deserialized: HashMap<(), ()> = Deserialize::deserialize(deserializer)?;
        if !deserialized.is_empty() {
            return Err(serde::de::Error::custom("should be empty string"));
        }
        Ok(EmptyGoStruct)
    }
}

impl ContainerJson {
    fn image_location(&self) -> Option<ImageLocation> {
        const PREFIX: &str = "rockslide-";

        for name in &self.names {
            if let Some(subname) = name.strip_prefix(PREFIX) {
                if let Some((left, right)) = subname.split_once('-') {
                    return Some(ImageLocation::new(left.to_owned(), right.to_owned()));
                }
            }
        }

        None
    }

    fn image_tag(&self) -> Option<Reference> {
        let idx = self.image.rfind(':')?;

        // TODO: Handle Reference::Digest here.
        Some(Reference::Tag(self.image[idx..].to_owned()))
    }

    fn manifest_reference(&self) -> Option<ManifestReference> {
        Some(ManifestReference::new(
            self.image_location()?,
            self.image_tag()?,
        ))
    }

    fn active_published_port(&self) -> Option<&PortMapping> {
        self.ports.get(0)
    }
}

#[async_trait]
impl RegistryHooks for Arc<ContainerOrchestrator> {
    async fn on_manifest_uploaded(&self, manifest_reference: &ManifestReference) {
        self.synchronize_container_state(manifest_reference).await;

        self.updated_published_set().await;
    }
}

fn nullable_array<'de, D, T>(deserializer: D) -> Result<Vec<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    let opt: Option<Vec<T>> = Deserialize::deserialize(deserializer)?;

    Ok(opt.unwrap_or_default())
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct PortMapping {
    host_ip: String,
    container_port: u16,
    host_port: u16,
    range: u16,
    protocol: String,
}

impl PortMapping {
    fn get_host_listening_addr(&self) -> Option<SocketAddr> {
        let ip = Ipv4Addr::from_str(&self.host_ip).ok()?;

        Some((ip, self.host_port).into())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use sec::Secret;

    use crate::container_orchestrator::Http;

    use super::RuntimeConfig;

    #[test]
    fn can_parse_sample_configs() {
        let example = r#"
            [http]
            access = { someuser = "somepw" }
            "#;

        let parsed: RuntimeConfig = toml::from_str(example).expect("should parse");

        let mut pw_map = HashMap::new();
        pw_map.insert("someuser".to_owned(), Secret::new("somepw".to_owned()));
        assert_eq!(
            parsed,
            RuntimeConfig {
                http: Http {
                    access: Some(pw_map)
                }
            }
        )
    }
}
