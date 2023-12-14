mod podman;
mod registry;

use podman::Podman;
use registry::{DockerRegistry, ManifestReference, Reference, RegistryHooks};
use tower_http::trace::TraceLayer;
use tracing::info;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

struct PodmanHook {
    // podman: Podman,
}

impl PodmanHook {
    fn new() -> Self {
        Self {}
    }
}

impl RegistryHooks for PodmanHook {
    fn on_manifest_uploaded(&self, manifest_reference: &ManifestReference) {
        if matches!(manifest_reference.reference(), Reference::Tag(tag) if tag == "prod") {
            let location = manifest_reference.location();
            let name = format!("rockslide-{}-{}", location.repository(), location.image());

            info!(%name, "starting container");
            // TODO: Start a podman container using image (cleanup previous one first).
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

    let hooks = PodmanHook::new();
    let registry = DockerRegistry::new("./rockslide-storage", hooks);

    let app = registry.make_router().layer(TraceLayer::new_for_http());

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
