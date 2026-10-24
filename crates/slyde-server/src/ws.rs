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
                        if text.contains("\"ping\"") {
                            if socket
                                .send(Message::Text("{\"type\":\"pong\"}".to_string().into()))
                                .await
                                .is_err()
                            {
                                break;
                            }
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
