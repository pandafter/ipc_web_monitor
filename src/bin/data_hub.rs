// src/bin/data_hub.rs
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
    // broadcast channel para enviar mensajes a todos los clientes conectados
    let (tx, _rx) = broadcast::channel::<String>(2000);
    let tx = Arc::new(tx);

    // estado compartido: último estímulo (para nuevos clientes)
    let latest_value = Arc::new(Mutex::new(String::new()));

    // contador de clientes
    let clients_count = Arc::new(AtomicUsize::new(0));

    // filtros para pasar valores a la ruta WS
    let tx_filter = {
        let tx = tx.clone();
        warp::any().map(move || tx.clone())
    };
    let latest_filter = {
        let lv = latest_value.clone();
        warp::any().map(move || lv.clone())
    };
    let count_filter = {
        let c = clients_count.clone();
        warp::any().map(move || c.clone())
    };

    // ruta websocket
    let ws_route = warp::path("ws")
        .and(warp::ws())
        .and(tx_filter)
        .and(latest_filter)
        .and(count_filter)
        .map(|ws: warp::ws::Ws, tx, latest_value, clients_count| {
            ws.on_upgrade(move |socket| client_connection(socket, tx, latest_value, clients_count))
        });

    // archivos estáticos en web/
    let static_files = warp::fs::dir("web/");

    let routes = ws_route.or(static_files);

    // lanzar listener IPC (Unix socket) para que los sensores inyecten mensajes
    let tx_for_ipc = tx.clone();
    let latest_for_ipc = latest_value.clone();
    tokio::spawn(async move {
        let socket_path = "/tmp/ipc_socket";
        let _ = std::fs::remove_file(socket_path);
        match UnixListener::bind(socket_path) {
            Ok(listener) => {
                println!("IPC listener listo en {}", socket_path);
                loop {
                    match listener.accept().await {
                        Ok((mut stream, _addr)) => {
                            let mut buf = vec![0u8; 8192];
                            if let Ok(n) = stream.read(&mut buf).await {
                                if n > 0 {
                                    let mut msg = String::from_utf8_lossy(&buf[..n]).to_string();
                                    msg = msg.trim_end().to_string();
                                    // registramos como último valor
                                    {
                                        let mut lv = latest_for_ipc.lock().await;
                                        *lv = msg.clone();
                                    }
                                    // si no contiene recv_ts lo añadimos (solo para logging)
                                    let msg_with_recv = if !msg.contains("recv_ts:") {
                                        let recv_ts = Utc::now().timestamp_micros();
                                        format!("{}|recv_ts:{}", msg, recv_ts)
                                    } else {
                                        msg.clone()
                                    };
                                    let _ = tx_for_ipc.send(msg_with_recv);
                                }
                            }
                        }
                        Err(e) => {
                            eprintln!("Error aceptando conexión IPC: {}", e);
                            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                        }
                    }
                }
            }
            Err(e) => {
                eprintln!("No se pudo bindear IPC socket: {}", e);
            }
        }
    });

    println!("Servidor HTTP+WS corriendo en 0.0.0.0:8080");
    warp::serve(routes).run(([0,0,0,0], 8080)).await;
    Ok(())
}

/// Maneja cada cliente WS:
/// - incrementa contador y anuncia USERS:<n>
/// - subscribe al broadcast (rebroadcaster)
/// - recibe mensajes del cliente; guarda latest_value y rebroadcast
async fn client_connection(
    ws: WebSocket,
    tx: Arc<broadcast::Sender<String>>,
    latest_value: Arc<Mutex<String>>,
    clients_count: Arc<AtomicUsize>,
) {
    // aumentar contador y anunciar
    let new_count = clients_count.fetch_add(1, Ordering::SeqCst) + 1;
    let _ = tx.send(format!("USERS:{}", new_count));

    // separar sink/stream
    let (ws_tx, mut ws_rx) = ws.split();
    // envolver sink para poder usarlo desde tareas
    let ws_tx = Arc::new(Mutex::new(ws_tx));

    // subscribir al broadcast
    let mut rx = tx.subscribe();

    // Cuando se conecta un cliente le enviamos INIT con el último valor si existe
    let latest = latest_value.lock().await.clone();
    if !latest.is_empty() {
        let init_msg = format!("INIT:{}", latest);
        let mut lock = ws_tx.lock().await;
        let _ = lock.send(Message::text(init_msg)).await;
    }


    // tarea: reenviar broadcast a este cliente
    let ws_tx_clone = ws_tx.clone();
    tokio::spawn(async move {
        while let Ok(msg) = rx.recv().await {
            let mut lock = ws_tx_clone.lock().await;
            // ignorar errores de envío (cliente puede desconectar)
            let _ = lock.send(Message::text(msg)).await;
        }
    });

    // escuchar mensajes del cliente
    while let Some(result) = ws_rx.next().await {
        match result {
            Ok(msg) => {
                if let Ok(text) = msg.to_str() {
                    // Esperamos que el cliente envíe: "<send_ts_us>|<payload>"
                    // send_ts_us es performance.now()*1000 (µs) del cliente que envía.
                    let parts: Vec<&str> = text.splitn(2, '|').collect();
                    if parts.len() >= 2 {
                        let send_ts_str = parts[0];
                        let payload = parts[1].to_string();

                        // guardar como último valor compartido
                        {
                            let mut lv = latest_value.lock().await;
                            *lv = payload.clone();
                        }

                        // anotar recv_ts del servidor (para logging)
                        let server_recv_ts = Utc::now().timestamp_micros();

                        // construir mensaje para broadcast:
                        // formato: "<send_ts_us>|<payload>|recv_ts:<server_recv_ts>"
                        let broadcast_msg = format!("{}|{}|recv_ts:{}", send_ts_str, payload, server_recv_ts);

                        // enviar eco al cliente (rápido)
                        {
                            let mut lock = ws_tx.lock().await;
                            let _ = lock.send(Message::text(broadcast_msg.clone())).await;
                        }

                        // enviar a todos los demás (broadcast)
                        let _ = tx.send(broadcast_msg);
                    } else {
                        // mensaje sin timestamp: tratamos como payload simple
                        let payload = text.to_string();
                        {
                            let mut lv = latest_value.lock().await;
                            *lv = payload.clone();
                        }
                        let server_recv_ts = Utc::now().timestamp_micros();
                        let broadcast_msg = format!("0|{}|recv_ts:{}", payload, server_recv_ts);
                        let mut lock = ws_tx.lock().await;
                        let _ = lock.send(Message::text(broadcast_msg.clone())).await;
                        let _ = tx.send(broadcast_msg);
                    }
                }
            }
            Err(e) => {
                eprintln!("WS receive error: {}", e);
                break;
            }
        }
    }

    // cliente desconectado -> decrementar y anunciar
    let remaining = clients_count.fetch_sub(1, Ordering::SeqCst).saturating_sub(1);
    let _ = tx.send(format!("USERS:{}", remaining));
}
