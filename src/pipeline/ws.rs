//! Shared WebSocket connect helper with permissive TLS.
//!
//! All ingesters call `connect_ws` instead of `tokio_tungstenite::connect_async`
//! so that ISP transparent-proxy certificates (e.g. *.ioh.co.id on Indonesian
//! networks) do not block data collection.
//!
//! The data collected is public market data — TLS here provides transport
//! encryption only; cert identity is not security-critical for this use case.

#[cfg(feature = "ingest")]
use tokio_tungstenite::{
    tungstenite::{client::IntoClientRequest, Error},
    MaybeTlsStream, WebSocketStream,
};

#[cfg(feature = "ingest")]
type WsStream = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

/// Connect to a WebSocket URL (or pre-built `Request`) using native-TLS with
/// certificate verification disabled.  Returns the same stream type as
/// `tokio_tungstenite::connect_async` so existing `Ok((ws, _))` destructures
/// work unchanged.
#[cfg(feature = "ingest")]
pub async fn connect_ws<R: IntoClientRequest + Unpin>(
    request: R,
) -> Result<(WsStream, tokio_tungstenite::tungstenite::http::Response<Option<Vec<u8>>>), Error> {
    let tls = native_tls::TlsConnector::builder()
        .danger_accept_invalid_certs(true)
        .danger_accept_invalid_hostnames(true)
        .build()
        .expect("native_tls::TlsConnector build should not fail");

    let connector = tokio_tungstenite::Connector::NativeTls(tls);

    tokio_tungstenite::connect_async_tls_with_config(request, None, false, Some(connector)).await
}
