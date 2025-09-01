use warp::Filter;
use std::sync::{atomic::Ordering};
use futures::{SinkExt, StreamExt};
use warp::ws::{Message, WebSocket};

use crate::state::{SharedTx, SharedValue, SharedCount};

pub fn routes(
    tx: SharedTx,
    latest_value: SharedValue,
    clients_count: SharedCount,
) -> impl warp::Filter<Extract = impl warp::Reply, Error = warp::Rejection> + Clone {
    warp::path("ws")
        .and(warp::ws())
        .map(move |ws: warp::ws::Ws| {
            let tx = tx.clone();
            let latest = latest_value.clone();
            let count = clients_count.clone();
            ws.on_upgrade(move |socket| client_connection(socket, tx, latest, count))
        })
}

async fn client_connection(
    ws: WebSocket,
    tx: SharedTx,
    latest_value: SharedValue,
    clients_count: SharedCount,
) {
    let count = clients_count.fetch_add(1, Ordering::SeqCst) + 1;
    let _ = tx.send(format!("USERS:{}", count));

    let (mut ws_tx, mut ws_rx) = ws.split();
    let mut rx = tx.subscribe();

    // enviar INIT
    let latest = latest_value.lock().await.clone();
    if !latest.is_empty() {
        let _ = ws_tx.send(Message::text(format!("INIT:{}", latest))).await;
    }

    loop {
        tokio::select! {
            Some(msg) = ws_rx.next() => {
                match msg {
                    Ok(m) => if let Ok(txt) = m.to_str() {
                        let parts: Vec<&str> = txt.splitn(2,'|').collect();
                        let payload = if parts.len() == 2 { parts[1] } else { txt };
                        {
                            let mut lv = latest_value.lock().await;
                            *lv = payload.to_string();
                        }
                        let broadcast_msg = format!("{}|{}", parts.get(0).unwrap_or(&"0"), payload);
                        // eco
                        let _ = ws_tx.send(Message::text(&broadcast_msg)).await;
                        let _ = tx.send(broadcast_msg);
                    },
                    Err(e) => { eprintln!("WS error: {}", e); break; }
                }
            },
            Ok(broadcast_msg) = rx.recv() => {
                let _ = ws_tx.send(Message::text(broadcast_msg)).await;
            }
        }
    }

    let rem = clients_count.fetch_sub(1, Ordering::SeqCst).saturating_sub(1);
    let _ = tx.send(format!("USERS:{}", rem));
}
