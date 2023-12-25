use std::path::PathBuf;

use sec::Secret;
use serde::Deserialize;

#[derive(Debug, Default, Deserialize)]
pub(crate) struct Config {
    #[serde(default)]
    pub rockslide: RockslideConfig,
    #[serde(default)]
    pub registry: RegistryConfig,
    #[serde(default)]
    pub containers: ContainerConfig,
}

#[derive(Debug, Deserialize)]
pub(crate) struct RockslideConfig {
    #[serde(default)]
    pub master_key: MasterKey,
    #[serde(default = "default_log")]
    pub log: String,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) enum MasterKey {
    #[default]
    Locked,
    Key(Secret<String>),
}

fn default_log() -> String {
    // axum logs rejections from built-in extractors with the `axum::rejection` target, at `TRACE`
    // level. `axum::rejection=trace` enables showing those events
    "rockslide=debug,tower_http=debug,axum::rejection=trace".to_owned()
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

#[derive(Debug, Deserialize)]
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

fn default_storage_path() -> PathBuf {
    "./rockslide-storage".into()
}
