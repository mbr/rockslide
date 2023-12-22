mod podman;
mod registry;

use std::{borrow::Cow, env, path::Path};

use podman::Podman;
use registry::{DockerRegistry, ManifestReference, Reference, RegistryHooks};
use tower_http::trace::TraceLayer;
use tracing::{error, info};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

struct PodmanHook {
    podman: Podman,
}

impl PodmanHook {
    fn new<P: AsRef<Path>>(podman_path: P) -> Self {
        let podman = Podman::new(podman_path);
        Self { podman }
    }
}

impl RegistryHooks for PodmanHook {
    fn on_manifest_uploaded(&self, manifest_reference: &ManifestReference) {
        // TODO: Make configurable?
        let production_tag = "prod";

        if matches!(manifest_reference.reference(), Reference::Tag(tag) if tag == production_tag) {
            let location = manifest_reference.location();
            let name = format!("rockslide-{}-{}", location.repository(), location.image());

            info!(%name, "starting container");

            // TODO: Remove old image.
            // TODO: -p 127.0.0.1:5555:8000
            // TODO: Determine automatically.
            let local_registry_url = "127.0.0.1:3000";
            let image_url = format!(
                "{}/{}/{}:{}",
                local_registry_url,
                location.repository(),
                location.image(),
                production_tag
            );

            match self
                .podman
                .run(&image_url)
                .rm()
                .rmi()
                .name(name)
                .tls_verify(false)
                .execute()
            {
                Ok(_) => {
                    // All good.
                }
                Err(err) => {
                    error!(%err, "failed to launch container")
                }
            }
        } else {
            info!(?manifest_reference, "new production image uploaded");
        }
    }
}

#[tokio::main]
async fn main() {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                // axum logs rejections from built-in extractors with the `axum::rejection`
                // target, at `TRACE` level. `axum::rejection=trace` enables showing those events
                "rockslide=debug,tower_http=debug,axum::rejection=trace".into()
            }),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let podman_path = env::var("ROCKSLIDE_PODMAN_PATH")
        .map(Cow::Owned)
        .unwrap_or(Cow::Borrowed("podman"));
    let hooks = PodmanHook::new(podman_path.as_ref());
    let registry = DockerRegistry::new("./rockslide-storage", hooks);

    let app = registry.make_router().layer(TraceLayer::new_for_http());

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
