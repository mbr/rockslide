use std::{net::SocketAddr, path::PathBuf};

use axum::async_trait;
use constant_time_eq::constant_time_eq;
use sec::Secret;
use serde::Deserialize;

use crate::registry::{AuthProvider, UnverifiedCredentials};

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Config {
    #[serde(default)]
    pub rockslide: RockslideConfig,
    #[serde(default)]
    pub registry: RegistryConfig,
    #[serde(default)]
    pub containers: ContainerConfig,
    #[serde(default)]
    pub reverse_proxy: ReverseProxyConfig,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RockslideConfig {
    #[serde(default)]
    pub master_key: MasterKey,
    #[serde(default = "default_log")]
    pub log: String,
}

#[derive(Debug, Default)]
pub(crate) enum MasterKey {
    #[default]
    Locked,
    Key(Secret<String>),
}

#[async_trait]
impl AuthProvider for MasterKey {
    #[inline]
    async fn check_credentials(&self, creds: &UnverifiedCredentials) -> bool {
        match self {
            MasterKey::Locked => false,
            MasterKey::Key(sec_pw) => constant_time_eq(
                creds.password.reveal_str().as_bytes(),
                sec_pw.reveal_str().as_bytes(),
            ),
        }
    }

    /// Check if the given user has access to the given repo.
    #[inline]
    async fn has_access_to(&self, _username: &str, _namespace: &str, _image: &str) -> bool {
        true
    }
}

impl<'de> Deserialize<'de> for MasterKey {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        Ok(Option::<String>::deserialize(deserializer)?
            .map(Secret::new)
            .map(MasterKey::Key)
            .unwrap_or(MasterKey::Locked))
    }
}

fn default_log() -> String {
    "rockslide=info".to_owned()
}

impl Default for RockslideConfig {
    fn default() -> Self {
        Self {
            master_key: Default::default(),
            log: default_log(),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RegistryConfig {
    #[serde(default = "default_storage_path")]
    pub storage_path: PathBuf,
}

impl Default for RegistryConfig {
    fn default() -> Self {
        Self {
            storage_path: default_storage_path(),
        }
    }
}

fn default_storage_path() -> PathBuf {
    "./rockslide-storage".into()
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ContainerConfig {
    #[serde(default = "default_podman_path")]
    pub podman_path: PathBuf,
}

impl Default for ContainerConfig {
    fn default() -> Self {
        Self {
            podman_path: default_podman_path(),
        }
    }
}

fn default_podman_path() -> PathBuf {
    "podman".into()
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReverseProxyConfig {
    #[serde(default = "default_http_bind")]
    pub http_bind: SocketAddr,
}

impl Default for ReverseProxyConfig {
    fn default() -> Self {
        Self {
            http_bind: default_http_bind(),
        }
    }
}

fn default_http_bind() -> SocketAddr {
    ([127, 0, 0, 1], 3000).into()
}
