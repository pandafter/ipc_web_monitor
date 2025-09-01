use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use tokio::sync::{broadcast, Mutex};

pub type Tx = broadcast::Sender<String>;
pub type SharedTx = Arc<Tx>;
pub type SharedValue = Arc<Mutex<String>>;
pub type SharedCount = Arc<AtomicUsize>;

pub fn new_state() -> (SharedTx, SharedValue, SharedCount) {
    let (tx, _) = broadcast::channel::<String>(2000);
    (
        Arc::new(tx),
        Arc::new(Mutex::new(String::new())),
        Arc::new(AtomicUsize::new(0)),
    )
}
