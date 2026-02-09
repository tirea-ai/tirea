use async_trait::async_trait;
use carve_agent::{
    AgentDefinition, AgentEvent, AgentOs, Message, Session, Tool, ToolDescriptor, ToolError,
    ToolResult,
};
use futures::StreamExt;
use serde::Deserialize;
use serde_json::{json, Value};
use std::time::Duration;

struct OpenMeteoWeatherTool {
    http: reqwest::Client,
}

impl OpenMeteoWeatherTool {
    fn new() -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(15))
            .build()
            .expect("reqwest client build");
        Self { http }
    }
}

#[derive(Debug, Deserialize)]
struct GeoResponse {
    #[serde(default)]
    results: Vec<GeoResult>,
}

#[derive(Debug, Deserialize)]
struct GeoResult {
    latitude: f64,
    longitude: f64,
    name: String,
    #[serde(default)]
    country: Option<String>,
    #[serde(default)]
    admin1: Option<String>,
    #[serde(default)]
    timezone: Option<String>,
}

#[async_trait]
impl Tool for OpenMeteoWeatherTool {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor::new(
            "get_weather",
            "Get Weather",
            "Get current weather for a city using Open-Meteo (real network call)",
        )
        .with_parameters(json!({
            "type": "object",
            "properties": {
                "city": {
                    "type": "string",
                    "description": "City name (e.g., 'San Francisco')"
                }
            },
            "required": ["city"]
        }))
    }

    async fn execute(
        &self,
        args: Value,
        _ctx: &carve_agent::Context<'_>,
    ) -> Result<ToolResult, ToolError> {
        let city = args
            .get("city")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidArguments("Missing 'city'".to_string()))?
            .trim();
        if city.is_empty() {
            return Err(ToolError::InvalidArguments("Empty 'city'".to_string()));
        }

        let geo_url = reqwest::Url::parse_with_params(
            "https://geocoding-api.open-meteo.com/v1/search",
            [
                ("name", city),
                ("count", "1"),
                ("language", "en"),
                ("format", "json"),
            ],
        )
        .map_err(|e| ToolError::ExecutionFailed(format!("bad geocoding url: {}", e)))?;

        let geo: GeoResponse = self
            .http
            .get(geo_url)
            .send()
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("geocoding request failed: {}", e)))?
            .error_for_status()
            .map_err(|e| ToolError::ExecutionFailed(format!("geocoding http error: {}", e)))?
            .json()
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("geocoding json parse failed: {}", e)))?;

        let r = geo
            .results
            .into_iter()
            .next()
            .ok_or_else(|| ToolError::NotFound(format!("city not found: {}", city)))?;

        let current = "temperature_2m,relative_humidity_2m,weather_code,wind_speed_10m";
        let timezone = "auto";
        let latitude = r.latitude.to_string();
        let longitude = r.longitude.to_string();
        let forecast_url = reqwest::Url::parse_with_params(
            "https://api.open-meteo.com/v1/forecast",
            [
                ("latitude", latitude.as_str()),
                ("longitude", longitude.as_str()),
                ("current", current),
                ("timezone", timezone),
            ],
        )
        .map_err(|e| ToolError::ExecutionFailed(format!("bad forecast url: {}", e)))?;

        let forecast: Value = self
            .http
            .get(forecast_url)
            .send()
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("forecast request failed: {}", e)))?
            .error_for_status()
            .map_err(|e| ToolError::ExecutionFailed(format!("forecast http error: {}", e)))?
            .json()
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("forecast json parse failed: {}", e)))?;

        Ok(ToolResult::success(
            "get_weather",
            json!({
                "query": { "city": city },
                "location": {
                    "name": r.name,
                    "admin1": r.admin1,
                    "country": r.country,
                    "timezone": r.timezone,
                    "latitude": r.latitude,
                    "longitude": r.longitude
                },
                "current": forecast.get("current").cloned().unwrap_or(Value::Null),
                "units": forecast.get("current_units").cloned().unwrap_or(Value::Null),
            }),
        ))
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn deepseek_real_weather_agent_smoke() {
    if std::env::var("DEEPSEEK_API_KEY").is_err() {
        panic!(
            "DEEPSEEK_API_KEY not set. If it's in ~/.bashrc, run: source ~/.bashrc (or export it) before running ignored tests."
        );
    }

    let os = AgentOs::builder()
        .with_tools(std::collections::HashMap::from([(
            "get_weather".to_string(),
            std::sync::Arc::new(OpenMeteoWeatherTool::new()) as std::sync::Arc<dyn Tool>,
        )]))
        .with_agent(
            "weather",
            AgentDefinition::new("deepseek-chat").with_system_prompt(
                "You are a weather assistant.\n\
Rules:\n\
- You MUST call the tool `get_weather` exactly once before answering.\n\
- Use the tool result as the source of truth.\n\
- Answer in 2-4 short sentences.\n",
            ),
        )
        .build()
        .unwrap();

    let session = Session::new("real-weather-smoke")
        .with_message(Message::user("What's the current weather in San Francisco? Use the tool."));

    let stream = os.run_stream("weather", session).unwrap();
    tokio::pin!(stream);

    let mut saw_weather_ready = false;
    let mut saw_weather_done = false;
    let mut text = String::new();

    let deadline = Duration::from_secs(60);
    let result = tokio::time::timeout(deadline, async {
        while let Some(ev) = stream.next().await {
            match ev {
                AgentEvent::ToolCallReady { name, .. } if name == "get_weather" => {
                    saw_weather_ready = true;
                }
                AgentEvent::ToolCallDone { result, .. }
                    if result.tool_name == "get_weather" && result.is_success() =>
                {
                    saw_weather_done = true;
                }
                AgentEvent::TextDelta { delta } => text.push_str(&delta),
                AgentEvent::RunFinish { .. } => break,
                AgentEvent::Error { message } => panic!("agent error: {}", message),
                _ => {}
            }
        }
    })
    .await;

    assert!(result.is_ok(), "test timed out after {:?}", deadline);
    assert!(saw_weather_ready, "model did not call get_weather");
    assert!(saw_weather_done, "get_weather never succeeded");
    assert!(!text.trim().is_empty(), "no assistant output produced");
}
