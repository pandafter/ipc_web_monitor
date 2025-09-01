use tokio::{io::AsyncReadExt, net::UnixListener};
use chrono::Utc;

use crate::state::{SharedTx, SharedValue};

pub async fn run_ipc(tx: SharedTx, latest_value: SharedValue) {
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
                        let mut lv = latest_value.lock().await;
                        *lv = msg.clone();
                    }
                    let msg_ts = if !msg.contains("recv_ts:") {
                        format!("{}|recv_ts:{}", msg, Utc::now().timestamp_micros())
                    } else { msg.clone() };
                    let _ = tx.send(msg_ts);
                }
            }
        }
    }
}
