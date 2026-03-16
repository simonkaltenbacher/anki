use std::env;
use std::path::PathBuf;

use anki_api::config::FileConfig;
use anki_api::config::RuntimeOverrides;
use anki_api::config::ServerConfig;
use anki_api::grpc;
use anki_api::logging;
use anki_api::store;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    logging::init_json_logging();
    let file_config = FileConfig::load_default().map_err(|err| {
        tracing::error!(error = %err, "failed to load anki api file config");
        err
    })?;
    let config =
        ServerConfig::resolve(RuntimeOverrides::default(), file_config).map_err(|err| {
            tracing::error!(error = %err, "failed to resolve anki api server config");
            err
        })?;

    let env_collection_path = env::var("ANKI_PUBLIC_API_COLLECTION_DB_PATH")
        .ok()
        .filter(|value| !value.is_empty());
    let collection_path = env_collection_path
        .as_ref()
        .map(PathBuf::from)
        .unwrap_or_else(default_collection_path);
    if env_collection_path.is_some() {
        tracing::info!("using collection sqlite path from ANKI_PUBLIC_API_COLLECTION_DB_PATH");
    } else {
        tracing::warn!(
            "ANKI_PUBLIC_API_COLLECTION_DB_PATH not set; using ephemeral temp collection for api server"
        );
    }
    let store = store::initialize_store(collection_path)?;

    if let Err(err) = grpc::serve_with_store(config, store).await {
        tracing::error!(error = %err, "anki api grpc server terminated with error");
        return Err(err);
    }

    Ok(())
}

fn default_collection_path() -> PathBuf {
    let mut path = env::temp_dir();
    path.push(format!("anki-api-{}.anki2", std::process::id()));
    path
}
