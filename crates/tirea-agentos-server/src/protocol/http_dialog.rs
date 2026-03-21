use std::convert::Infallible;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use async_trait::async_trait;
use axum::response::Response;
use bytes::Bytes;
use serde::Serialize;
use tirea_agentos::contracts::{AgentEvent, RunRequest, ToolCallDecision};
use tirea_agentos::runtime::{AgentOsRunError, ForwardedDecision, ResolvedRun};
use tirea_contract::Transcoder;
use tokio::sync::{broadcast, mpsc};

use crate::service::{
    forward_dialog_decisions_by_thread, start_http_dialog_run, ApiError, AppState, PreparedHttpRun,
};
use crate::transport::http_run::{wire_http_sse_relay, HttpSseRelayConfig};
use crate::transport::http_sse::sse_body_stream;

type RelayDoneFuture = Pin<Box<dyn Future<Output = ()> + Send>>;
pub type RelayDoneCallback =
    Box<dyn FnOnce(mpsc::Sender<Bytes>) -> RelayDoneFuture + Send + 'static>;
pub type ErrorFormatter = Arc<dyn Fn(String) -> Bytes + Send + Sync + 'static>;

pub struct DialogRelayPlan<E> {
    pub encoder: E,
    pub fanout: Option<broadcast::Sender<Bytes>>,
    pub resumable_downstream: bool,
    pub protocol_label: &'static str,
    pub on_relay_done: RelayDoneCallback,
    pub error_formatter: ErrorFormatter,
}

#[async_trait]
pub trait DialogRunAdapter {
    type Request: Clone + Send + Sync;
    type Encoder: Transcoder<Input = AgentEvent> + 'static;

    fn validate(req: &Self::Request) -> Result<(), ApiError>;
    fn thread_id(req: &Self::Request) -> &str;
    fn has_user_input(req: &Self::Request) -> bool;
    fn suspension_decisions(req: &Self::Request) -> Vec<ToolCallDecision>;
    fn frontend_run_id(_req: &Self::Request) -> Option<&str> {
        None
    }

    fn apply_extensions(resolved: &mut ResolvedRun, req: &Self::Request);
    fn into_run_request(req: Self::Request, agent_id: String) -> RunRequest;

    async fn build_relay_plan(
        st: &AppState,
        prepared: &PreparedHttpRun,
        req: &Self::Request,
    ) -> Result<DialogRelayPlan<Self::Encoder>, ApiError>;

    fn into_response<S>(stream: S) -> Response
    where
        S: futures::Stream<Item = Result<Bytes, Infallible>> + Send + 'static;

    fn forwarded_decision_response(
        forwarded: ForwardedDecision,
        frontend_run_id: Option<&str>,
    ) -> Response;
}

pub async fn handle_dialog_run<A>(
    st: AppState,
    agent_id: String,
    req: A::Request,
) -> Result<Response, ApiError>
where
    A: DialogRunAdapter,
    <A::Encoder as Transcoder>::Output: Serialize + Send + 'static,
{
    A::validate(&req)?;
    let frontend_run_id = A::frontend_run_id(&req).map(str::to_string);
    let suspension_decisions = A::suspension_decisions(&req);

    if let Some(forwarded) = forward_dialog_decisions_by_thread(
        &st.os,
        &agent_id,
        A::thread_id(&req),
        A::has_user_input(&req),
        frontend_run_id.as_deref(),
        &suspension_decisions,
    )
    .await?
    {
        return Ok(A::forwarded_decision_response(
            forwarded,
            frontend_run_id.as_deref(),
        ));
    }

    let mut resolved = st.os.resolve(&agent_id).map_err(AgentOsRunError::from)?;
    A::apply_extensions(&mut resolved, &req);
    let run_request = A::into_run_request(req.clone(), agent_id.clone());
    let prepared = start_http_dialog_run(&st.os, resolved, run_request, &agent_id).await?;
    let relay_plan = A::build_relay_plan(&st, &prepared, &req).await?;

    let starter = prepared
        .starter
        .lock()
        .unwrap()
        .take()
        .expect("starter already consumed");
    let ingress_rx = prepared
        .ingress_rx
        .lock()
        .unwrap()
        .take()
        .expect("ingress_rx already consumed");
    let sse_rx = wire_http_sse_relay(
        starter,
        relay_plan.encoder,
        ingress_rx,
        HttpSseRelayConfig {
            thread_id: prepared.thread_id,
            fanout: relay_plan.fanout,
            resumable_downstream: relay_plan.resumable_downstream,
            protocol_label: relay_plan.protocol_label,
            on_relay_done: relay_plan.on_relay_done,
            error_formatter: {
                let fmt = relay_plan.error_formatter;
                move |msg| fmt(msg)
            },
        },
    );

    Ok(A::into_response(sse_body_stream(sse_rx)))
}
