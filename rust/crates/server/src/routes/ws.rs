use crate::error::AppError;
use crate::*;

pub(crate) async fn collab_ws(
    State(state): State<SharedState>,
    Path(schedule_id): Path<i64>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Response {
    // The HTTP auth middleware is skipped for /collab; do the checks here. We
    // deny the handshake (HTTP error) rather than accepting then closing — the
    // connection is refused either way (documented divergence from the Python
    // post-accept ws.close(1011/1008)).
    if state.db.is_none() {
        return AppError::new(StatusCode::SERVICE_UNAVAILABLE, "no database").into_response();
    }
    let token = bsession(&headers);
    if auth_member(&state, token.clone()).await.is_none() {
        return AppError::new(StatusCode::UNAUTHORIZED, "not authenticated").into_response();
    }
    // Admins edit; everyone else observes (their update frames are dropped).
    let is_admin = current_is_admin(&state, token).await;
    ws.on_upgrade(move |socket| handle_collab(state, schedule_id, is_admin, socket))
}

pub(crate) async fn handle_collab(
    state: SharedState,
    schedule_id: i64,
    is_admin: bool,
    mut socket: WebSocket,
) {
    use tokio::sync::broadcast::error::RecvError;

    // Lazily load persisted state, then join the room.
    let initial = with_db(&state, move |conn| {
        conn.and_then(|c| db::get_yjs_state(c, schedule_id).unwrap_or(None))
    })
    .await;
    let joined = state.collab.join(schedule_id, initial.as_deref());
    let conn_id = joined.conn_id;
    let doc = joined.doc;
    let tx = joined.tx;
    let mut rx = joined.rx;

    // Sync step 1: tell the client our state vector so they can catch up.
    let sv = collab::state_vector(&doc);
    let _ = socket
        .send(Message::Binary(collab::encode_sync_step1(&sv)))
        .await;

    loop {
        tokio::select! {
            incoming = socket.recv() => {
                match incoming {
                    Some(Ok(Message::Binary(raw))) => {
                        // Drop oversized frames before parsing/applying.
                        if raw.len() > state.opts.max_ws_message_bytes {
                            continue;
                        }
                        let Some((mtype, stype, payload)) = collab::parse_message(&raw) else {
                            continue;
                        };
                        if mtype == collab::MSG_SYNC {
                            if stype == collab::SYNC_STEP1 {
                                let diff = collab::diff_for(&doc, &payload);
                                let _ = socket
                                    .send(Message::Binary(collab::encode_sync_step2(&diff)))
                                    .await;
                            } else if stype == collab::SYNC_STEP2 || stype == collab::SYNC_UPDATE {
                                // Apply/persist/broadcast only for admins; a
                                // non-admin (Observer) frame is silently dropped.
                                if is_admin && collab::apply_update(&doc, &payload) {
                                    let state_bytes = collab::full_update(&doc);
                                    let _ = with_db(&state, move |conn| {
                                        if let Some(c) = conn {
                                            let _ = db::set_yjs_state(c, schedule_id, &state_bytes);
                                        }
                                    })
                                    .await;
                                    let _ = tx.send((conn_id, collab::encode_update_msg(&payload)));
                                }
                            }
                        } else if mtype == collab::MSG_AWARENESS {
                            let _ = tx.send((conn_id, raw));
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(_)) => {} // text/ping/pong — ignore
                    Some(Err(_)) => break,
                }
            }
            received = rx.recv() => {
                match received {
                    Ok((from, data)) => {
                        if from != conn_id
                            && socket.send(Message::Binary(data)).await.is_err()
                        {
                            break;
                        }
                    }
                    Err(RecvError::Lagged(_)) => {} // dropped some broadcasts; keep going
                    Err(RecvError::Closed) => break,
                }
            }
        }
    }

    drop(rx);
    state.collab.leave_if_empty(schedule_id);
}
