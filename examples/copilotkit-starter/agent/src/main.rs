use awaken_examples::starter_backend::{
    StarterBackendArgs, StarterBackendConfig, serve_starter_backend,
};
use clap::Parser;

#[tokio::main]
async fn main() {
    let args = StarterBackendArgs::parse();
    serve_starter_backend(
        args,
        StarterBackendConfig::new("copilotkit-starter-agent", false),
    )
    .await;
}
