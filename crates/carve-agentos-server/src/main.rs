use carve_agentos::contracts::storage::{AgentStateReader, AgentStateStore};
use carve_agentos::orchestrator::AgentDefinition;
use carve_agentos::orchestrator::{AgentOs, AgentOsBuilder, ModelDefinition};
use carve_agentos_server::http::{self, AppState};
use carve_thread_store_adapters::FileStore;
use clap::Parser;
use serde::Deserialize;
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Debug, Parser)]
#[command(name = "carve-agentos-server")]
struct Args {
    #[arg(long, env = "AGENTOS_HTTP_ADDR", default_value = "127.0.0.1:8080")]
    http_addr: String,

    #[arg(long, env = "AGENTOS_STORAGE_DIR", default_value = "./threads")]
    storage_dir: PathBuf,

    #[arg(long, env = "AGENTOS_CONFIG")]
    config: Option<PathBuf>,

    #[arg(long, env = "AGENTOS_NATS_URL")]
    nats_url: Option<String>,

    /// TensorZero gateway URL (e.g. http://localhost:4000/openai/v1/).
    /// When set, all model calls are routed through TensorZero for observability.
    #[arg(long, env = "TENSORZERO_URL")]
    tensorzero_url: Option<String>,
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

fn build_os(
    cfg: Option<Config>,
    tensorzero_url: Option<String>,
    write_store: Arc<dyn AgentStateStore>,
) -> AgentOs {
    let mut builder = AgentOsBuilder::new().with_agent_state_store(write_store);

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

    // When TensorZero URL is provided, register a custom provider and model mapping.
    if let Some(tz_url) = &tensorzero_url {
        let endpoint = tz_url.clone();
        let tz_client = genai::Client::builder()
            .with_service_target_resolver_fn(move |mut t: genai::ServiceTarget| {
                t.endpoint = genai::resolver::Endpoint::from_owned(&*endpoint);
                t.auth = genai::resolver::AuthData::from_single("tensorzero");
                Ok(t)
            })
            .build();
        builder = builder.with_provider("tz", tz_client);
        eprintln!("TensorZero provider registered at {tz_url}");
    }

    for a in agents {
        let mut def = AgentDefinition {
            id: a.id.clone(),
            system_prompt: a.system_prompt,
            ..Default::default()
        };
        if let Some(model) = &a.model {
            def.model = model.clone();
        }
        if let Some(max_rounds) = a.max_rounds {
            def.max_rounds = max_rounds;
        }
        if let Some(parallel) = a.parallel_tools {
            def.parallel_tools = parallel;
        }

        // Map each agent's model through TensorZero when configured.
        if tensorzero_url.is_some() {
            if let Some(model) = &a.model {
                builder = builder.with_model(
                    model,
                    ModelDefinition::new("tz", "openai::tensorzero::function_name::agent_chat"),
                );
                eprintln!("  model '{model}' → TensorZero agent_chat");
            }
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

    let file_store = Arc::new(FileStore::new(args.storage_dir));
    let write_store: Arc<dyn AgentStateStore> = file_store.clone();
    let read_store: Arc<dyn AgentStateReader> = file_store.clone();
    let os = Arc::new(build_os(cfg, args.tensorzero_url, write_store));

    let app = http::router(AppState {
        os: os.clone(),
        read_store,
    });

    if let Some(nats_url) = args.nats_url.clone() {
        let os = os.clone();
        tokio::spawn(async move {
            let gateway =
                match carve_agentos_server::nats::NatsGateway::connect(os, &nats_url).await {
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
