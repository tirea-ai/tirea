use carve_agent::{AgentDefinition, AgentOs, AgentOsBuilder, FileStorage};
use carve_agentos_server::http::{self, AppState};
use clap::Parser;
use serde::Deserialize;
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Debug, Parser)]
#[command(name = "carve-agentos-server")]
struct Args {
    #[arg(long, env = "AGENTOS_HTTP_ADDR", default_value = "127.0.0.1:8080")]
    http_addr: String,

    #[arg(long, env = "AGENTOS_STORAGE_DIR", default_value = "./sessions")]
    storage_dir: PathBuf,

    #[arg(long, env = "AGENTOS_CONFIG")]
    config: Option<PathBuf>,

    #[arg(long, env = "AGENTOS_NATS_URL")]
    nats_url: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Config {
    agents: Vec<AgentConfigFile>,
}

#[derive(Debug, Deserialize)]
struct AgentConfigFile {
    id: String,
    model: Option<String>,
    #[serde(default)]
    system_prompt: String,
    max_rounds: Option<usize>,
    parallel_tools: Option<bool>,
}

fn build_os(cfg: Option<Config>) -> AgentOs {
    let mut builder = AgentOsBuilder::new();

    let agents = match cfg {
        Some(c) => c.agents,
        None => vec![AgentConfigFile {
            id: "default".to_string(),
            model: None,
            system_prompt: String::new(),
            max_rounds: None,
            parallel_tools: None,
        }],
    };

    if agents.is_empty() {
        eprintln!("config has no agents");
        std::process::exit(2);
    }

    for a in agents {
        let mut def = AgentDefinition {
            id: a.id.clone(),
            system_prompt: a.system_prompt,
            ..Default::default()
        };
        if let Some(model) = a.model {
            def.model = model;
        }
        if let Some(max_rounds) = a.max_rounds {
            def.max_rounds = max_rounds;
        }
        if let Some(parallel) = a.parallel_tools {
            def.parallel_tools = parallel;
        }
        builder = builder.with_agent(a.id, def);
    }

    match builder.build() {
        Ok(os) => os,
        Err(e) => {
            eprintln!("failed to build AgentOs: {e}");
            std::process::exit(2);
        }
    }
}

#[tokio::main]
async fn main() {
    let args = Args::parse();

    let cfg = match args.config.as_ref() {
        Some(path) => {
            let raw = match std::fs::read_to_string(path) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("failed to read config {}: {e}", path.display());
                    std::process::exit(2);
                }
            };
            let parsed = match serde_json::from_str::<Config>(&raw) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("failed to parse config (JSON): {e}");
                    std::process::exit(2);
                }
            };
            Some(parsed)
        }
        None => None,
    };

    let os = Arc::new(build_os(cfg));
    let storage = Arc::new(FileStorage::new(args.storage_dir));

    let app = http::router(AppState {
        os: os.clone(),
        storage: storage.clone(),
    });

    if let Some(nats_url) = args.nats_url.clone() {
        let os = os.clone();
        let storage = storage.clone();
        tokio::spawn(async move {
            let gateway = match carve_agentos_server::nats::NatsGateway::connect(
                os, storage, &nats_url,
            )
            .await
            {
                Ok(g) => g,
                Err(e) => {
                    eprintln!("nats connect failed: {}", e);
                    return;
                }
            };
            if let Err(e) = gateway.serve().await {
                eprintln!("nats gateway stopped: {}", e);
            }
        });
    }

    let listener = tokio::net::TcpListener::bind(&args.http_addr)
        .await
        .expect("failed to bind http listener");

    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await
        .expect("http server crashed");
}
