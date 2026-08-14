pub mod telnet;
pub mod tls;
pub mod websocket;
pub use telnet::{TelnetConnection, ConnectionId};
pub use tls::MaybeTls;
