use std::time::{SystemTime, UNIX_EPOCH};

/// Obtiene el timestamp actual en microsegundos
pub fn current_timestamp_micros() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("Error obteniendo timestamp")
        .as_micros()
}

/// Formatea un mensaje con timestamp y contenido
pub fn format_message(prefix: &str, counter: u64) -> String {
    let timestamp = current_timestamp_micros();
    format!("{}|{} #{}", timestamp, prefix, counter)
}
