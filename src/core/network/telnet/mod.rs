pub mod constants;
pub mod parser;
pub mod option;
pub mod codec;
pub mod connection;

pub use constants::*;
pub use parser::{TelnetParser, TelnetEvent};
pub use option::{OptionNegotiator, NegotiationCommand, QState};
pub use codec::TelnetCodec;
pub use connection::{TelnetConnection, ConnectionId, ClientCapabilities};

use std::net::SocketAddr;
use tokio::net::TcpListener;
use crate::config::driver_config::TelnetServerConfig;
use crate::error::{OxigeonError, Result};

/// TCP listener that accepts Telnet connections.
pub struct TelnetListener {
    config: TelnetServerConfig,
    listener: Option<TcpListener>,
}

impl TelnetListener {
    pub fn new(config: TelnetServerConfig) -> Self {
        TelnetListener {
            config,
            listener: None,
        }
    }

    pub async fn start(&mut self) -> Result<()> {
        let addr = format!("{}:{}", self.config.bind, self.config.port);
        self.listener = Some(TcpListener::bind(&addr).await?);
        tracing::info!("Telnet server listening on {}", addr);
        Ok(())
    }

    pub async fn accept(&mut self) -> Result<(TelnetConnection, tokio::net::tcp::OwnedReadHalf, SocketAddr)> {
        let listener = self.listener.as_ref()
            .ok_or_else(|| OxigeonError::Internal("Listener not started".into()))?;
        let (stream, addr) = listener.accept().await?;
        let (reader, writer) = stream.into_split();
        let conn = TelnetConnection::new(writer, addr);
        Ok((conn, reader, addr))
    }

    pub fn name(&self) -> &str { "Telnet" }

    pub fn addr(&self) -> String {
        format!("{}:{}", self.config.bind, self.config.port)
    }

    pub fn is_started(&self) -> bool {
        self.listener.is_some()
    }
}
