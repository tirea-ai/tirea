use clap::Parser;
use tirea_examples::starter_backend::{
    serve_starter_backend, StarterBackendArgs, StarterBackendConfig,
};

#[tokio::main]
async fn main() {
    let args = StarterBackendArgs::parse();
    serve_starter_backend(
        args,
        StarterBackendConfig::new("copilotkit-starter-agent", false),
    )
    .await;
}
