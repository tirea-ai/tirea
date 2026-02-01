//! Shared types for cross-module testing.
//!
//! This module defines types that will be imported and used in other modules.

use carve_state_derive::CarveViewModel;
use serde::{Deserialize, Serialize};

/// Address type - defined in a separate module
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, CarveViewModel)]
pub struct Address {
    pub street: String,
    pub city: String,
    pub zip: String,
}

/// Contact information - another nested type
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, CarveViewModel)]
pub struct ContactInfo {
    pub email: String,
    pub phone: Option<String>,
}

/// Metadata - used for testing multiple imports
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, CarveViewModel)]
pub struct Metadata {
    pub created_at: String,
    pub updated_at: String,
}
