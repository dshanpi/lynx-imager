//! Task-owned, hardware-free preparation for complete RK eMMC images.
use fs2::FileExt;
use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

const RESERVE: u64 = 64 * 1024 * 1024;
const MAX_IMAGE: u64 = u32::MAX as u64 * 512;

pub(super) struct Prepared {
    pub image: PathBuf,
    pub loader: PathBuf,
    pub source_sha256: String,
    pub image_sha256: String,
    pub loader_sha256: String,
    pub bytes: u64,
    directory: PathBuf,
    owner: Option<File>,
}
impl Drop for Prepared {
    fn drop(&mut self) {
        self.owner.take();
        // A Unix helper may outlive its GUI after a crash or an early error.
        // Its inherited stdin retains the same flock open-file description.
        let Ok(lock) = open_lock(&self.directory.join("owner.lock")) else {
            return;
        };
        if FileExt::try_lock_exclusive(&lock).is_err() {
            return;
        }
        drop(lock);
        if let Err(error) = fs::remove_dir_all(&self.directory) {
            log::warn!("RK raw temporary cleanup failed: {error}");
        }
    }
}
impl Prepared {
    pub(super) fn protect_child(&self, command: &mut std::process::Command) -> Result<(), String> {
        #[cfg(unix)]
        {
            // Raw CLI commands do not read stdin. Stdio performs the inherited
            // descriptor setup without unsafe pre_exec callbacks; closing the
            // parent does not release flock until the helper also exits.
            let owner = self
                .owner
                .as_ref()
                .ok_or_else(|| err("missing cache owner"))?;
            command.stdin(std::process::Stdio::from(owner.try_clone().map_err(err)?));
        }
        // Windows helpers are already owned by a kill-on-close Job Object.
        #[cfg(not(unix))]
        let _ = command;
        Ok(())
    }
}
fn err(error: impl std::fmt::Display) -> String {
    format!("ROCKCHIP_RAW_PREPARE:{error}")
}
fn open_lock(path: &Path) -> Result<File, String> {
    OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
        .map_err(err)
}
// The root lock closes the race between directory creation, owner acquisition,
// and cleanup by another LYNX process. An OS lock survives no process crash.
fn allocate(root: &Path) -> Result<Prepared, String> {
    fs::create_dir_all(root).map_err(err)?;
    let root_lock = open_lock(&root.join(".lock"))?;
    FileExt::lock_exclusive(&root_lock).map_err(err)?;
    cleanup_locked(root)?;
    let directory = root.join(format!(
        "task-{}-{:016x}",
        std::process::id(),
        rand::random::<u64>()
    ));
    fs::create_dir(&directory).map_err(err)?;
    let owner = open_lock(&directory.join("owner.lock"))?;
    FileExt::lock_exclusive(&owner).map_err(err)?;
    Ok(Prepared {
        image: directory.join("disk.img"),
        loader: directory.join("loader.bin"),
        source_sha256: String::new(),
        image_sha256: String::new(),
        loader_sha256: String::new(),
        bytes: 0,
        directory,
        owner: Some(owner),
    })
}
fn cleanup_locked(root: &Path) -> Result<(), String> {
    for entry in fs::read_dir(root).map_err(err)? {
        let entry = entry.map_err(err)?;
        if !entry.file_name().to_string_lossy().starts_with("task-")
            || !entry.file_type().map_err(err)?.is_dir()
        {
            continue;
        }
        let directory = entry.path();
        if let Ok(owner) = open_lock(&directory.join("owner.lock")) {
            if FileExt::try_lock_exclusive(&owner).is_ok() {
                drop(owner);
                fs::remove_dir_all(directory).map_err(err)?;
            }
        }
    }
    Ok(())
}
pub(super) fn cleanup(root: &Path) -> Result<(), String> {
    if !root.exists() {
        return Ok(());
    }
    let lock = open_lock(&root.join(".lock"))?;
    FileExt::lock_exclusive(&lock).map_err(err)?;
    cleanup_locked(root)
}
struct HashReader<R> {
    inner: R,
    hash: Sha256,
}
impl<R: Read> Read for HashReader<R> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buffer)?;
        self.hash.update(&buffer[..n]);
        Ok(n)
    }
}
fn copy_checked(
    reader: &mut dyn Read,
    writer: &mut dyn Write,
    cancel: &AtomicBool,
    limit: u64,
    mut space: impl FnMut() -> Result<u64, String>,
    mut progress: impl FnMut(u64),
) -> Result<(u64, String), String> {
    let mut buffer = vec![0; 1024 * 1024];
    let mut bytes = 0u64;
    let mut hash = Sha256::new();
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err("ROCKCHIP_RAW_CANCELLED:preparation cancelled".to_owned());
        }
        let n = reader.read(&mut buffer).map_err(err)?;
        if n == 0 {
            break;
        }
        bytes = bytes
            .checked_add(n as u64)
            .filter(|v| *v <= limit)
            .ok_or_else(|| err("image exceeds supported size"))?;
        if space()? < RESERVE + n as u64 {
            return Err("ROCKCHIP_RAW_SPACE:insufficient temporary disk space".to_owned());
        }
        writer.write_all(&buffer[..n]).map_err(err)?;
        hash.update(&buffer[..n]);
        progress(bytes);
    }
    writer.flush().map_err(err)?;
    if bytes == 0 {
        return Err(err("empty file"));
    }
    Ok((bytes, hex::encode(hash.finalize())))
}
pub(super) fn prepare(
    source: &Path,
    loader: &Path,
    root: &Path,
    cancel: &AtomicBool,
    mut progress: impl FnMut(&str, u64),
) -> Result<Prepared, String> {
    let mut prepared = allocate(root)?;
    let source_file = File::open(source).map_err(err)?;
    let metadata = source_file.metadata().map_err(err)?;
    if !metadata.is_file() || metadata.len() == 0 {
        return Err(err("image must be a nonempty regular file"));
    }
    let name = source.to_string_lossy().to_ascii_lowercase();
    let gzip = name.ends_with(".img.gz");
    let xz = name.ends_with(".img.xz");
    if !gzip && !xz && !name.ends_with(".img") {
        return Err(err("supported inputs: .img, .img.gz, .img.xz"));
    }
    let available = fs2::available_space(root).map_err(err)?;
    if available
        < RESERVE.saturating_add(if gzip || xz {
            1024 * 1024
        } else {
            metadata.len()
        })
    {
        return Err("ROCKCHIP_RAW_SPACE:insufficient temporary disk space".to_owned());
    }
    let mut input = HashReader {
        inner: source_file,
        hash: Sha256::new(),
    };
    let mut output = File::create(&prepared.image).map_err(err)?;
    let phase = if gzip || xz {
        "raw_decompress"
    } else {
        "raw_prepare"
    };
    progress(phase, 0);
    let result = {
        let mut reader: Box<dyn Read + '_> = if gzip {
            Box::new(flate2::read::MultiGzDecoder::new(&mut input))
        } else if xz {
            let stream = xz2::stream::Stream::new_stream_decoder(
                256 * 1024 * 1024,
                xz2::stream::CONCATENATED,
            )
            .map_err(err)?;
            Box::new(xz2::read::XzDecoder::new_stream(&mut input, stream))
        } else {
            Box::new(&mut input)
        };
        copy_checked(
            &mut reader,
            &mut output,
            cancel,
            MAX_IMAGE,
            || fs2::available_space(root).map_err(err),
            |n| progress(phase, n),
        )?
    };
    prepared.bytes = result.0;
    prepared.image_sha256 = result.1;
    prepared.source_sha256 = hex::encode(input.hash.finalize());
    if prepared.bytes < 1024 || prepared.bytes % 512 != 0 {
        return Err(err("raw image must be sector aligned"));
    }
    output.sync_all().map_err(err)?;
    drop(output);
    let mut input = File::open(loader).map_err(err)?;
    if !input.metadata().map_err(err)?.is_file() {
        return Err(err("Loader must be a regular file"));
    }
    let mut output = File::create(&prepared.loader).map_err(err)?;
    let (_, hash) = copy_checked(
        &mut input,
        &mut output,
        cancel,
        64 * 1024 * 1024,
        || fs2::available_space(root).map_err(err),
        |_| {},
    )?;
    prepared.loader_sha256 = hash;
    output.sync_all().map_err(err)?;
    Ok(prepared)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn root() -> PathBuf {
        std::env::temp_dir().join(format!("lynx-raw-test-{:016x}", rand::random::<u64>()))
    }
    // On Unix an unrelated test can fork while this test owns a flock. Until
    // that child execs/exits it also owns the lock, even with CLOEXEC set.
    // Keep the exact lifecycle assertions, but run them without unrelated
    // test threads spawning children. This does not change production cleanup.
    fn isolated_owner_test(name: &str) -> bool {
        #[cfg(unix)]
        {
            use std::time::{Duration, Instant};
            const WORKER: &str = "LYNX_RAW_CACHE_TEST_WORKER";
            if std::env::var(WORKER).as_deref() == Ok(name) {
                return false;
            }
            let full_name = format!("workbench::flasher::rockchip_raw::tests::{name}");
            let mut child = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", &full_name, "--nocapture"])
                .env(WORKER, name)
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
                .unwrap();
            let deadline = Instant::now() + Duration::from_secs(30);
            loop {
                if let Some(status) = child.try_wait().unwrap() {
                    let output = child.wait_with_output().unwrap();
                    let stdout = String::from_utf8_lossy(&output.stdout);
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    assert!(
                        status.success(),
                        "isolated test {name} failed: {status}\n{stdout}\n{stderr}"
                    );
                    assert!(
                        stdout.contains(&format!("test {full_name} ... ok")),
                        "isolated worker did not execute the requested test: {stdout}"
                    );
                    return true;
                }
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    panic!("isolated test {name} exceeded its process lifetime bound");
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        }
        #[cfg(not(unix))]
        {
            let _ = name;
            false
        }
    }
    #[cfg(unix)]
    #[test]
    fn unrelated_fork_retains_owner_until_exec() {
        if isolated_owner_test("unrelated_fork_retains_owner_until_exec") {
            return;
        }
        use std::os::fd::AsRawFd;
        use std::os::unix::net::UnixStream;
        use std::os::unix::process::CommandExt;
        use std::time::Duration;

        let root = root();
        let prepared = allocate(&root).unwrap();
        let directory = prepared.directory.clone();
        let (mut parent, child_socket) = UnixStream::pair().unwrap();
        parent
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let child_fd = child_socket.as_raw_fd();
        let parent_fd = parent.as_raw_fd();
        let mut command = std::process::Command::new("true");
        // Test only: force a fork and hold it before exec closes CLOEXEC files.
        // Only async-signal-safe syscalls run in the child callback. No owner
        // descriptor is explicitly passed through protect_child or stdin.
        unsafe {
            command.pre_exec(move || {
                libc::close(parent_fd);
                let mut byte = 1u8;
                if libc::write(child_fd, (&byte as *const u8).cast(), 1) != 1
                    || libc::read(child_fd, (&mut byte as *mut u8).cast(), 1) != 1
                {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let spawning = std::thread::spawn(move || command.spawn().unwrap());
        let mut ready = [0];
        parent.read_exact(&mut ready).unwrap();
        assert_eq!(ready, [1]);
        drop(prepared);
        let retained_after_drop = directory.exists();
        cleanup(&root).unwrap();
        let retained_after_cleanup = directory.exists();
        parent.write_all(&[1]).unwrap();
        let mut child = spawning.join().unwrap();
        assert!(child.wait().unwrap().success());
        drop(child_socket);
        assert!(
            retained_after_drop,
            "forked child still holds the owner lock"
        );
        assert!(
            retained_after_cleanup,
            "cleanup must preserve an inherited owner"
        );
        cleanup(&root).unwrap();
        assert!(!directory.exists());
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn compressed_snapshots_and_active_owner_cleanup() {
        if isolated_owner_test("compressed_snapshots_and_active_owner_cleanup") {
            return;
        }
        let root = root();
        fs::create_dir_all(&root).unwrap();
        let data = vec![0x5a; 4096];
        let loader = root.join("loader.bin");
        fs::write(&loader, b"loader fixture").unwrap();
        for extension in ["img", "img.gz", "img.xz"] {
            let source = root.join(format!("中文 disk.{extension}"));
            let bytes = match extension {
                "img.gz" => {
                    let mut w =
                        flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
                    w.write_all(&data).unwrap();
                    w.finish().unwrap()
                }
                "img.xz" => {
                    let mut w = xz2::write::XzEncoder::new(Vec::new(), 1);
                    w.write_all(&data).unwrap();
                    w.finish().unwrap()
                }
                _ => data.clone(),
            };
            fs::write(&source, &bytes).unwrap();
            let p = prepare(
                &source,
                &loader,
                &root.join("cache"),
                &AtomicBool::new(false),
                |_, _| {},
            )
            .unwrap();
            assert_eq!(fs::read(&p.image).unwrap(), data);
            assert_eq!(p.source_sha256, hex::encode(Sha256::digest(&bytes)));
            assert_eq!(p.image_sha256, hex::encode(Sha256::digest(&data)));
            let other = allocate(&root.join("cache")).unwrap();
            assert!(p.image.exists());
            drop(other);
            let dir = p.directory.clone();
            drop(p);
            assert!(!dir.exists());
            if extension != "img" {
                fs::write(&source, &bytes[..bytes.len() - 4]).unwrap();
                assert!(prepare(
                    &source,
                    &loader,
                    &root.join("cache"),
                    &AtomicBool::new(false),
                    |_, _| {}
                )
                .is_err());
                let mut corrupt = bytes.clone();
                let i = corrupt.len() - 8;
                corrupt[i] ^= 0xff;
                fs::write(&source, corrupt).unwrap();
                assert!(prepare(
                    &source,
                    &loader,
                    &root.join("cache"),
                    &AtomicBool::new(false),
                    |_, _| {}
                )
                .is_err());
            }
        }
        let cache = root.join("cache");
        let stale = cache.join("task-stale");
        fs::create_dir(&stale).unwrap();
        fs::write(stale.join("disk.img"), b"stale").unwrap();
        let p = allocate(&cache).unwrap();
        assert!(!stale.exists());
        drop(p);
        fs::remove_dir_all(root).unwrap();
    }
    #[cfg(unix)]
    #[test]
    fn surviving_helper_prevents_drop_and_restart_cleanup() {
        if isolated_owner_test("surviving_helper_prevents_drop_and_restart_cleanup") {
            return;
        }
        let root = root();
        let prepared = allocate(&root).unwrap();
        let directory = prepared.directory.clone();
        fs::write(&prepared.image, b"active raw snapshot").unwrap();
        let mut command = std::process::Command::new("sleep");
        command.arg("60");
        prepared.protect_child(&mut command).unwrap();
        let mut child = command.spawn().unwrap();
        drop(command);
        assert!(child.try_wait().unwrap().is_none());
        drop(prepared);
        assert!(directory.join("disk.img").exists());
        cleanup(&root).unwrap();
        assert!(directory.join("disk.img").exists());
        child.kill().unwrap();
        child.wait().unwrap();
        cleanup(&root).unwrap();
        assert!(!directory.exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn cancel_space_limit_and_empty_fail_before_hardware() {
        for (cancel, space, limit, expected) in [
            (true, u64::MAX, 512, "CANCELLED"),
            (false, 0, 512, "SPACE"),
            (false, u64::MAX, 1, "size"),
        ] {
            let mut input = &[1u8; 512][..];
            let mut output = Vec::new();
            assert!(copy_checked(
                &mut input,
                &mut output,
                &AtomicBool::new(cancel),
                limit,
                || Ok(space),
                |_| {}
            )
            .unwrap_err()
            .contains(expected));
            assert!(output.is_empty());
        }
        assert!(copy_checked(
            &mut &[][..],
            &mut Vec::new(),
            &AtomicBool::new(false),
            512,
            || Ok(u64::MAX),
            |_| {}
        )
        .is_err());
    }
}
