//! Compile-fail test: flatten attribute should be rejected

use carve_state_derive::CarveViewModel;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Inner {
    pub value: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, CarveViewModel)]
pub struct Outer {
    pub name: String,

    // P1-7: This should cause a compile error
    #[carve(flatten)]
    pub inner: Inner,
}

fn main() {}
