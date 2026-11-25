use super::{fits, host, inspect, source, transfer, Request};
use serde::{Deserialize, Serialize};
use std::{
    io::{BufRead, BufReader, Read, Write},
    net::{Ipv4Addr, TcpListener, TcpStream},
    path::Path,
    time::{Duration, Instant},
};
use tauri::{AppHandle, Emitter};

#[derive(Serialize, Deserialize, Clone)]
struct Progress {
    phase: String,
    bytes: u64,
}

fn packet(stream: &mut TcpStream, bytes: &[u8]) -> Result<(), String> {
    stream
        .write_all(bytes)
        .and_then(|_| stream.write_all(b"\n"))
        .map_err(|e| e.to_string())
}
fn line(reader: &mut BufReader<TcpStream>) -> Result<String, String> {
    use std::io::Read;
    let mut buf = Vec::new();
    let n = reader
        .take(64 * 1024)
        .read_until(b'\n', &mut buf)
        .map_err(|e| e.to_string())?;
    if n == 0 || buf.pop() != Some(b'\n') {
        return Err("写卡进程通信中断".into());
    }
    String::from_utf8(buf).map_err(|e| e.to_string())
}

fn perform(
    req: Request,
    input: impl Read,
    mut progress: impl FnMut(&str, u64),
) -> Result<(), String> {
    fits(&req.card, &req.image)?;
    if !host::list()?.contains(&req.card) {
        return Err("卡片已更换，请重新选择".into());
    }
    let mut target = host::open(&req.card)?;
    let flush = target.disk.try_clone().map_err(|e| e.to_string())?;
    transfer(
        input,
        &mut target.disk,
        &req.image,
        || {
            flush.sync_all()?;
            #[cfg(target_os = "linux")]
            {
                use std::os::fd::AsRawFd;
                let r = unsafe {
                    libc::posix_fadvise(flush.as_raw_fd(), 0, 0, libc::POSIX_FADV_DONTNEED)
                };
                if r != 0 {
                    return Err(std::io::Error::from_raw_os_error(r));
                }
            }
            Ok(())
        },
        &mut progress,
    )
}
pub fn entry() -> Option<i32> {
    let mut args = std::env::args().skip(1);
    if args.next().as_deref() != Some("--imager-card-worker") {
        return None;
    }
    let result = (|| {
        let port = args
            .next()
            .and_then(|s| s.parse::<u16>().ok())
            .ok_or("无效端口")?;
        let token = args
            .next()
            .filter(|s| s.len() == 32 && s.bytes().all(|b| b.is_ascii_hexdigit()))
            .ok_or("无效会话")?;
        if args.next().is_some() {
            return Err("无效参数".to_string());
        }
        let mut stream = TcpStream::connect_timeout(
            &(Ipv4Addr::LOCALHOST, port).into(),
            Duration::from_secs(10),
        )
        .map_err(|e| e.to_string())?;
        stream
            .set_write_timeout(Some(Duration::from_secs(5)))
            .map_err(|e| e.to_string())?;
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .map_err(|e| e.to_string())?;
        packet(&mut stream, token.as_bytes())?;
        let mut reader = BufReader::new(stream.try_clone().map_err(|e| e.to_string())?);
        let req: Request = serde_json::from_str(&line(&mut reader)?).map_err(|e| e.to_string())?;
        let mut last = Instant::now();
        let mut prev = String::new();
        let result = perform(req, reader, |phase, bytes| {
            if prev != phase || last.elapsed() >= Duration::from_millis(350) {
                let p = Progress {
                    phase: phase.to_owned(),
                    bytes,
                };
                let _ = packet(&mut stream, &serde_json::to_vec(&p).unwrap());
                last = Instant::now();
                prev = phase.to_owned();
            }
        });
        let done = serde_json::json!({"done":true,"error":result.as_ref().err()});
        packet(&mut stream, &serde_json::to_vec(&done).unwrap())?;
        result
    })();
    Some(if result.is_ok() { 0 } else { 1 })
}

pub(super) fn launch(app: &AppHandle, req: Request) -> Result<(), String> {
    let checked = inspect(req.image.path.clone())?;
    if checked.bytes != req.image.bytes || checked.sha256 != req.image.sha256 {
        return Err("镜像已改变，请重新选择".into());
    }
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).map_err(|e| e.to_string())?;
    let token = format!("{:032x}", rand::random::<u128>());
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    elevate(&exe, port, &token)?;
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    let deadline = Instant::now() + Duration::from_secs(90);
    let (mut stream, mut reader) = loop {
        if Instant::now() > deadline {
            return Err("写卡授权已取消或超时".into());
        }
        match listener.accept() {
            Ok((s, _)) => {
                s.set_read_timeout(Some(Duration::from_secs(2)))
                    .map_err(|e| e.to_string())?;
                let mut r = BufReader::new(s.try_clone().map_err(|e| e.to_string())?);
                if line(&mut r).is_ok_and(|v| v == token) {
                    break (s, r);
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(80))
            }
            Err(e) => return Err(e.to_string()),
        }
    };
    stream.set_read_timeout(None).map_err(|e| e.to_string())?;
    packet(
        &mut stream,
        &serde_json::to_vec(&req).map_err(|e| e.to_string())?,
    )?;
    let image_path = req.image.path.clone();
    let sender = std::thread::spawn(move || -> Result<(), String> {
        let copied = source(&image_path)
            .and_then(|mut src| std::io::copy(&mut src, &mut stream).map_err(|e| e.to_string()));
        let _ = stream.shutdown(std::net::Shutdown::Write);
        copied.map(|_| ())
    });
    loop {
        let update: serde_json::Value =
            serde_json::from_str(&line(&mut reader)?).map_err(|e| e.to_string())?;
        if update["done"] == true {
            return match update["error"].as_str() {
                Some(e) => Err(e.into()),
                None => sender.join().map_err(|_| "镜像读取中断")?,
            };
        }
        let p: Progress = serde_json::from_value(update).map_err(|e| e.to_string())?;
        let _ = app.emit("imager-card-progress", p);
    }
}

#[cfg(windows)]
fn elevate(exe: &Path, port: u16, token: &str) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows::{
        core::{w, PCWSTR},
        Win32::UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_HIDE},
    };
    let path: Vec<_> = exe.as_os_str().encode_wide().chain(Some(0)).collect();
    let args: Vec<_> = format!("--imager-card-worker {port} {token}")
        .encode_utf16()
        .chain(Some(0))
        .collect();
    let result = unsafe {
        ShellExecuteW(
            None,
            w!("runas"),
            PCWSTR(path.as_ptr()),
            PCWSTR(args.as_ptr()),
            None,
            SW_HIDE,
        )
    };
    if result.0 as isize <= 32 {
        Err("Windows 写卡授权已取消或失败".into())
    } else {
        Ok(())
    }
}
#[cfg(target_os = "linux")]
fn elevate(exe: &Path, port: u16, token: &str) -> Result<(), String> {
    let mut c = if unsafe { libc::geteuid() } == 0 {
        std::process::Command::new(exe)
    } else {
        let mut cmd = std::process::Command::new("/usr/bin/pkexec");
        cmd.arg(exe);
        cmd
    };
    let mut child = c
        .args(["--imager-card-worker", &port.to_string(), token])
        .spawn()
        .map_err(|e| format!("无法申请写卡权限：{e}"))?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}
#[cfg(target_os = "macos")]
fn elevate(exe: &Path, port: u16, token: &str) -> Result<(), String> {
    use std::{
        ffi::{c_void, CString},
        os::unix::ffi::OsStrExt,
        ptr,
    };
    #[link(name = "Security", kind = "framework")]
    extern "C" {
        fn AuthorizationCreate(
            rights: *const c_void,
            env: *const c_void,
            flags: u32,
            auth: *mut *mut c_void,
        ) -> i32;
        fn AuthorizationExecuteWithPrivileges(
            auth: *mut c_void,
            path: *const libc::c_char,
            flags: u32,
            args: *const *const libc::c_char,
            pipe: *mut *mut libc::FILE,
        ) -> i32;
        fn AuthorizationFree(auth: *mut c_void, flags: u32) -> i32;
    }
    let path = CString::new(exe.as_os_str().as_bytes()).map_err(|e| e.to_string())?;
    let argv = [
        CString::new("--imager-card-worker").unwrap(),
        CString::new(port.to_string()).unwrap(),
        CString::new(token).unwrap(),
    ];
    let args = [
        argv[0].as_ptr(),
        argv[1].as_ptr(),
        argv[2].as_ptr(),
        ptr::null(),
    ];
    let mut auth = ptr::null_mut();
    if unsafe { AuthorizationCreate(ptr::null(), ptr::null(), 0, &mut auth) } != 0 {
        return Err("macOS 无法申请写卡权限".into());
    }
    let status = unsafe {
        AuthorizationExecuteWithPrivileges(auth, path.as_ptr(), 0, args.as_ptr(), ptr::null_mut())
    };
    unsafe {
        AuthorizationFree(auth, 0);
    }
    if status == 0 {
        Ok(())
    } else {
        Err(format!("macOS 写卡授权失败（{status}）"))
    }
}
