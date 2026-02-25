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

use crate::runtime::control::{RuntimeInput, ToolCallDecision};

/// Wraps each [`ToolCallDecision`] into [`RuntimeInput::Decision`].
///
/// Used as the send-direction transcoder in `TranscoderEndpoint` so that
/// protocol layers continue to operate on `ToolCallDecision` while the
/// runtime endpoint receives [`RuntimeInput`].
pub struct DecisionTranscoder;

impl Transcoder for DecisionTranscoder {
    type Input = ToolCallDecision;
    type Output = RuntimeInput;

    fn transcode(&mut self, item: &Self::Input) -> Vec<Self::Output> {
        vec![RuntimeInput::Decision(item.clone())]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    #[test]
    fn identity_passthrough() {
        let mut t = Identity::<u32>::default();
        assert_eq!(t.transcode(&42), vec![42]);
    }

    #[test]
    fn decision_transcoder_wraps_to_runtime_input() {
        let mut t = DecisionTranscoder;
        let decision = ToolCallDecision::resume("tc1", Value::Null, 0);
        let output = t.transcode(&decision);
        assert_eq!(output.len(), 1);
        assert!(matches!(&output[0], RuntimeInput::Decision(d) if d.target_id == "tc1"));
    }
}
