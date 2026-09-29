//! WebSocket connections: binary frames carry `ls-net` packets, one packet
//! per frame, in both directions. WebSocket has no unreliable channel, so
//! unreliable packets travel on the same ordered stream; the link's
//! backpressure drops queued unreliable frames when the socket falls behind.

use std::net::SocketAddr;

use futures_util::{SinkExt, StreamExt};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::watch;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::protocol::{CloseFrame, WebSocketConfig};
use tokio_tungstenite::tungstenite::Message as WsMessage;
use tokio_tungstenite::WebSocketStream;
use tracing::{debug, info};

use crate::endpoint::Hub;
use crate::link::CloseReason;
use crate::stats::inc;

/// Transport name in logs.
pub const TRANSPORT: &str = "websocket";

/// Largest message accepted from a client (inputs and pings are tiny; the
/// `Hello` carries a name of at most 32 bytes).
pub const MAX_CLIENT_MESSAGE: usize = 64 * 1024;

/// Server-side WebSocket settings.
pub fn server_config() -> WebSocketConfig {
    WebSocketConfig::default()
        .max_message_size(Some(MAX_CLIENT_MESSAGE))
        .max_frame_size(Some(MAX_CLIENT_MESSAGE))
}

/// Why a connection ended (for logs).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum End {
    ServerClosed,
    Overflow,
    PeerClosed,
    IdleTimeout,
    Error,
    Shutdown,
}

/// Runs one WebSocket connection (after the HTTP upgrade) to completion.
pub async fn serve<S>(
    mut ws: WebSocketStream<S>,
    remote: SocketAddr,
    hub: Hub,
    mut shutdown: watch::Receiver<bool>,
) where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let settings = *hub.settings();
    // 1. Handshake: the first binary frame must be a `Hello`.
    let first = tokio::time::timeout(settings.handshake_timeout, async {
        while let Some(msg) = ws.next().await {
            match msg {
                Ok(WsMessage::Binary(b)) => return Some(b.to_vec()),
                Ok(WsMessage::Ping(_) | WsMessage::Pong(_)) => continue,
                _ => return None,
            }
        }
        None
    })
    .await;
    let first = match first {
        Ok(Some(b)) => b,
        Ok(None) => {
            debug!(%remote, "websocket closed before Hello");
            return;
        }
        Err(_) => {
            inc(&hub.stats().timeouts);
            info!(%remote, "handshake timed out");
            let _ = ws
                .close(Some(close_frame(CloseCode::Policy, "handshake timeout")))
                .await;
            return;
        }
    };
    let peer = match hub.open(TRANSPORT, remote, &first, usize::MAX) {
        Ok(p) => p,
        Err(reject) => {
            info!(%remote, "handshake rejected");
            let _ = ws.send(WsMessage::Binary(reject.into())).await;
            let _ = ws
                .close(Some(close_frame(CloseCode::Normal, "rejected")))
                .await;
            return;
        }
    };
    let conn = peer.info().id;
    debug!(conn, %remote, "websocket handshake done");

    // 2. Pump until one side stops.
    let (mut sink, mut stream) = ws.split();
    let end = {
        let writer = async {
            while let Some(f) = peer.next_outgoing().await {
                if sink.feed(WsMessage::Binary(f.data.into())).await.is_err() {
                    return End::Error;
                }
                // Batch what is already queued into one flush.
                while let Some(f) = peer.try_next_outgoing() {
                    if sink.feed(WsMessage::Binary(f.data.into())).await.is_err() {
                        return End::Error;
                    }
                }
                if sink.flush().await.is_err() {
                    return End::Error;
                }
            }
            match peer.close_reason() {
                Some(CloseReason::Overflow) => End::Overflow,
                _ => End::ServerClosed,
            }
        };
        let reader = async {
            loop {
                match tokio::time::timeout(settings.idle_timeout, stream.next()).await {
                    Err(_) => return End::IdleTimeout,
                    Ok(None) | Ok(Some(Ok(WsMessage::Close(_)))) => return End::PeerClosed,
                    Ok(Some(Err(_))) => return End::Error,
                    Ok(Some(Ok(WsMessage::Binary(b)))) => {
                        if peer.push_incoming(b.to_vec()).await.is_err() {
                            // The sim closed the link; the writer finishes up.
                            std::future::pending::<()>().await;
                        }
                    }
                    Ok(Some(Ok(_))) => {} // ping/pong/text: ignored
                }
            }
        };
        let stop = async {
            let _ = shutdown.wait_for(|s| *s).await;
            // Give the sim thread time to queue `Disconnect` and close links.
            tokio::time::sleep(settings.shutdown_grace).await;
            End::Shutdown
        };
        tokio::select! {
            e = writer => e,
            e = reader => e,
            e = stop => e,
        }
    };
    let close = match end {
        End::ServerClosed => Some(close_frame(CloseCode::Normal, "closed by server")),
        End::Overflow => Some(close_frame(CloseCode::Again, "client too slow")),
        End::IdleTimeout => Some(close_frame(CloseCode::Policy, "idle timeout")),
        End::Shutdown => Some(close_frame(CloseCode::Away, "server shutting down")),
        End::PeerClosed | End::Error => None,
    };
    if end == End::IdleTimeout {
        inc(&hub.stats().timeouts);
    }
    info!(conn, %remote, reason = ?end, "websocket closed");
    // Dropping the peer link tells the sim thread the player is gone.
    drop(peer);
    if let Some(frame) = close {
        let _ = tokio::time::timeout(settings.shutdown_grace, async {
            let _ = sink.send(WsMessage::Close(Some(frame))).await;
            // Wait for the peer's close reply so the TCP stream ends cleanly.
            while let Some(Ok(m)) = stream.next().await {
                if matches!(m, WsMessage::Close(_)) {
                    break;
                }
            }
        })
        .await;
    }
}

fn close_frame(code: CloseCode, reason: &'static str) -> CloseFrame {
    CloseFrame {
        code,
        reason: reason.into(),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{mpsc, Arc};
    use std::time::Duration;

    use ls_net::{Hello, Message, Transport};
    use tokio::io::DuplexStream;
    use tokio_tungstenite::tungstenite::protocol::Role;

    use super::*;
    use crate::endpoint::ConnSettings;
    use crate::sim::SimCommand;
    use crate::stats::SharedStats;

    struct Setup {
        client: WebSocketStream<DuplexStream>,
        commands: mpsc::Receiver<SimCommand>,
        stats: Arc<SharedStats>,
        server: tokio::task::JoinHandle<()>,
        _shutdown: watch::Sender<bool>,
    }

    async fn setup(settings: ConnSettings) -> Setup {
        let (a, b) = tokio::io::duplex(1 << 16);
        let (tx, rx) = mpsc::channel();
        let stats = Arc::new(SharedStats::default());
        let hub = Hub::new(tx, stats.clone(), settings);
        let (stop_tx, stop_rx) = watch::channel(false);
        let server_ws = WebSocketStream::from_raw_socket(a, Role::Server, None).await;
        let remote = "127.0.0.1:9".parse().unwrap();
        let server = tokio::spawn(serve(server_ws, remote, hub, stop_rx));
        let client = WebSocketStream::from_raw_socket(b, Role::Client, None).await;
        Setup {
            client,
            commands: rx,
            stats,
            server,
            _shutdown: stop_tx,
        }
    }

    fn short() -> ConnSettings {
        ConnSettings {
            handshake_timeout: Duration::from_millis(150),
            idle_timeout: Duration::from_millis(150),
            shutdown_grace: Duration::from_millis(100),
            ..ConnSettings::default()
        }
    }

    async fn hello(client: &mut WebSocketStream<DuplexStream>) {
        let bytes = Message::Hello(Hello::new("pilot", None)).encode().unwrap();
        client.send(WsMessage::Binary(bytes.into())).await.unwrap();
    }

    fn join_link(commands: &mpsc::Receiver<SimCommand>) -> crate::link::SimLink {
        for _ in 0..200 {
            if let Ok(SimCommand::Join { hello, link }) = commands.try_recv() {
                assert_eq!(hello.name, "pilot");
                return link;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        panic!("no join");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn packets_flow_between_socket_and_sim_link() {
        let mut s = setup(ConnSettings::default()).await;
        hello(&mut s.client).await;
        let mut link = tokio::task::block_in_place(|| join_link(&s.commands));
        assert_eq!(link.info().transport, TRANSPORT);
        s.client
            .send(WsMessage::Binary(vec![1, 2, 3].into()))
            .await
            .unwrap();
        let mut got = None;
        for _ in 0..200 {
            if let Some(p) = link.recv().unwrap() {
                got = Some(p.data);
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert_eq!(got.unwrap(), [1, 2, 3]);
        link.send_unreliable(b"snapshot").unwrap();
        link.send_reliable(b"events").unwrap();
        for want in [&b"snapshot"[..], b"events"] {
            match s.client.next().await {
                Some(Ok(WsMessage::Binary(b))) => assert_eq!(&b[..], want),
                other => panic!("{other:?}"),
            }
        }
        // The sim closes the link: queued frames flush, then a close frame.
        link.send_reliable(b"bye").unwrap();
        link.close();
        match s.client.next().await {
            Some(Ok(WsMessage::Binary(b))) => assert_eq!(&b[..], b"bye"),
            other => panic!("{other:?}"),
        }
        assert!(matches!(
            s.client.next().await,
            Some(Ok(WsMessage::Close(_)))
        ));
        drop(s.client);
        s.server.await.unwrap();
        assert_eq!(
            s.stats
                .connections
                .load(std::sync::atomic::Ordering::Relaxed),
            1
        );
    }

    #[tokio::test]
    async fn bad_hello_gets_a_reject() {
        let mut s = setup(ConnSettings::default()).await;
        s.client
            .send(WsMessage::Binary(vec![9, 9, 9].into()))
            .await
            .unwrap();
        match s.client.next().await {
            Some(Ok(WsMessage::Binary(b))) => {
                assert!(matches!(Message::decode(&b).unwrap(), Message::Reject(_)));
            }
            other => panic!("{other:?}"),
        }
        s.server.await.unwrap();
        assert!(s.commands.try_recv().is_err(), "no join for a bad hello");
        assert_eq!(
            s.stats.rejected.load(std::sync::atomic::Ordering::Relaxed),
            1
        );
    }

    #[tokio::test]
    async fn silent_connection_times_out_before_hello() {
        let mut s = setup(short()).await;
        let end = tokio::time::timeout(Duration::from_secs(5), s.client.next())
            .await
            .unwrap();
        assert!(matches!(end, Some(Ok(WsMessage::Close(_)))), "{end:?}");
        drop(s.client);
        s.server.await.unwrap();
        assert_eq!(
            s.stats.timeouts.load(std::sync::atomic::Ordering::Relaxed),
            1
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn idle_player_times_out_and_the_sim_sees_it() {
        let mut s = setup(short()).await;
        hello(&mut s.client).await;
        let mut link = tokio::task::block_in_place(|| join_link(&s.commands));
        let end = tokio::time::timeout(Duration::from_secs(5), s.client.next())
            .await
            .unwrap();
        assert!(matches!(end, Some(Ok(WsMessage::Close(_)))), "{end:?}");
        drop(s.client);
        s.server.await.unwrap();
        assert_eq!(
            s.stats.timeouts.load(std::sync::atomic::Ordering::Relaxed),
            1
        );
        assert!(link.recv().is_err(), "sim side sees the closed link");
        assert!(!link.is_open());
    }
}
