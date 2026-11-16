use odis_signer::config::Config;
use odis_signer::server::build_router;
use tokio::net::TcpListener;

#[tokio::main]
async fn main() {
    let _ = dotenvy::dotenv();

    let config = match Config::from_env() {
        Ok(config) => config,
        Err(error) => {
            // Logging starts only once the config names its format.
            eprintln!("failed to load config: {error}");
            std::process::exit(1);
        }
    };
    odis_signer::logging::init(config.log_format);
    let port = config.server_port;

    let app = match build_router(config).await {
        Ok(app) => app,
        Err(error) => {
            tracing::error!("failed to build router: {error}");
            std::process::exit(1);
        }
    };

    let listener = TcpListener::bind(("0.0.0.0", port))
        .await
        .expect("failed to bind port");

    tracing::info!("ODIS signer listening on port {port}");
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .expect("server error");
}

async fn shutdown_signal() {
    let ctrl_c = tokio::signal::ctrl_c();
    let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .expect("failed to register SIGTERM handler");

    tokio::select! {
        _ = ctrl_c => tracing::info!("received SIGINT, shutting down"),
        _ = sigterm.recv() => tracing::info!("received SIGTERM, shutting down"),
    }
}
