//! Models that use types from other modules.

use super::types::{Address, ContactInfo, Metadata};
use carve_state_derive::CarveViewModel;
use serde::{Deserialize, Serialize};

/// Person uses Address from another module
#[derive(Debug, Clone, Serialize, Deserialize, CarveViewModel)]
pub struct Person {
    pub name: String,
    pub age: u32,

    // Cross-module nested reference
    #[carve(nested)]
    pub address: Address,

    // Another cross-module nested reference
    #[carve(nested)]
    pub contact: ContactInfo,
}

/// Company uses Address and has optional Metadata
#[derive(Debug, Clone, Serialize, Deserialize, CarveViewModel)]
pub struct Company {
    pub name: String,

    // Cross-module nested in Option
    #[carve(nested)]
    pub headquarters: Option<Address>,

    // Cross-module nested in Vec
    #[carve(nested)]
    pub offices: Vec<Address>,

    // Cross-module nested
    #[carve(nested)]
    pub metadata: Option<Metadata>,
}

/// Event references Person (which itself references cross-module types)
#[derive(Debug, Clone, Serialize, Deserialize, CarveViewModel)]
pub struct Event {
    pub event_name: String,

    // Nested reference to a type that itself has nested cross-module references
    #[carve(nested)]
    pub organizer: Person,
}
