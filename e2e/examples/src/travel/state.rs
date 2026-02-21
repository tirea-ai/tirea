use serde::{Deserialize, Serialize};
use tirea_state_derive::State;

/// A single trip in the travel planner.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Trip {
    pub id: String,
    pub name: String,
    pub destination: String,
    pub places: Vec<Place>,
}

/// A place of interest within a trip.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Place {
    pub id: String,
    pub name: String,
    pub address: String,
    pub lat: f64,
    pub lng: f64,
    pub description: String,
    pub category: String,
}

/// Search progress indicator sent during place searches.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SearchProgress {
    pub query: String,
    pub status: String,
    pub results_count: usize,
}

/// Root state for the travel example.
///
/// Corresponds to CopilotKit's `CopilotKitState` with travel-specific fields.
#[derive(Debug, Clone, Default, Serialize, Deserialize, State)]
pub struct TravelState {
    pub selected_trip_id: Option<String>,
    pub trips: Vec<Trip>,
    pub search_progress: Vec<SearchProgress>,
}
