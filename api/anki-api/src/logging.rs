use tracing_subscriber::EnvFilter;

pub fn init_json_logging() {
    let env_filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info,anki_api=debug"));

    let _ = tracing_subscriber::fmt()
        .with_env_filter(env_filter)
        .json()
        .with_target(true)
        .try_init();
}

pub fn init_stderr_logging() {
    let env_filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info,anki_api=debug"));

    let _ = tracing_subscriber::fmt()
        .with_env_filter(env_filter)
        .with_target(true)
        .try_init();
}
