use carve_agent::{AgentDefinition, AgentPlugin, PermissionPlugin, Tool};
use clap::Parser;
use std::sync::Arc;
use uncarve_examples::travel::tools::*;

#[tokio::main]
async fn main() {
    let args = uncarve_examples::Args::parse();

    let agent_def = AgentDefinition {
        id: "travel".into(),
        model: "deepseek-chat".into(),
        system_prompt: concat!(
            "You are a travel planning assistant. Help users plan trips by adding, ",
            "updating, and searching for places of interest. Use the provided tools ",
            "to manage trips and find destinations.\n\n",
            "When the user asks to plan a trip, create it with add_trips, then ",
            "search_for_places to find interesting locations. Always select the ",
            "active trip with select_trip after creating it."
        ).into(),
        max_rounds: 10,
        parallel_tools: true,
        ..Default::default()
    };

    let tools: Vec<Arc<dyn Tool>> = vec![
        Arc::new(AddTripTool),
        Arc::new(UpdateTripTool),
        Arc::new(DeleteTripTool),
        Arc::new(SelectTripTool),
        Arc::new(SearchPlacesTool),
    ];

    let plugins: Vec<(String, Arc<dyn AgentPlugin>)> = vec![
        ("permission".into(), Arc::new(PermissionPlugin)),
    ];

    uncarve_examples::serve(args, agent_def, tools, plugins).await;
}
