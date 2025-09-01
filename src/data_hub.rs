use warp::Filter;
use std::sync::{Arc, atomic::{AtomicUsize, Ordering}};
use tokio::sync::{broadcast, Mutex};
use tokio::io::AsyncReadExt;
use tokio::net::UnixListener;
use futures::{SinkExt, StreamExt};
use warp::ws::{Message, WebSocket};
use chrono::Utc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let (tx, _) = broadcast::channel::<String>(2000);
    let tx = Arc::new(tx);
    let latest_value = Arc::new(Mutex::new(String::new()));
    let clients_count = Arc::new(AtomicUsize::new(0));

    // ruta WS
    let ws_route = warp::path("ws")
        .and(warp::ws())
        .map({
            let tx = tx.clone();
            let latest = latest_value.clone();
            let count = clients_count.clone();
            move |ws: warp::ws::Ws| {
                let tx = tx.clone();
                let latest = latest.clone();
                let count = count.clone();
                ws.on_upgrade(move |socket| client_connection(socket, tx, latest, count))
            }
        });

    // archivos estáticos
    let routes = ws_route.or(warp::fs::dir("web/"));

    // IPC listener
    let tx_ipc = tx.clone();
    let latest_ipc = latest_value.clone();
    tokio::spawn(async move {
        let path = "/tmp/ipc_socket";
        let _ = std::fs::remove_file(path);
        let listener = match UnixListener::bind(path) {
            Ok(l) => l,
            Err(e) => { eprintln!("No se pudo bindear IPC socket: {}", e); return; }
        };
        println!("IPC listener listo en {}", path);

        loop {
            if let Ok((mut stream, _)) = listener.accept().await {
                let mut buf = vec![0u8; 8192];
                if let Ok(n) = stream.read(&mut buf).await {
                    if n > 0 {
                        let msg = String::from_utf8_lossy(&buf[..n]).trim_end().to_string();
                        {
                            let mut lv = latest_ipc.lock().await;
                            *lv = msg.clone();
                        }
                        let msg_ts = if !msg.contains("recv_ts:") {
                            format!("{}|recv_ts:{}", msg, Utc::now().timestamp_micros())
                        } else { msg.clone() };
                        let _ = tx_ipc.send(msg_ts);
                    }
                }
            }
        }
    });

    println!("Servidor corriendo en 0.0.0.0:5050");
    warp::serve(routes).run(([0,0,0,0], 5050)).await;

    Ok(())
}

async fn client_connection(
    ws: WebSocket,
    tx: Arc<broadcast::Sender<String>>,
    latest_value: Arc<Mutex<String>>,
    clients_count: Arc<AtomicUsize>
) {
    let count = clients_count.fetch_add(1, Ordering::SeqCst) + 1;
    let _ = tx.send(format!("USERS:{}", count));

    let (mut ws_tx, mut ws_rx) = ws.split();
    let mut rx = tx.subscribe();

    // enviar INIT al conectar
    let latest = latest_value.lock().await.clone();
    if !latest.is_empty() {
        let _ = ws_tx.send(Message::text(format!("INIT:{}", latest))).await;
    }

    // manejar lectura WS y broadcast
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
                        let _ = ws_tx.send(Message::text(&broadcast_msg)).await; // eco
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
