use axum::{
    body::Body,
    http::{header::LOCATION, Request, Response, StatusCode},
    response::{Html, IntoResponse},
    routing::{get, post},
    Router,
};
use tower_http::trace::TraceLayer;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

// https://github.com/distribution/distribution/blob/5cb406d511b7b9163bff9b6439072e4892e5ae3b/docs/spec/api.md

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

    // build our application with a route
    let app = Router::new()
        .route("/v2/", get(index_v2))
        .route("/v2/test/blobs/uploads/", post(upload_blob_test))
        .layer(TraceLayer::new_for_http());

    // run it with hyper on localhost:3000
    axum::Server::bind(&"0.0.0.0:3000".parse().unwrap())
        .serve(app.into_make_service())
        .await
        .unwrap();
}

async fn index_v2() -> Html<&'static str> {
    // Login will just store credentials locally, not do anything with them. This is all we need.
    Html("all systems operational")
}

async fn upload_blob_test(request: Request<Body>) -> Response<Body> {
    let mut resp = StatusCode::ACCEPTED.into_response();
    let location = format!("/v2/test/blobs/uploads/asdf123"); // TODO: should be uuid
    resp.headers_mut()
        .append(LOCATION, location.parse().unwrap());

    resp.map(|_| Default::default())
}
