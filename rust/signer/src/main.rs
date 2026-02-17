use odis_signer::config::Config;
use odis_signer::server::build_router;
use tokio::net::TcpListener;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let _ = dotenvy::dotenv();

    let config = Config::from_env().expect("failed to load config");
    let port = config.server_port;

    let app = build_router(config).await.expect("failed to build router");

    let listener = TcpListener::bind(("0.0.0.0", port))
        .await
        .expect("failed to bind port");

    tracing::info!("ODIS signer listening on port {port}");
    axum::serve(listener, app).await.expect("server error");
}
