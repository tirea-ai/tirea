use async_nats::ConnectErrorKind;

#[derive(Debug, thiserror::Error)]
pub enum NatsProtocolError {
    #[error("nats connect error: {0}")]
    Connect(#[from] async_nats::error::Error<ConnectErrorKind>),

    #[error("nats subscribe error: {0}")]
    Subscribe(#[from] async_nats::SubscribeError),

    #[error("nats error: {0}")]
    Nats(#[from] async_nats::Error),

    #[error("bad request: {0}")]
    BadRequest(String),

    #[error("run error: {0}")]
    Run(String),
}
