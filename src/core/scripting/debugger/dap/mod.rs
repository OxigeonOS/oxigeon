//! Debug Adapter Protocol server.
//!
//! Hand-rolled over `serde_json` rather than pulling in a DAP crate: the hard
//! part here is not the message shapes but marshalling requests onto a `!Send`
//! VM thread that is blocked inside a Lua hook, and no crate's threading model
//! fits that. Telnet, GMCP and MCCP2 are all hand-written in this codebase too.

pub mod codec;
pub mod session;

use std::net::SocketAddr;

use tokio::net::TcpListener;

use super::state::SharedDebugState;

/// Bind the DAP listener and serve clients until the process ends.
///
/// Returns the actually-bound address so callers (and tests, which bind port 0)
/// know where it landed.
pub async fn serve(
    bind: &str,
    port: u16,
    st: SharedDebugState,
) -> std::io::Result<SocketAddr> {
    let listener = TcpListener::bind((bind, port)).await?;
    let addr = listener.local_addr()?;

    tokio::spawn(async move {
        loop {
            match listener.accept().await {
                Ok((stream, peer)) => {
                    // One client at a time: the pause loop has a single request
                    // channel, and two editors driving one VM is meaningless.
                    if st.clients.load(std::sync::atomic::Ordering::Relaxed) > 0 {
                        tracing::warn!("debugger: rejecting {peer}, a client is already attached");
                        continue;
                    }
                    tracing::info!("debugger: client attached from {peer}");
                    let st2 = st.clone();
                    tokio::spawn(async move {
                        if let Err(e) = session::run(stream, st2.clone()).await {
                            tracing::warn!("debugger: client session ended: {e}");
                        }
                        session::detach(&st2);
                        tracing::info!("debugger: client detached");
                    });
                }
                Err(e) => {
                    tracing::error!("debugger: accept failed: {e}");
                    return;
                }
            }
        }
    });

    Ok(addr)
}
