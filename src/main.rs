mod config;
mod container_orchestrator;
pub(crate) mod podman;
pub(crate) mod registry;
mod reverse_proxy;

use std::{
    env, fs,
    net::{IpAddr, SocketAddr, ToSocketAddrs},
    sync::Arc,
};

use anyhow::Context;
use axum::Router;
use config::Config;
use gethostname::gethostname;
use registry::ContainerRegistry;
use reverse_proxy::ReverseProxy;
use tower_http::trace::TraceLayer;
use tracing::{debug, info};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use crate::{container_orchestrator::ContainerOrchestrator, podman::podman_is_remote};

fn load_config() -> anyhow::Result<Config> {
    match env::args().len() {
        0 | 1 => Ok(Default::default()),
        2 => {
            let arg = env::args().nth(1).expect("should have arg 1");
            let contents = fs::read_to_string(&arg)
                .context("could not read configuration file")
                .context(arg)?;
            let cfg = toml::from_str(&contents).context("failed to parse configuration")?;

            Ok(cfg)
        }
        _ => Err(anyhow::anyhow!(
            "expected at most one command arg, pointing to a config file"
        )),
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Parse configuration, if available, otherwise use a default.
    let cfg = load_config().context("could not load configuration")?;

    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| (&cfg.rockslide.log).into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    debug!(?cfg, "loaded configuration");

    let local_ip: IpAddr = if podman_is_remote() {
        info!("podman is remote, trying to guess IP address");
        let local_hostname = gethostname();
        let dummy_addr = (
            local_hostname
                .to_str()
                .ok_or_else(|| anyhow::anyhow!("local hostname is not valid UTF8"))?,
            12345,
        )
            .to_socket_addrs()
            .ok()
            .and_then(|addrs| addrs.into_iter().next())
            .ok_or_else(|| anyhow::anyhow!("failed to resolve local hostname"))?;
        dummy_addr.ip()
    } else {
        [127, 0, 0, 1].into()
    };

    let local_addr = SocketAddr::from((local_ip, cfg.reverse_proxy.http_bind.port()));
    // TODO: Fix (see #34).
    let local_addr = SocketAddr::from(([127, 0, 0, 1], cfg.reverse_proxy.http_bind.port()));
    info!(%local_addr, "guessing local registry address");

    let reverse_proxy = ReverseProxy::new();

    let credentials = (
        "rockslide-podman".to_owned(),
        cfg.rockslide.master_key.as_secret_string(),
    );
    let orchestrator = Arc::new(ContainerOrchestrator::new(
        &cfg.containers.podman_path,
        reverse_proxy.clone(),
        local_addr,
        credentials,
        &cfg.registry.storage_path,
    )?);
    reverse_proxy.set_orchestrator(orchestrator.clone());

    // TODO: Probably should not fail if synchronization fails.
    orchestrator.synchronize_all().await?;
    orchestrator.updated_published_set().await;

    let registry = ContainerRegistry::new(
        &cfg.registry.storage_path,
        orchestrator,
        cfg.rockslide.master_key,
    )?;

    let app = Router::new()
        .merge(registry.make_router())
        .merge(reverse_proxy.make_router())
        .layer(TraceLayer::new_for_http());

    let listener = tokio::net::TcpListener::bind(cfg.reverse_proxy.http_bind)
        .await
        .context("failed to bind listener")?;
    axum::serve(listener, app)
        .await
        .context("http server exited with error")?;

    Ok(())
}
