pub mod commands;

use serialport::SerialPort;
use std::collections::{HashMap, VecDeque};
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter, Manager};

/// Shared stop flag for read threads.
pub type StopFlag = Arc<AtomicBool>;

/// REST 读取缓冲：读线程收到的字节在 emit Tauri 事件的同时也压入此环形缓冲，
/// 供 HTTP 服务的 `GET /serial/read` 端点 drain（前端串口工具仍走事件，互不影响）。
pub type ReadBuffer = Arc<Mutex<VecDeque<u8>>>;

/// REST 读缓冲上限（字节）。超出时丢弃最旧数据，避免无人 drain 时无限增长。
const READ_BUFFER_CAP: usize = 64 * 1024;

/// Coalesce the small USB packets produced by CDC-ACM adapters before crossing
/// the Tauri event boundary. Eight milliseconds stays below one display frame
/// while avoiding hundreds of JSON/event dispatches during a busy boot log.
const SERIAL_EVENT_BATCH_WINDOW: Duration = Duration::from_millis(8);
const SERIAL_EVENT_BATCH_CAP: usize = 16 * 1024;
pub const SERIAL_CAPTURE_READ_CAP: usize = 256 * 1024;
const SERIAL_CAPTURE_RETENTION: Duration = Duration::from_secs(7 * 24 * 60 * 60);
const SERIAL_CAPTURE_TOTAL_CAP: u64 = 1024 * 1024 * 1024;

#[derive(Debug)]
struct SerialCaptureInner {
    writer: File,
    next_offset: u64,
    capture_error: Option<String>,
    emit_errors: u64,
}

/// Authoritative byte capture for one serial-open session. Incoming bytes are
/// persisted before they cross the WebView event boundary, so a lost event can
/// be detected by offset and replayed without using the drain-only REST buffer.
#[derive(Debug)]
pub struct SerialCapture {
    pub session_id: String,
    path: PathBuf,
    inner: Mutex<SerialCaptureInner>,
}

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SerialCaptureStatus {
    pub port: String,
    pub session_id: String,
    pub next_offset: u64,
    pub capture_error: Option<String>,
    pub emit_errors: u64,
    pub connected: bool,
}

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SerialCaptureChunk {
    pub session_id: String,
    pub start_offset: u64,
    pub next_offset: u64,
    pub capture_end_offset: u64,
    pub data: Vec<u8>,
}

impl SerialCapture {
    pub fn create(app: &AppHandle) -> Result<Arc<Self>, String> {
        let root = app
            .path()
            .app_data_dir()
            .map_err(|error| format!("SERIAL_CAPTURE_PATH:{error}"))?
            .join("serial-captures");
        Self::create_in(&root)
    }

    fn create_in(root: &Path) -> Result<Arc<Self>, String> {
        std::fs::create_dir_all(root).map_err(|error| format!("SERIAL_CAPTURE_CREATE:{error}"))?;
        cleanup_closed_captures(root);
        let epoch_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        let session_id = format!("serial-{epoch_ms}-{:016x}", rand::random::<u64>());
        let path = root.join(format!("{session_id}.active"));
        let mut options = OpenOptions::new();
        options.create_new(true).read(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let writer = options
            .open(&path)
            .map_err(|error| format!("SERIAL_CAPTURE_CREATE:{error}"))?;
        Ok(Arc::new(Self {
            session_id,
            path,
            inner: Mutex::new(SerialCaptureInner {
                writer,
                next_offset: 0,
                capture_error: None,
                emit_errors: 0,
            }),
        }))
    }

    fn append(&self, data: &[u8]) -> (u64, u64) {
        let mut inner = match self.inner.lock() {
            Ok(inner) => inner,
            Err(_) => return (0, 0),
        };
        let start_offset = inner.next_offset;
        if inner.capture_error.is_none() {
            if let Err(error) = inner.writer.write_all(data) {
                let message = format!("SERIAL_CAPTURE_WRITE:{error}");
                log::error!("Serial capture write failed: {message}");
                inner.capture_error = Some(message);
            }
        }
        inner.next_offset = inner.next_offset.saturating_add(data.len() as u64);
        (start_offset, inner.next_offset)
    }

    fn record_emit_error(&self, error: impl std::fmt::Display) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.emit_errors = inner.emit_errors.saturating_add(1);
        }
        log::error!("Serial WebView event delivery failed: {error}");
    }

    pub fn status(&self, port: &str, connected: bool) -> Result<SerialCaptureStatus, String> {
        let inner = self
            .inner
            .lock()
            .map_err(|_| "SERIAL_CAPTURE_LOCK:poisoned".to_owned())?;
        Ok(SerialCaptureStatus {
            port: port.to_owned(),
            session_id: self.session_id.clone(),
            next_offset: inner.next_offset,
            capture_error: inner.capture_error.clone(),
            emit_errors: inner.emit_errors,
            connected,
        })
    }

    pub fn read_range(&self, offset: u64, max_bytes: usize) -> Result<SerialCaptureChunk, String> {
        let capture_end_offset = self
            .inner
            .lock()
            .map_err(|_| "SERIAL_CAPTURE_LOCK:poisoned".to_owned())?
            .next_offset;
        if offset > capture_end_offset {
            return Err(format!(
                "SERIAL_CAPTURE_OFFSET:requested={offset}:end={capture_end_offset}"
            ));
        }
        let take = (capture_end_offset - offset)
            .min(max_bytes.clamp(1, SERIAL_CAPTURE_READ_CAP) as u64) as usize;
        let mut reader =
            File::open(&self.path).map_err(|error| format!("SERIAL_CAPTURE_READ:{error}"))?;
        reader
            .seek(SeekFrom::Start(offset))
            .map_err(|error| format!("SERIAL_CAPTURE_READ:{error}"))?;
        let mut data = vec![0_u8; take];
        reader
            .read_exact(&mut data)
            .map_err(|error| format!("SERIAL_CAPTURE_READ:{error}"))?;
        Ok(SerialCaptureChunk {
            session_id: self.session_id.clone(),
            start_offset: offset,
            next_offset: offset + data.len() as u64,
            capture_end_offset,
            data,
        })
    }

    pub fn export_to(&self, target: &Path) -> Result<u64, String> {
        if target.as_os_str().is_empty() {
            return Err("SERIAL_CAPTURE_EXPORT_PATH:empty".to_owned());
        }
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| "SERIAL_CAPTURE_LOCK:poisoned".to_owned())?;
        inner
            .writer
            .flush()
            .map_err(|error| format!("SERIAL_CAPTURE_FLUSH:{error}"))?;
        std::fs::copy(&self.path, target).map_err(|error| format!("SERIAL_CAPTURE_EXPORT:{error}"))
    }
}

impl Drop for SerialCapture {
    fn drop(&mut self) {
        if let Ok(mut inner) = self.inner.lock() {
            let _ = inner.writer.flush();
        }
        let closed_path = self.path.with_extension("bin");
        if let Err(error) = std::fs::rename(&self.path, &closed_path) {
            log::warn!("Could not finalize serial capture file: {error}");
        }
    }
}

fn cleanup_closed_captures(root: &Path) {
    let now = SystemTime::now();
    let mut captures = match std::fs::read_dir(root) {
        Ok(entries) => entries
            .flatten()
            .filter_map(|entry| {
                let path = entry.path();
                let extension = path.extension().and_then(|value| value.to_str());
                if !matches!(extension, Some("bin" | "active")) {
                    return None;
                }
                let active = extension == Some("active");
                let metadata = entry.metadata().ok()?;
                Some((
                    path,
                    metadata.modified().unwrap_or(UNIX_EPOCH),
                    metadata.len(),
                    active,
                ))
            })
            .collect::<Vec<_>>(),
        Err(_) => return,
    };
    // A process kill cannot run SerialCapture::drop(), so its `.active` file
    // never becomes `.bin`. Treat old active files as abandoned sessions and
    // apply the same retention boundary instead of leaking them forever.
    for (path, modified, _, _) in &captures {
        if now.duration_since(*modified).unwrap_or_default() > SERIAL_CAPTURE_RETENTION {
            let _ = std::fs::remove_file(path);
        }
    }
    captures.retain(|(path, _, _, active)| path.exists() && !active);
    captures.sort_by_key(|(_, modified, _, _)| *modified);
    let mut total = captures.iter().map(|(_, _, size, _)| size).sum::<u64>();
    for (path, _, size, _) in captures {
        if total <= SERIAL_CAPTURE_TOTAL_CAP {
            break;
        }
        if std::fs::remove_file(path).is_ok() {
            total = total.saturating_sub(size);
        }
    }
}

/// Handle to an open serial port with its read thread.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SerialPortConfiguration {
    pub baud_rate: u32,
    pub data_bits: u8,
    pub stop_bits: u8,
    pub parity: String,
    pub flow_control: String,
}

pub struct SerialPortHandle {
    /// Dedicated writer handle. The reader owns a cloned OS handle so a
    /// blocking CDC-ACM read can never delay interactive TX.
    pub writer: Mutex<Box<dyn SerialPort>>,
    pub configuration: SerialPortConfiguration,
    pub read_thread: Option<JoinHandle<()>>,
    pub stop_flag: StopFlag,
    pub disconnected: Arc<AtomicBool>,
    /// REST 读缓冲（由读线程填充，HTTP `serial_read_core` drain）。
    pub read_buffer: ReadBuffer,
    pub capture: Arc<SerialCapture>,
    /// Application handle retained so non-IPC writers/closers use the same
    /// backend audit path as direct UI commands.
    pub audit_app: Option<AppHandle>,
    pub audit_context: Option<SerialAuditContext>,
}

#[derive(Clone, Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SerialAuditContext {
    pub actor_type: String,
    pub actor_id: String,
    pub scope_type: String,
    pub scope_id: String,
    pub session_id: String,
    pub session_title: String,
}

impl Drop for SerialPortHandle {
    fn drop(&mut self) {
        // Signal the read thread to stop
        self.stop_flag
            .store(true, std::sync::atomic::Ordering::Relaxed);
        // Join the read thread so it releases its cloned OS handle.
        if let Some(thread) = self.read_thread.take() {
            let _ = thread.join();
        }
        // The independent writer handle is closed when this struct is dropped.
    }
}

/// Managed state holding all open serial port connections.
pub struct SerialState {
    pub ports: Arc<Mutex<HashMap<String, SerialPortHandle>>>,
}

impl SerialState {
    pub fn new() -> Self {
        Self {
            ports: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Close all open ports — call on app shutdown
    pub fn close_all(&self) {
        if let Ok(mut ports) = self.ports.lock() {
            ports.clear();
        }
    }
}

impl Drop for SerialState {
    fn drop(&mut self) {
        self.close_all();
    }
}

/// Delivery-side dependencies owned by one serial read thread. Keeping them in
/// one value makes the single-reader/fan-out boundary explicit and prevents
/// additional consumers from turning into more positional thread arguments.
pub struct SerialReadDelivery {
    pub read_buffer: ReadBuffer,
    pub app_handle: AppHandle,
    pub audit_context: Option<SerialAuditContext>,
    pub capture: Arc<SerialCapture>,
}

/// Spawn a read thread that continuously reads from the serial port
/// and emits data via Tauri events (并同时写入 REST 读缓冲)。
pub fn spawn_read_thread(
    port_name: String,
    mut reader: Box<dyn SerialPort>,
    stop_flag: StopFlag,
    disconnected: Arc<AtomicBool>,
    delivery: SerialReadDelivery,
) -> JoinHandle<()> {
    std::thread::spawn(move || {
        let SerialReadDelivery {
            read_buffer,
            app_handle,
            audit_context,
            capture,
        } = delivery;
        let mut buf = vec![0u8; 4096];
        let mut pending = Vec::with_capacity(SERIAL_EVENT_BATCH_CAP);
        let mut batch_started: Option<Instant> = None;
        // One event is enough to mark this session incomplete. Emitting on
        // every subsequent byte would flood the audit store when no REST
        // client drains the compatibility buffer.
        let mut gap_reported = false;
        let flush_pending = |pending: &mut Vec<u8>, gap_reported: &mut bool| {
            if pending.is_empty() {
                return;
            }
            let data = std::mem::take(pending);
            let (start_offset, end_offset) = capture.append(&data);
            if let Err(error) = app_handle.emit(
                "serial-data-received",
                SerialDataEvent {
                    port: port_name.clone(),
                    session_id: capture.session_id.clone(),
                    start_offset,
                    end_offset,
                    data: data.clone(),
                },
            ) {
                capture.record_emit_error(error);
            }
            // Keep the compatibility read buffer in the same batches as the
            // visible event path. It is bounded independently of xterm.
            if let Ok(mut rb) = read_buffer.lock() {
                rb.extend(data.iter().copied());
                let overflow = rb.len().saturating_sub(READ_BUFFER_CAP);
                if overflow > 0 {
                    rb.drain(0..overflow);
                    if !*gap_reported {
                        *gap_reported = true;
                        if let Some(context) = audit_context.as_ref() {
                            commands::publish_serial_gap(
                                &app_handle,
                                context,
                                &port_name,
                                overflow,
                            );
                        }
                    }
                }
            }
        };
        loop {
            if stop_flag.load(Ordering::Relaxed) {
                flush_pending(&mut pending, &mut gap_reported);
                break;
            }
            let result = reader.read(&mut buf);
            match result {
                Ok(n) if n > 0 => {
                    if batch_started.is_none() {
                        batch_started = Some(Instant::now());
                    }
                    pending.extend_from_slice(&buf[..n]);
                    if pending.len() >= SERIAL_EVENT_BATCH_CAP
                        || batch_started
                            .is_some_and(|started| started.elapsed() >= SERIAL_EVENT_BATCH_WINDOW)
                    {
                        flush_pending(&mut pending, &mut gap_reported);
                        batch_started = None;
                    }
                    // Keep draining packets until the batch window or capacity
                    // is reached. The short port timeout flushes sparse output.
                    continue;
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::TimedOut => {
                    // A CDC adapter can leave a few milliseconds of air
                    // between packets from the same UART burst. Do not turn
                    // that tiny gap back into a separate WebKit event.
                    if batch_started
                        .is_some_and(|started| started.elapsed() >= SERIAL_EVENT_BATCH_WINDOW)
                    {
                        flush_pending(&mut pending, &mut gap_reported);
                        batch_started = None;
                    }
                }
                Err(_) => {
                    flush_pending(&mut pending, &mut gap_reported);
                    disconnected.store(true, Ordering::Relaxed);
                    if let Some(context) = audit_context.as_ref() {
                        commands::publish_serial_disconnected(
                            &app_handle,
                            context,
                            &port_name,
                            "unexpected_disconnect",
                        );
                    }
                    // Port error, emit empty event as disconnect signal
                    let end_offset = capture
                        .status(&port_name, false)
                        .map(|status| status.next_offset)
                        .unwrap_or_default();
                    if let Err(error) = app_handle.emit(
                        "serial-data-received",
                        SerialDataEvent {
                            port: port_name.clone(),
                            session_id: capture.session_id.clone(),
                            start_offset: end_offset,
                            end_offset,
                            data: vec![],
                        },
                    ) {
                        capture.record_emit_error(error);
                    }
                    break;
                }
                _ => {
                    if batch_started
                        .is_some_and(|started| started.elapsed() >= SERIAL_EVENT_BATCH_WINDOW)
                    {
                        flush_pending(&mut pending, &mut gap_reported);
                        batch_started = None;
                    }
                }
            }
            // Successful reads continue above. Back off only after a timeout
            // or a zero-length read to avoid a disconnected driver busy-loop.
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    })
}

/// Event payload for serial data received.
#[derive(Clone, serde::Serialize)]
pub struct SerialDataEvent {
    pub port: String,
    pub session_id: String,
    pub start_offset: u64,
    pub end_offset: u64,
    pub data: Vec<u8>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

    #[test]
    fn capture_offsets_and_range_reads_are_lossless() {
        let root = std::env::temp_dir().join(format!(
            "lynx-serial-capture-test-{}-{:016x}",
            std::process::id(),
            rand::random::<u64>()
        ));
        let capture = SerialCapture::create_in(&root).expect("create capture");
        assert_eq!(capture.append(b"abc"), (0, 3));
        assert_eq!(capture.append(b"defgh"), (3, 8));
        let chunk = capture.read_range(2, 4).expect("read capture range");
        assert_eq!(chunk.start_offset, 2);
        assert_eq!(chunk.next_offset, 6);
        assert_eq!(chunk.capture_end_offset, 8);
        assert_eq!(chunk.data, b"cdef");
        assert!(capture.read_range(9, 4).is_err());
        drop(capture);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn fragmented_ten_megabyte_capture_replays_with_identical_digest() {
        let root = std::env::temp_dir().join(format!(
            "lynx-serial-throughput-test-{}-{:016x}",
            std::process::id(),
            rand::random::<u64>()
        ));
        let capture = SerialCapture::create_in(&root).expect("create capture");
        let expected = (0..10 * 1024 * 1024)
            .map(|index| ((index * 31 + index / 97) & 0xff) as u8)
            .collect::<Vec<_>>();
        let mut written = 0;
        let chunk_sizes = [1_usize, 7, 20, 511, 4096, 13, 2048, 3];
        let mut chunk_index = 0;
        while written < expected.len() {
            let end = (written + chunk_sizes[chunk_index % chunk_sizes.len()]).min(expected.len());
            let (start_offset, end_offset) = capture.append(&expected[written..end]);
            assert_eq!(start_offset, written as u64);
            assert_eq!(end_offset, end as u64);
            written = end;
            chunk_index += 1;
        }

        let mut replayed = Vec::with_capacity(expected.len());
        let mut offset = 0_u64;
        while offset < expected.len() as u64 {
            let chunk = capture
                .read_range(offset, SERIAL_CAPTURE_READ_CAP)
                .expect("replay capture");
            assert_eq!(chunk.start_offset, offset);
            offset = chunk.next_offset;
            replayed.extend_from_slice(&chunk.data);
        }
        assert_eq!(Sha256::digest(&replayed), Sha256::digest(&expected));
        drop(capture);
        let _ = std::fs::remove_dir_all(root);
    }
}
