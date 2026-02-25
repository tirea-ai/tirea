//! Stream transcoder trait for protocol bridging.

use std::marker::PhantomData;

/// Stream transcoder: maps an input stream to an output stream.
///
/// Stateful, supports 1:N mapping and stream lifecycle hooks.
/// Used for both directions (recv and send) of a protocol endpoint.
pub trait Transcoder: Send {
    /// Input item type consumed by this transcoder.
    type Input: Send + 'static;
    /// Output item type produced by this transcoder.
    type Output: Send + 'static;

    /// Events emitted before the input stream starts.
    fn prologue(&mut self) -> Vec<Self::Output> {
        Vec::new()
    }

    /// Map one input item to zero or more output items.
    fn transcode(&mut self, item: &Self::Input) -> Vec<Self::Output>;

    /// Events emitted after the input stream ends.
    fn epilogue(&mut self) -> Vec<Self::Output> {
        Vec::new()
    }
}

/// Pass-through transcoder (no transformation).
pub struct Identity<T>(PhantomData<T>);

impl<T> Default for Identity<T> {
    fn default() -> Self {
        Self(PhantomData)
    }
}

impl<T: Clone + Send + 'static> Transcoder for Identity<T> {
    type Input = T;
    type Output = T;

    fn transcode(&mut self, item: &Self::Input) -> Vec<Self::Output> {
        vec![item.clone()]
    }
}
