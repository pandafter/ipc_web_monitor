use ipc_web_monitor::format_message;
use std::os::unix::net::UnixStream;
use std::io::Write;
use std::thread;
use std::time::Duration;

fn main() -> std::io::Result<()> {
    let socket_path = "/tmp/ipc_socket";

    for i in 0..10000 {
        if let Ok(mut stream) = UnixStream::connect(socket_path) {
            let message = format_message("Sensor data", i);
            let _ = stream.write_all(message.as_bytes());
        }
        thread::sleep(Duration::from_micros(50)); // controla velocidad
    }

    Ok(())
}
