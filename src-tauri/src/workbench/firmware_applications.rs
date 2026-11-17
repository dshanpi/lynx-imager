//! Read-only application discovery and local candidate inspection.
//! An inspected file is not a release, hardware profile, or flash authorization.
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::State;

use super::{storage, WorkbenchState};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FirmwareApplication {
    pub id: String,
    pub name: String,
    pub description: String,
    pub board_model: String,
    pub system: String,
    pub routes: Vec<String>,
    pub documentation_url: String,
    pub documentation_scope: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplicationCatalog {
    pub device_id: i64,
    pub board_uid: String,
    pub board_name: String,
    pub hardware_profile: &'static str,
    pub source: &'static str,
    pub publication_status: &'static str,
    pub repository: &'static str,
    pub releases_url: &'static str,
    pub applications: Vec<FirmwareApplication>,
}

fn applications() -> Result<Vec<FirmwareApplication>, String> {
    serde_json::from_str(include_str!("../../resources/firmware-applications.json"))
        .map_err(|_| "FIRMWARE_CATALOG_INVALID".to_owned())
}

#[tauri::command]
pub fn workbench_firmware_applications(
    state: State<'_, Arc<WorkbenchState>>,
    device_id: i64,
) -> Result<ApplicationCatalog, String> {
    let db = state.db.lock().map_err(|e| e.to_string())?;
    let board = storage::list_devices(&db)
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|board| board.id == Some(device_id))
        .ok_or("FIRMWARE_BOARD_NOT_SAVED")?;
    Ok(ApplicationCatalog {
        device_id,
        board_uid: board.board_uid,
        board_name: board.name,
        hardware_profile: "unverified",
        source: "bundled_documentation",
        publication_status: "github_configured",
        repository: super::firmware_release_source::REPOSITORY,
        releases_url: super::firmware_release_source::RELEASES_URL,
        applications: applications()?,
    })
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CandidateFile {
    pub file_name: String,
    pub size_bytes: u64,
    pub sha256: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CandidateLayout {
    pub boot_offset: u64,
    pub parameters_offset: u64,
    pub application_offset: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FirmwareCandidateManifest {
    pub schema_version: u32,
    pub application_id: String,
    pub version: String,
    pub board_model: String,
    pub chip: String,
    pub route: String,
    pub flash_capacity_bytes: u64,
    pub layout: CandidateLayout,
    pub image: CandidateFile,
    pub loader: CandidateFile,
    pub loader_source: String,
    pub source_commit: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FirmwareCandidateInspection {
    pub device_id: i64,
    pub board_uid: String,
    pub manifest: FirmwareCandidateManifest,
    pub image_path: String,
    pub loader_path: String,
    pub integrity: &'static str,
    pub hardware_compatibility: &'static str,
    pub loader_compatibility: &'static str,
    pub publication_status: &'static str,
    pub install_authorized: bool,
}

fn hex_digest(value: &str, len: usize) -> bool {
    value.len() == len && value.bytes().all(|b| b.is_ascii_hexdigit())
}

pub(super) fn validate_manifest(manifest: &FirmwareCandidateManifest) -> Result<(), String> {
    if manifest.schema_version != 1
        || manifest.application_id != "nekobox"
        || manifest.board_model != "yuzukineko"
        || manifest.chip != "f101"
        || manifest.route != "xip_spinor_full"
    {
        return Err("FIRMWARE_CANDIDATE_ROUTE_UNSUPPORTED".into());
    }
    if manifest.flash_capacity_bytes != 16 * 1024 * 1024
        || manifest.layout.boot_offset != 0
        || manifest.layout.parameters_offset != 0x20000
        || manifest.layout.application_offset != 0x80000
        || manifest.image.size_bytes <= manifest.layout.application_offset
        || manifest.image.size_bytes > manifest.flash_capacity_bytes
    {
        return Err("FIRMWARE_CANDIDATE_LAYOUT_INVALID".into());
    }
    if manifest.version.is_empty()
        || manifest.version.len() > 100
        || manifest.version.chars().any(char::is_control)
        || !hex_digest(&manifest.source_commit, 40)
        || manifest.loader_source.trim().is_empty()
        || manifest.loader_source.len() > 2048
        || manifest.loader_source.chars().any(char::is_control)
        || manifest
            .image
            .file_name
            .eq_ignore_ascii_case(&manifest.loader.file_name)
    {
        return Err("FIRMWARE_CANDIDATE_METADATA_INVALID".into());
    }
    for file in [&manifest.image, &manifest.loader] {
        // Only sibling filenames. Never execute a manifest or interpret host paths from it.
        let stem = file
            .file_name
            .split('.')
            .next()
            .unwrap_or_default()
            .to_ascii_uppercase();
        if file.file_name.eq_ignore_ascii_case("candidate.json")
            || [
                "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7",
                "COM8", "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8",
                "LPT9",
            ]
            .contains(&stem.as_str())
            || file.file_name.is_empty()
            || file.file_name.starts_with('.')
            || file.file_name.len() > 200
            || file
                .file_name
                .chars()
                .any(|c| c.is_control() || "/\\:*?\"<>|".contains(c))
            || file.file_name.ends_with([' ', '.'])
            || file.size_bytes == 0
            || file.size_bytes > 64 * 1024 * 1024
            || !hex_digest(&file.sha256, 64)
        {
            return Err("FIRMWARE_CANDIDATE_FILE_INVALID".into());
        }
    }
    Ok(())
}

pub(super) fn verify_file(root: &Path, expected: &CandidateFile) -> Result<PathBuf, String> {
    let path = root.join(&expected.file_name);
    let metadata = std::fs::symlink_metadata(&path)
        .map_err(|_| "FIRMWARE_CANDIDATE_FILE_MISSING".to_owned())?;
    if !metadata.file_type().is_file() || metadata.len() != expected.size_bytes {
        return Err("FIRMWARE_CANDIDATE_FILE_SIZE_OR_TYPE".into());
    }
    let canonical = path
        .canonicalize()
        .map_err(|_| "FIRMWARE_CANDIDATE_PATH_INVALID")?;
    if canonical.parent() != Some(root) {
        return Err("FIRMWARE_CANDIDATE_PATH_ESCAPE".into());
    }
    let mut file = File::open(&canonical).map_err(|_| "FIRMWARE_CANDIDATE_FILE_UNREADABLE")?;
    let mut hash = Sha256::new();
    let mut read = 0u64;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let n = file
            .read(&mut buffer)
            .map_err(|_| "FIRMWARE_CANDIDATE_READ_FAILED")?;
        if n == 0 {
            break;
        }
        read += n as u64;
        if read > expected.size_bytes {
            return Err("FIRMWARE_CANDIDATE_FILE_CHANGED".into());
        }
        hash.update(&buffer[..n]);
    }
    if read != expected.size_bytes
        || !hex::encode(hash.finalize()).eq_ignore_ascii_case(&expected.sha256)
    {
        return Err("FIRMWARE_CANDIDATE_HASH_MISMATCH".into());
    }
    Ok(canonical)
}

pub(super) fn inspect_candidate(
    path: &Path,
    device_id: i64,
    board_uid: String,
) -> Result<FirmwareCandidateInspection, String> {
    let metadata =
        std::fs::symlink_metadata(path).map_err(|_| "FIRMWARE_CANDIDATE_MANIFEST_MISSING")?;
    if !metadata.file_type().is_file() || metadata.len() > 64 * 1024 {
        return Err("FIRMWARE_CANDIDATE_MANIFEST_SIZE_OR_TYPE".into());
    }
    let canonical = path
        .canonicalize()
        .map_err(|_| "FIRMWARE_CANDIDATE_MANIFEST_PATH")?;
    let root = canonical
        .parent()
        .ok_or("FIRMWARE_CANDIDATE_MANIFEST_PATH")?;
    let mut bytes = Vec::new();
    File::open(&canonical)
        .map_err(|_| "FIRMWARE_CANDIDATE_MANIFEST_UNREADABLE")?
        .take(64 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "FIRMWARE_CANDIDATE_MANIFEST_UNREADABLE")?;
    if bytes.len() > 64 * 1024 {
        return Err("FIRMWARE_CANDIDATE_MANIFEST_TOO_LARGE".into());
    }
    let manifest: FirmwareCandidateManifest =
        serde_json::from_slice(&bytes).map_err(|_| "FIRMWARE_CANDIDATE_MANIFEST_INVALID")?;
    validate_manifest(&manifest)?;
    let image_path = verify_file(root, &manifest.image)?;
    let loader_path = verify_file(root, &manifest.loader)?;
    Ok(FirmwareCandidateInspection {
        device_id,
        board_uid,
        manifest,
        image_path: image_path.to_string_lossy().into_owned(),
        loader_path: loader_path.to_string_lossy().into_owned(),
        integrity: "verified",
        hardware_compatibility: "unverified",
        loader_compatibility: "unverified",
        publication_status: "local_candidate",
        install_authorized: false,
    })
}

#[tauri::command]
pub async fn workbench_inspect_firmware_candidate(
    state: State<'_, Arc<WorkbenchState>>,
    device_id: i64,
    manifest_path: String,
) -> Result<FirmwareCandidateInspection, String> {
    let uid = {
        let db = state.db.lock().map_err(|e| e.to_string())?;
        storage::list_devices(&db)
            .map_err(|e| e.to_string())?
            .into_iter()
            .find(|board| board.id == Some(device_id))
            .ok_or("FIRMWARE_BOARD_NOT_SAVED")?
            .board_uid
    };
    tauri::async_runtime::spawn_blocking(move || {
        inspect_candidate(Path::new(&manifest_path), device_id, uid)
    })
    .await
    .map_err(|_| "FIRMWARE_CANDIDATE_INSPECTION_FAILED".to_owned())?
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestDir(PathBuf);
    impl TestDir {
        fn path(&self) -> &Path {
            &self.0
        }
    }
    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn fixture() -> (TestDir, FirmwareCandidateManifest) {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = TestDir(std::env::temp_dir().join(format!(
            "lynx-candidate-test-{}-{stamp}-{id}",
            std::process::id()
        )));
        std::fs::create_dir(&dir.0).unwrap();
        let image = vec![0x5a; 600 * 1024];
        let loader = vec![0x6b; 128];
        std::fs::write(dir.path().join("nekobox-full.img"), &image).unwrap();
        std::fs::write(dir.path().join("fes.img"), &loader).unwrap();
        let m = FirmwareCandidateManifest {
            schema_version: 1,
            application_id: "nekobox".into(),
            version: "candidate-1".into(),
            board_model: "yuzukineko".into(),
            chip: "f101".into(),
            route: "xip_spinor_full".into(),
            flash_capacity_bytes: 16 * 1024 * 1024,
            layout: CandidateLayout {
                boot_offset: 0,
                parameters_offset: 0x20000,
                application_offset: 0x80000,
            },
            image: CandidateFile {
                file_name: "nekobox-full.img".into(),
                size_bytes: image.len() as u64,
                sha256: hex::encode(Sha256::digest(&image)),
            },
            loader: CandidateFile {
                file_name: "fes.img".into(),
                size_bytes: loader.len() as u64,
                sha256: hex::encode(Sha256::digest(&loader)),
            },
            loader_source: "unit-test fixture, not distributable firmware".into(),
            source_commit: "a".repeat(40),
        };
        (dir, m)
    }
    fn run(
        dir: &Path,
        m: &FirmwareCandidateManifest,
    ) -> Result<FirmwareCandidateInspection, String> {
        let path = dir.join("candidate.json");
        std::fs::write(&path, serde_json::to_vec(m).unwrap()).unwrap();
        inspect_candidate(&path, 7, "saved-board-7".into())
    }
    #[test]
    fn integrity_never_grants_hardware_or_publish_authority() {
        let (d, m) = fixture();
        let v = run(d.path(), &m).unwrap();
        assert_eq!(v.integrity, "verified");
        assert_eq!(v.device_id, 7);
        assert_eq!(v.board_uid, "saved-board-7");
        assert!(!v.install_authorized);
        assert_eq!(v.hardware_compatibility, "unverified");
        assert_eq!(v.loader_compatibility, "unverified");
        assert_eq!(v.publication_status, "local_candidate");
    }
    #[test]
    fn substitution_and_truncation_are_rejected() {
        let (d, mut m) = fixture();
        m.image.sha256 = "0".repeat(64);
        assert_eq!(
            run(d.path(), &m).unwrap_err(),
            "FIRMWARE_CANDIDATE_HASH_MISMATCH"
        );
        let (d, m) = fixture();
        std::fs::write(d.path().join(&m.loader.file_name), [1]).unwrap();
        assert_eq!(
            run(d.path(), &m).unwrap_err(),
            "FIRMWARE_CANDIDATE_FILE_SIZE_OR_TYPE"
        );
    }
    #[test]
    fn untrusted_paths_never_leave_package_directory() {
        for name in [
            "../outside.img",
            "..\\outside.img",
            "/tmp/a",
            "C:\\a.img",
            "a:b.img",
            "x\n.img",
            ".hidden",
            "a.",
        ] {
            let (d, mut m) = fixture();
            m.image.file_name = name.into();
            assert_eq!(
                run(d.path(), &m).unwrap_err(),
                "FIRMWARE_CANDIDATE_FILE_INVALID"
            );
        }
    }
    #[test]
    fn routes_and_layouts_cannot_be_interchanged() {
        let (d, mut m) = fixture();
        m.route = "uf2".into();
        assert_eq!(
            run(d.path(), &m).unwrap_err(),
            "FIRMWARE_CANDIDATE_ROUTE_UNSUPPORTED"
        );
        m.route = "xip_spinor_full".into();
        m.layout.application_offset = 0x200000;
        assert_eq!(
            run(d.path(), &m).unwrap_err(),
            "FIRMWARE_CANDIDATE_LAYOUT_INVALID"
        );
        m.layout.application_offset = 0x80000;
        m.flash_capacity_bytes = 4 * 1024 * 1024;
        assert!(run(d.path(), &m).is_err());
    }
    #[test]
    fn manifest_cannot_inject_host_commands_or_status() {
        let (d, m) = fixture();
        let mut value = serde_json::to_value(m).unwrap();
        value["postInstall"] = "anything".into();
        let p = d.path().join("candidate.json");
        std::fs::write(&p, serde_json::to_vec(&value).unwrap()).unwrap();
        assert_eq!(
            inspect_candidate(&p, 7, "board".into()).unwrap_err(),
            "FIRMWARE_CANDIDATE_MANIFEST_INVALID"
        );
    }
    #[test]
    fn absent_and_oversized_metadata_fail_closed() {
        let (d, mut m) = fixture();
        m.loader_source.clear();
        assert!(run(d.path(), &m).is_err());
        let p = d.path().join("large.json");
        std::fs::write(&p, vec![b' '; 65537]).unwrap();
        assert!(inspect_candidate(&p, 7, "board".into()).is_err());
    }
    #[cfg(unix)]
    #[test]
    fn symlinked_payload_is_rejected_even_if_its_hash_matches() {
        let (d, m) = fixture();
        let p = d.path().join(&m.image.file_name);
        std::fs::rename(&p, d.path().join("hidden-image")).unwrap();
        std::os::unix::fs::symlink(d.path().join("hidden-image"), p).unwrap();
        assert!(run(d.path(), &m).is_err());
    }
    #[test]
    fn catalog_is_documentation_only_and_xip_is_not_uf2() {
        let apps = applications().unwrap();
        assert_eq!(apps.len(), 6);
        assert_eq!(
            apps.iter().find(|a| a.id == "nekobox").unwrap().routes,
            vec!["xip"]
        );
        for app in apps {
            assert!(app
                .documentation_url
                .starts_with("https://zephyr.100ask.net/docs/yuzukineko/"));
        }
    }
}
