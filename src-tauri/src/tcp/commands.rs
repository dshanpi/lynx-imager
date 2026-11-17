use crate::tcp;
use std::io::Write;
use tauri::State;

#[tauri::command]
pub fn tcp_connect(
    state: State<'_, tcp::TcpState>,
    id: String,
    host: String,
    port: u16,
) -> Result<(), String> {
    let addr = format!("{}:{}", host, port);
    let stream = std::net::TcpStream::connect(&addr)
        .map_err(|e| format!("TCP connect to {} failed: {}", addr, e))?;
    stream.set_nonblocking(false).map_err(|e| e.to_string())?;

    let mut connections = state.connections.lock().map_err(|e| e.to_string())?;
    // Close existing connection with same id
    if let Some(old) = connections.remove(&id) {
        old.stop_flag
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }
    connections.insert(
        id,
        tcp::TcpHandle {
            stream: std::sync::Arc::new(std::sync::Mutex::new(stream)),
            read_thread: None,
            stop_flag: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        },
    );
    Ok(())
}

#[tauri::command]
pub fn tcp_send(state: State<'_, tcp::TcpState>, id: String, data: Vec<u8>) -> Result<(), String> {
    let connections = state.connections.lock().map_err(|e| e.to_string())?;
    let handle = connections.get(&id).ok_or("TCP connection not found")?;
    let mut stream = handle.stream.lock().map_err(|e| e.to_string())?;
    stream
        .write_all(&data)
        .map_err(|e| format!("TCP send error: {}", e))?;
    stream
        .flush()
        .map_err(|e| format!("TCP flush error: {}", e))?;
    Ok(())
}

#[tauri::command]
pub fn tcp_start_read(
    state: State<'_, tcp::TcpState>,
    app_handle: tauri::AppHandle,
    id: String,
) -> Result<(), String> {
    let mut connections = state.connections.lock().map_err(|e| e.to_string())?;
    let handle = connections.get_mut(&id).ok_or("TCP connection not found")?;

    // Stop existing read thread if any
    handle
        .stop_flag
        .store(true, std::sync::atomic::Ordering::Relaxed);
    handle.stop_flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));

    let stream = handle.stream.clone();
    let stop_flag = handle.stop_flag.clone();
    handle.read_thread = Some(tcp::spawn_tcp_read_thread(
        id, stream, stop_flag, app_handle,
    ));
    Ok(())
}

#[tauri::command]
pub fn tcp_close(state: State<'_, tcp::TcpState>, id: String) -> Result<(), String> {
    let mut connections = state.connections.lock().map_err(|e| e.to_string())?;
    if let Some(handle) = connections.remove(&id) {
        handle
            .stop_flag
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }
    Ok(())
}
