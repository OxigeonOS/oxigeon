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

    // Initialize logging.
    //
    // `logging.file` used to be parsed and then ignored: only `logging.level`
    // was read, so setting a path produced no file and no warning — the config
    // key looked like it worked and silently did nothing. It writes now, and a
    // path that cannot be opened says so on stderr rather than failing quietly,
    // because the whole point of the setting is that you are not watching
    // stdout.
    //
    // The guard has to outlive `main`, or the non-blocking writer's worker
    // thread is dropped and the last lines never reach the file.
    let log_level = &driver_config.logging.level;
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(log_level));

    let _log_guard = match driver_config.logging.file.as_deref() {
        Some(path) if !path.is_empty() => {
            let path = std::path::Path::new(path);
            if let Some(dir) = path.parent() {
                if !dir.as_os_str().is_empty() {
                    if let Err(e) = std::fs::create_dir_all(dir) {
                        eprintln!("Cannot create log directory {}: {e}", dir.display());
                    }
                }
            }
            match std::fs::OpenOptions::new().create(true).append(true).open(path) {
                Ok(file) => {
                    // ANSI off for the file: escape codes in a log someone
                    // will `grep` are noise, and the terminal is not reading it.
                    let (writer, guard) = tracing_appender::non_blocking(file);
                    tracing_subscriber::fmt()
                        .with_env_filter(filter)
                        .with_writer(writer)
                        .with_ansi(false)
                        .init();
                    eprintln!("Logging to {}", path.display());
                    Some(guard)
                }
                Err(e) => {
                    eprintln!("Cannot open log file {}: {e} — logging to stdout", path.display());
                    tracing_subscriber::fmt().with_env_filter(filter).init();
                    None
                }
            }
        }
        _ => {
            tracing_subscriber::fmt().with_env_filter(filter).init();
            None
        }
    };

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
