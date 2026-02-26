use async_trait::async_trait;
use serde_json::{json, Value};
use tirea_agentos::contracts::runtime::tool_call::{Tool, ToolCallContext, ToolDescriptor, ToolError, ToolResult};

use crate::state::StarterState;

pub struct GetWeatherTool;

#[async_trait]
impl Tool for GetWeatherTool {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor::new(
            "get_weather",
            "Get Weather",
            "Get weather details for a location.",
        )
        .with_parameters(json!({
            "type": "object",
            "properties": {
                "location": { "type": "string", "description": "City or location name" }
            },
            "required": ["location"]
        }))
    }

    async fn execute(
        &self,
        args: Value,
        _ctx: &ToolCallContext<'_>,
    ) -> Result<ToolResult, ToolError> {
        let location = args["location"]
            .as_str()
            .ok_or_else(|| ToolError::InvalidArguments("Missing 'location'".into()))?;

        Ok(ToolResult::success(
            "get_weather",
            json!({
                "location": location,
                "temperature_f": 70,
                "condition": "Sunny",
                "humidity_pct": 45
            }),
        ))
    }
}

pub struct GetStockPriceTool;

#[async_trait]
impl Tool for GetStockPriceTool {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor::new(
            "get_stock_price",
            "Get Stock Price",
            "Return a demo stock quote for the provided ticker symbol.",
        )
        .with_parameters(json!({
            "type": "object",
            "properties": {
                "symbol": { "type": "string", "description": "Ticker symbol, e.g. AAPL" }
            },
            "required": ["symbol"]
        }))
    }

    async fn execute(
        &self,
        args: Value,
        _ctx: &ToolCallContext<'_>,
    ) -> Result<ToolResult, ToolError> {
        let symbol = args["symbol"]
            .as_str()
            .ok_or_else(|| ToolError::InvalidArguments("Missing 'symbol'".into()))?
            .to_uppercase();

        let price = match symbol.as_str() {
            "AAPL" => 188.42_f64,
            "MSFT" => 421.10_f64,
            "NVDA" => 131.75_f64,
            _ => 99.99_f64,
        };

        Ok(ToolResult::success(
            "get_stock_price",
            json!({
                "symbol": symbol,
                "price_usd": price,
                "source": "starter-demo"
            }),
        ))
    }
}

pub struct AppendNoteTool;

#[async_trait]
impl Tool for AppendNoteTool {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor::new(
            "append_note",
            "Append Note",
            "Append a note into backend-persisted state.",
        )
        .with_parameters(json!({
            "type": "object",
            "properties": {
                "note": { "type": "string", "description": "Note text to append" }
            },
            "required": ["note"]
        }))
    }

    async fn execute(
        &self,
        args: Value,
        ctx: &ToolCallContext<'_>,
    ) -> Result<ToolResult, ToolError> {
        let note = args["note"]
            .as_str()
            .ok_or_else(|| ToolError::InvalidArguments("Missing 'note'".into()))?
            .trim();
        if note.is_empty() {
            return Err(ToolError::InvalidArguments(
                "Field 'note' cannot be empty".into(),
            ));
        }

        let state = ctx.state::<StarterState>("");
        let mut notes = state.notes().unwrap_or_default();
        notes.push(note.to_string());
        state
            .set_notes(notes.clone())
            .map_err(|err| ToolError::Internal(format!("failed to persist notes: {err}")))?;

        Ok(ToolResult::success(
            "append_note",
            json!({
                "added": note,
                "count": notes.len()
            }),
        ))
    }
}
