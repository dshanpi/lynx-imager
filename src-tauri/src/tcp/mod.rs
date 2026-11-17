pub mod commands;

use std::collections::HashMap;
use std::io::Read;
use std::net::TcpStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use tauri::{AppHandle, Emitter};

pub type StopFlag = Arc<AtomicBool>;

pub struct TcpHandle {
    pub stream: Arc<Mutex<TcpStream>>,
    pub read_thread: Option<JoinHandle<()>>,
    pub stop_flag: StopFlag,
}

pub struct TcpState {
    pub connections: Arc<Mutex<HashMap<String, TcpHandle>>>,
}

impl TcpState {
    pub fn new() -> Self {
        Self {
            connections: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

pub fn spawn_tcp_read_thread(
    conn_id: String,
    shared_stream: Arc<Mutex<TcpStream>>,
    stop_flag: StopFlag,
    app_handle: AppHandle,
) -> JoinHandle<()> {
    std::thread::spawn(move || {
        let mut buf = vec![0u8; 4096];
        loop {
            if stop_flag.load(Ordering::Relaxed) {
                break;
            }
            let result = {
                let mut stream = match shared_stream.lock() {
                    Ok(s) => s,
                    Err(_) => break,
                };
                stream
                    .set_read_timeout(Some(std::time::Duration::from_millis(100)))
                    .ok();
                stream.read(&mut buf)
            };
            match result {
                Ok(n) if n > 0 => {
                    let data: Vec<u8> = buf[..n].to_vec();
                    let _ = app_handle.emit(
                        "tcp-data-received",
                        TcpDataEvent {
                            id: conn_id.clone(),
                            data,
                        },
                    );
                }
                Err(ref e)
                    if e.kind() == std::io::ErrorKind::WouldBlock
                        || e.kind() == std::io::ErrorKind::TimedOut => {}
                Err(_) => {
                    let _ = app_handle.emit("tcp-disconnected", conn_id.clone());
                    break;
                }
                _ => {}
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    })
}

#[derive(Clone, serde::Serialize)]
pub struct TcpDataEvent {
    pub id: String,
    pub data: Vec<u8>,
}
