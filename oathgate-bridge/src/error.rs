//! bridge error type

use std::borrow::Cow;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("vhost: {0}")]
    VHost(#[from] oathgate_vhost::Error),

    #[error("net: {0}")]
    NetProtocol(#[from] oathgate_net::ProtocolError),

    #[error("i/o: {0}")]
    IO(#[from] std::io::Error),

    #[error("router: {0}")]
    Router(#[from] crate::router::RouterError),

    #[error("{0}")]
    Other(Cow<'static, str>),
}

impl From<String> for Error {
    fn from(msg: String) -> Self {
        Error::Other(Cow::Owned(msg))
    }
}

impl From<&'static str> for Error {
    fn from(msg: &'static str) -> Self {
        Error::Other(Cow::Borrowed(msg))
    }
}
