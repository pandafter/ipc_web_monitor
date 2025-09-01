use warp::Filter;
use tokio;

use ipc_web_monitor::state::new_state;
use ipc_web_monitor::{ws_server, ipc_server};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let (tx, latest_value, clients_count) = new_state();

    // spawn IPC
    let tx_ipc = tx.clone();
    let latest_ipc = latest_value.clone();
    tokio::spawn(async move {
        ipc_server::run_ipc(tx_ipc, latest_ipc).await;
    });

    // WS + archivos estáticos
    let ws_route = ws_server::routes(tx.clone(), latest_value.clone(), clients_count.clone());
    let routes = ws_route.or(warp::fs::dir("web/"));

    println!("Servidor corriendo en 0.0.0.0:5050");
    warp::serve(routes).run(([0,0,0,0], 5050)).await;
    Ok(())
}
