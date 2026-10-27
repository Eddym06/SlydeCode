use std::sync::Arc;

use axum::extract::Query;
use axum::extract::State;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use tokio::sync::broadcast;

use crate::state::{AppState, AuthQuery, ServerEvent, is_authorized, origin_allowed};

pub async fn upgrade(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(auth): Query<AuthQuery>,
) -> Response {
    if !origin_allowed(&state, &headers) {
        return (StatusCode::FORBIDDEN, "origin not allowed").into_response();
    }
    if !is_authorized(&state, &headers, &auth) {
        return (StatusCode::UNAUTHORIZED, "missing or invalid token").into_response();
    }
    ws.on_upgrade(move |socket| session(socket, state))
}

async fn session(mut socket: WebSocket, state: Arc<AppState>) {
    // Counted as an open window until this function returns, however it ends.
    let _window = state.presence.join();
    let mut events = state.events.subscribe();

    let initial = ServerEvent::StateChanged {
        view: state.view().await,
    };
    if send(&mut socket, &initial).await.is_err() {
        return;
    }

    loop {
        tokio::select! {
            incoming = socket.recv() => {
                match incoming {
                    None => break,
                    Some(Err(_)) => break,
                    Some(Ok(Message::Close(_))) => break,
                    Some(Ok(Message::Text(text))) => {
                        // Keepalive only. Mutations go through POST /api/command so
                        // every change is validated in one place.
                        if text.contains("\"ping\"")
                            && socket
                                .send(Message::Text("{\"type\":\"pong\"}".to_string().into()))
                                .await
                                .is_err()
                        {
                            break;
                        }
                    }
                    Some(Ok(_)) => {}
                }
            }
            event = events.recv() => {
                match event {
                    Ok(event) => {
                        if send(&mut socket, &event).await.is_err() {
                            break;
                        }
                    }
                    // A slow tab missed some frames; the next state event carries
                    // the full view, so dropping intermediate frames is safe.
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        }
    }
}

async fn send(socket: &mut WebSocket, event: &ServerEvent) -> Result<(), axum::Error> {
    let payload = serde_json::to_string(event).unwrap_or_else(|_| "{}".to_string());
    socket.send(Message::Text(payload.into())).await
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use futures_util::StreamExt;
    use slyde_core::{AspectRatio, PresentationState, Theme};

    use crate::{ServerConfig, start};

    async fn until(mut condition: impl FnMut() -> bool) -> bool {
        for _ in 0..100 {
            if condition() {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        condition()
    }

    #[tokio::test]
    async fn each_open_socket_counts_as_one_window_until_it_closes() {
        let state = PresentationState::new("Untitled", Theme::studio(), AspectRatio::R16x9);
        let ui = std::env::temp_dir();
        let server = start(ServerConfig::new(ui, state)).await.unwrap();
        let url = format!("ws://127.0.0.1:{}/ws?t={}", server.port, server.token);

        assert_eq!(server.state.presence.connected(), 0);
        let (mut editor, _) = tokio_tungstenite::connect_async(&url).await.unwrap();
        // The first frame is the full state, so the session is live once it arrives.
        editor.next().await.unwrap().unwrap();
        let (mut presenter, _) = tokio_tungstenite::connect_async(&url).await.unwrap();
        presenter.next().await.unwrap().unwrap();
        assert_eq!(server.state.presence.connected(), 2);

        drop(editor);
        assert!(until(|| server.state.presence.connected() == 1).await);
        drop(presenter);
        assert!(until(|| server.state.presence.connected() == 0).await);
        server.shutdown().await;
    }

    #[tokio::test]
    async fn a_rejected_handshake_is_not_a_window() {
        let state = PresentationState::new("Untitled", Theme::studio(), AspectRatio::R16x9);
        let server = start(ServerConfig::new(std::env::temp_dir(), state))
            .await
            .unwrap();
        let url = format!("ws://127.0.0.1:{}/ws?t=wrong", server.port);
        assert!(tokio_tungstenite::connect_async(&url).await.is_err());
        assert_eq!(server.state.presence.connected(), 0);
        server.shutdown().await;
    }
}
