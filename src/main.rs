use oxigeon::config::{load_driver_config, load_server_config};
use oxigeon::driver::Driver;

#[tokio::main]
async fn main() {
    // Load configurations
    let driver_cfg_path = std::env::var("DRIVER_CONFIG")
        .unwrap_or_else(|_| "config/driver.toml".to_string());
    let server_cfg_path = std::env::var("SERVER_CONFIG")
        .unwrap_or_else(|_| "config/server.toml".to_string());

    let driver_config = match load_driver_config(&driver_cfg_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Failed to load driver config: {}", e);
            std::process::exit(1);
        }
    };

    let server_config = match load_server_config(&server_cfg_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Failed to load server config: {}", e);
            std::process::exit(1);
        }
    };

    // Initialize logging
    let log_level = &driver_config.logging.level;
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(log_level))
        )
        .init();

    let version = env!("CARGO_PKG_VERSION");
    // Box interior is 39 chars wide. Center "Oxigeon v{ver}" within it.
    let title = format!("Oxigeon v{}", version);
    let pad_total = 39usize.saturating_sub(title.len());
    let pad_left = pad_total / 2;
    let pad_right = pad_total - pad_left;
    tracing::info!("╔═══════════════════════════════════════╗");
    tracing::info!("║{left}{title}{right}║",
        left  = " ".repeat(pad_left),
        title = title,
        right = " ".repeat(pad_right));
    tracing::info!("╚═══════════════════════════════════════╝");
    tracing::info!("Game: {}", server_config.game.name);

    // Build and run the driver
    let driver = match Driver::new(driver_config, server_config).await {
        Ok(d) => d,
        Err(e) => {
            tracing::error!("Failed to initialize driver: {}", e);
            std::process::exit(1);
        }
    };

    if let Err(e) = driver.run().await {
        tracing::error!("Driver error: {}", e);
        std::process::exit(1);
    }
}
