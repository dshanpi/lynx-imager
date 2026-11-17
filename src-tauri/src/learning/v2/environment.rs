//! Account/release scoped local bindings. Inspection never runs curriculum commands.
use super::{courses, manifest};
use crate::{
    auth::state::AuthState,
    learning::local::LocalLearningState,
    workbench::{self, WorkbenchState},
};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{io::Read, sync::Arc};
use tauri::State;

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EnvironmentRequest {
    pub enrollment_id: String,
    pub release_id: String,
    pub manifest_sha256: String,
    pub profile_id: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BindRequest {
    #[serde(flatten)]
    pub context: EnvironmentRequest,
    pub project_id: i64,
    #[serde(default)]
    pub ssh_alias: String,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvironmentBinding {
    pub project_id: i64,
    pub project_name: String,
    pub host_name: String,
    pub workspace_path: String,
    pub board_uid: String,
    pub board_name: String,
    pub ssh_alias: String,
    pub config_digest: String,
    pub changed: bool,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvironmentView {
    pub profile_id: String,
    pub target: String,
    pub binding: Option<EnvironmentBinding>,
    pub projects: Vec<ProjectChoice>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectChoice {
    pub project_id: i64,
    pub name: String,
    pub host_name: String,
    pub workspace_path: String,
    pub has_board: bool,
}
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Inspection {
    pub commit: String,
    pub branch: String,
    pub dirty: bool,
    pub untracked: bool,
    pub diff_sha256: String,
    pub os: String,
    pub architecture: String,
    pub compiler: String,
    #[serde(default)]
    pub checked_at: String,
    #[serde(default)]
    pub fingerprint: String,
    #[serde(default)]
    pub checks: Vec<VersionResult>,
}
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VersionResult {
    pub component: String,
    pub expected: String,
    pub observed: String,
    pub status: String,
}

pub(crate) fn migrate(db: &Connection) -> rusqlite::Result<()> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS learn_environment_bindings(user_id INTEGER NOT NULL,enrollment_id TEXT NOT NULL,release_id TEXT NOT NULL,manifest_sha256 TEXT NOT NULL,profile_id TEXT NOT NULL,binding TEXT NOT NULL,PRIMARY KEY(user_id,enrollment_id,release_id,manifest_sha256,profile_id));")
}
fn failure(code: &str, text: &str) -> String {
    format!("{code}:{text}")
}
fn valid_alias(value: &str) -> bool {
    !value.starts_with('-')
        && value.len() <= 128
        && !value.is_empty()
        && value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"._-".contains(&c))
}
fn project_path(path: &str) -> bool {
    path.starts_with('/')
        && path.len() <= 4096
        && !path.chars().any(char::is_control)
        && !path.split('/').any(|p| p == "..")
}
pub(super) async fn verified(
    auth: &AuthState,
    request: &EnvironmentRequest,
) -> Result<(i64, manifest::VerifiedCourse), String> {
    let user = auth
        .get_user()
        .await?
        .ok_or_else(|| failure("LEARN_AUTH_REQUIRED", "请先登录"))?
        .id;
    let (base, token) = courses::context(auth)
        .await
        .map_err(|e| failure(e.machine_code, e.message))?;
    let course = courses::course(
        &base,
        &token,
        &request.enrollment_id,
        &request.release_id,
        &request.manifest_sha256,
        &manifest::official_keys().map_err(|e| failure(e.machine_code, e.message))?,
    )
    .await
    .map_err(|e| failure(e.machine_code, e.message))?;
    if !course
        .manifest
        .environments
        .iter()
        .any(|p| p.profile_id == request.profile_id)
    {
        return Err(failure("LEARN_ENVIRONMENT_INVALID", "当前课程没有此环境"));
    }
    if auth.get_user().await?.map(|u| u.id) != Some(user) {
        return Err(failure(
            "LEARN_ACCOUNT_CHANGED",
            "账号已切换，请重新打开课程",
        ));
    }
    Ok((user, course))
}
fn load(
    db: &Connection,
    user: i64,
    r: &EnvironmentRequest,
) -> Result<Option<EnvironmentBinding>, String> {
    let raw:Option<String> = db.query_row("SELECT binding FROM learn_environment_bindings WHERE user_id=?1 AND enrollment_id=?2 AND release_id=?3 AND manifest_sha256=?4 AND profile_id=?5",params![user,r.enrollment_id,r.release_id,r.manifest_sha256,r.profile_id],|row|row.get(0)).optional().map_err(|_|failure("LEARN_STORE_UNAVAILABLE","无法读取环境绑定"))?;
    raw.map(|s| {
        serde_json::from_str(&s)
            .map_err(|_| failure("LEARN_BINDING_INVALID", "环境绑定损坏，请重新绑定"))
    })
    .transpose()
}
fn configuration(
    w: &WorkbenchState,
    id: i64,
    alias: &str,
) -> Result<
    (
        EnvironmentBinding,
        workbench::types::ProjectConfig,
        workbench::types::HostConfig,
    ),
    String,
> {
    let db = w.db.lock().map_err(|_| "WORKBENCH_STORE_UNAVAILABLE")?;
    let p = workbench::storage::project_by_id(&db, id)
        .map_err(|_| failure("LEARN_PROJECT_MISSING", "工程已移除，请重新绑定"))?;
    let h = workbench::storage::host_by_id(&db, p.host_id)
        .map_err(|_| failure("LEARN_HOST_MISSING", "Ubuntu 主机已移除"))?;
    if !project_path(&p.sdk_dir) {
        return Err(failure(
            "LEARN_PROJECT_PATH_INVALID",
            "工程须使用绝对路径，且不能含上级目录",
        ));
    }
    let board = p
        .device_id
        .map(|id| workbench::storage::device_by_id(&db, id))
        .transpose()
        .map_err(|_| failure("LEARN_BOARD_MISSING", "工程绑定的板卡已移除"))?;
    let board_uid = board
        .as_ref()
        .map(|b| b.board_uid.clone())
        .unwrap_or_default();
    let digest = configuration_digest(&p, &h, board.as_ref())?;
    Ok((
        EnvironmentBinding {
            project_id: id,
            project_name: p.name.clone(),
            host_name: h.name.clone(),
            workspace_path: p.sdk_dir.clone(),
            board_uid,
            board_name: board.map(|b| b.name).unwrap_or_default(),
            ssh_alias: alias.into(),
            config_digest: digest,
            changed: false,
        },
        p,
        h,
    ))
}

fn configuration_digest(
    p: &workbench::types::ProjectConfig,
    h: &workbench::types::HostConfig,
    board: Option<&workbench::types::DeviceConfig>,
) -> Result<String, String> {
    let board_uid = board.map(|b| b.board_uid.as_str()).unwrap_or_default();
    let mut identity = serde_json::json!({"project":p,"hostId":h.id,"address":h.address,"port":h.port,"username":h.username,"hostFingerprint":h.host_fingerprint,"boardUid":board_uid});
    // Preserve software-only binding digests. A saved board also pins its full
    // transport/control configuration, not just an unchanged display identity.
    if let Some(device) = board {
        identity["boardConfig"] =
            serde_json::to_value(device).map_err(|_| "LEARN_BINDING_INVALID")?;
    }
    Ok(hex::encode(Sha256::digest(
        serde_json::to_vec(&identity).map_err(|_| "LEARN_BINDING_INVALID")?,
    )))
}

pub(super) fn bound_board(
    local: &LocalLearningState,
    w: &WorkbenchState,
    user: i64,
    request: &EnvironmentRequest,
) -> Result<(EnvironmentBinding, workbench::types::DeviceConfig), String> {
    let (binding, project, host) = bound(local, w, user, request)?;
    let id = project.device_id.ok_or("LEARN_BOARD_REQUIRED")?;
    let device = {
        let db = w.db.lock().map_err(|_| "WORKBENCH_STORE_UNAVAILABLE")?;
        workbench::storage::device_by_id(&db, id).map_err(|_| "LEARN_BOARD_MISSING")?
    };
    if device.board_uid.is_empty()
        || configuration_digest(&project, &host, Some(&device))? != binding.config_digest
    {
        return Err("LEARN_BINDING_CHANGED".into());
    }
    Ok((binding, device))
}

#[tauri::command]
pub async fn learn_get_environment(
    auth: State<'_, Arc<AuthState>>,
    local: State<'_, Arc<LocalLearningState>>,
    workbench: State<'_, Arc<WorkbenchState>>,
    request: EnvironmentRequest,
) -> Result<EnvironmentView, String> {
    let (user, c) = verified(&auth, &request).await?;
    let profile = c
        .manifest
        .environments
        .iter()
        .find(|p| p.profile_id == request.profile_id)
        .ok_or("LEARN_ENVIRONMENT_INVALID")?;
    let saved = {
        let db = local.db.lock().map_err(|_| "LEARN_STORE_UNAVAILABLE")?;
        load(&db, user, &request)?
    };
    let binding = saved.map(|mut b| {
        b.changed = configuration(&workbench, b.project_id, &b.ssh_alias)
            .map(|(v, _, _)| v.config_digest != b.config_digest)
            .unwrap_or(true);
        b
    });
    let db = workbench
        .db
        .lock()
        .map_err(|_| "WORKBENCH_STORE_UNAVAILABLE")?;
    let projects = workbench::storage::list_projects(&db)
        .map_err(|_| "LEARN_PROJECT_LIST_FAILED")?
        .into_iter()
        .filter_map(|p| {
            let host = workbench::storage::host_by_id(&db, p.host_id).ok()?;
            Some(ProjectChoice {
                project_id: p.id?,
                name: p.name,
                host_name: host.name,
                workspace_path: p.sdk_dir,
                has_board: p.device_id.is_some(),
            })
        })
        .collect();
    Ok(EnvironmentView {
        profile_id: request.profile_id,
        target: profile.target.clone(),
        binding,
        projects,
    })
}
#[tauri::command]
pub async fn learn_bind_environment(
    auth: State<'_, Arc<AuthState>>,
    local: State<'_, Arc<LocalLearningState>>,
    workbench: State<'_, Arc<WorkbenchState>>,
    request: BindRequest,
) -> Result<EnvironmentBinding, String> {
    let (user, c) = verified(&auth, &request.context).await?;
    let profile = c
        .manifest
        .environments
        .iter()
        .find(|p| p.profile_id == request.context.profile_id)
        .ok_or("LEARN_ENVIRONMENT_INVALID")?;
    if profile.target == "none"
        || (!request.ssh_alias.is_empty() && !valid_alias(&request.ssh_alias))
    {
        return Err(failure(
            "LEARN_BINDING_INVALID",
            "此活动无需环境，或 SSH 别名无效",
        ));
    }
    let (binding, _, _) = configuration(&workbench, request.project_id, &request.ssh_alias)?;
    if profile.target == "board" && binding.board_uid.is_empty() {
        return Err(failure(
            "LEARN_BOARD_REQUIRED",
            "请先在 Linux 工作台为此工程保存板卡绑定",
        ));
    }
    let db = local.db.lock().map_err(|_| "LEARN_STORE_UNAVAILABLE")?;
    db.execute("INSERT INTO learn_environment_bindings VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(user_id,enrollment_id,release_id,manifest_sha256,profile_id) DO UPDATE SET binding=excluded.binding",params![user,request.context.enrollment_id,request.context.release_id,request.context.manifest_sha256,request.context.profile_id,serde_json::to_string(&binding).map_err(|_|"LEARN_BINDING_INVALID")?]).map_err(|_|"LEARN_STORE_UNAVAILABLE")?;
    Ok(binding)
}
pub(super) fn bound(
    local: &LocalLearningState,
    w: &WorkbenchState,
    user: i64,
    r: &EnvironmentRequest,
) -> Result<
    (
        EnvironmentBinding,
        workbench::types::ProjectConfig,
        workbench::types::HostConfig,
    ),
    String,
> {
    let saved = {
        let db = local.db.lock().map_err(|_| "LEARN_STORE_UNAVAILABLE")?;
        load(&db, user, r)?.ok_or_else(|| failure("LEARN_BINDING_REQUIRED", "请先绑定工程"))?
    };
    let current = configuration(w, saved.project_id, &saved.ssh_alias)?;
    if saved.config_digest != current.0.config_digest {
        return Err(failure(
            "LEARN_BINDING_CHANGED",
            "工程、主机或板卡配置已变化，请重新确认绑定",
        ));
    }
    Ok(current)
}
fn confirm_inspection_binding(
    local: &LocalLearningState,
    w: &WorkbenchState,
    user: i64,
    request: &EnvironmentRequest,
    expected: &str,
) -> Result<(), String> {
    let current = bound(local, w, user, request)?;
    if current.0.config_digest != expected {
        return Err(failure(
            "LEARN_BINDING_CHANGED",
            "检查期间工程绑定已变化，请重新核验",
        ));
    }
    Ok(())
}

// The remote probe is fixed product code, not sourced from a tutorial or model.
const PROBE: &str = r#"import json,subprocess,hashlib,platform,os,signal,select,time

def git(*args):
 p=subprocess.Popen(['git','--no-optional-locks','-c','core.fsmonitor=false',*args],stdout=subprocess.PIPE,stderr=subprocess.DEVNULL,start_new_session=True)
 data=bytearray(); deadline=time.monotonic()+15
 try:
  while True:
   left=deadline-time.monotonic()
   if left<=0: raise RuntimeError('probe timeout')
   if not select.select([p.stdout],[],[],left)[0]: raise RuntimeError('probe timeout')
   part=os.read(p.stdout.fileno(),8192)
   if not part: break
   data.extend(part)
   if len(data)>4194304: raise RuntimeError('probe output too large')
  if p.wait(timeout=max(0.01,deadline-time.monotonic())): raise RuntimeError('git probe failed')
  return bytes(data)
 finally:
  if p.poll() is None: os.killpg(p.pid,signal.SIGKILL)
  p.wait();p.stdout.close()
status=git('status','--porcelain','--untracked-files=normal')
osname=platform.system()
try:
 for line in open('/etc/os-release'):
  if line.startswith('PRETTY_NAME='): osname=line.strip().split('=',1)[1].strip('"')
except FileNotFoundError: pass
compiler=''
try:
 p=subprocess.run(['cc','--version'],capture_output=True,timeout=5)
 if p.returncode==0: compiler=p.stdout.decode('utf-8','replace').splitlines()[0][:512]
except FileNotFoundError: pass
print(json.dumps(dict(commit=git('rev-parse','--verify','HEAD').decode().strip(),branch=git('branch','--show-current').decode().strip(),dirty=bool(status),untracked=any(x.startswith(b'??') for x in status.splitlines()),diffSha256=hashlib.sha256(git('diff','--no-ext-diff','--no-textconv','--binary','HEAD')).hexdigest(),os=osname,architecture=platform.machine(),compiler=compiler)))
"#;
fn inspection_profile<'a>(
    course: &'a manifest::VerifiedCourse,
    request: &EnvironmentRequest,
) -> Result<&'a manifest::EnvironmentProfile, String> {
    let profile = course
        .manifest
        .environments
        .iter()
        .find(|p| p.profile_id == request.profile_id)
        .ok_or("LEARN_ENVIRONMENT_INVALID")?;
    if profile.target != "ubuntu" {
        return Err(failure(
            "LEARN_INSPECT_TARGET_UNSUPPORTED",
            "当前探针仅核验 Ubuntu 工程，不能替代板卡环境验证",
        ));
    }
    Ok(profile)
}

#[tauri::command]
pub async fn learn_inspect_environment(
    auth: State<'_, Arc<AuthState>>,
    local: State<'_, Arc<LocalLearningState>>,
    workbench: State<'_, Arc<WorkbenchState>>,
    request: EnvironmentRequest,
) -> Result<Inspection, String> {
    let (user, c) = verified(&auth, &request).await?;
    let profile = inspection_profile(&c, &request)?;
    let (binding, project, host) = bound(&local, &workbench, user, &request)?;
    let expected_binding = binding.config_digest.clone();
    let credentials = workbench::credentials::resolve(host.id, Default::default())?;
    let mut result = tauri::async_runtime::spawn_blocking(move || -> Result<Inspection, String> {
        let (ssh, _, _) = workbench::ssh::connect(&host, &credentials)?;
        ssh.set_timeout(20_000);
        let mut channel = ssh.channel_session().map_err(|_| "LEARN_INSPECT_CHANNEL")?;
        channel
            .exec(&format!(
                "cd {} && python3 -c {}",
                workbench::ssh::shell_quote(&project.sdk_dir),
                workbench::ssh::shell_quote(PROBE)
            ))
            .map_err(|_| "LEARN_INSPECT_EXEC")?;
        let mut bytes = Vec::new();
        channel
            .by_ref()
            .take(65_537)
            .read_to_end(&mut bytes)
            .map_err(|_| "LEARN_INSPECT_READ")?;
        if bytes.len() > 65_536 {
            return Err("LEARN_INSPECT_OUTPUT_LIMIT".into());
        }
        channel.wait_close().map_err(|_| "LEARN_INSPECT_CLOSE")?;
        if channel.exit_status().map_err(|_| "LEARN_INSPECT_STATUS")? != 0 {
            return Err(failure(
                "LEARN_INSPECT_FAILED",
                "请检查工程 Git、Python 3 与读取权限",
            ));
        }
        let mut result: Inspection =
            serde_json::from_slice(&bytes).map_err(|_| "LEARN_INSPECT_RESPONSE_INVALID")?;
        if ![40, 64].contains(&result.commit.len())
            || !result.commit.bytes().all(|b| b.is_ascii_hexdigit())
            || !manifest::valid_digest(&result.diff_sha256)
        {
            return Err("LEARN_INSPECT_RESPONSE_INVALID".into());
        }
        result.fingerprint = hex::encode(Sha256::digest(format!(
            "{}:{}",
            binding.config_digest,
            String::from_utf8_lossy(&bytes)
        )));
        result.checked_at = chrono::Utc::now().to_rfc3339();
        Ok(result)
    })
    .await
    .map_err(|_| "LEARN_INSPECT_TASK_FAILED")??;
    if auth.get_user().await?.map(|u| u.id) != Some(user) {
        return Err("LEARN_ACCOUNT_CHANGED".into());
    }
    confirm_inspection_binding(&local, &workbench, user, &request, &expected_binding)?;
    result.checks = profile
        .version_checks
        .iter()
        .map(|v| {
            let observed = if v.component == "os" {
                result.os.clone()
            } else {
                String::new()
            };
            let status = if observed.is_empty() || v.operator != "eq" {
                "unverified"
            } else if observed == v.expected {
                "matched"
            } else {
                "mismatch"
            };
            VersionResult {
                component: v.component.clone(),
                expected: v.expected.clone(),
                observed,
                status: status.into(),
            }
        })
        .collect();
    Ok(result)
}

#[tauri::command]
pub async fn learn_open_ide(
    app: tauri::AppHandle,
    auth: State<'_, Arc<AuthState>>,
    local: State<'_, Arc<LocalLearningState>>,
    workbench: State<'_, Arc<WorkbenchState>>,
    request: EnvironmentRequest,
    relative_file: String,
) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    let (user, _) = verified(&auth, &request).await?;
    let (binding, project, host) = bound(&local, &workbench, user, &request)?;
    app.opener()
        .open_url(
            crate::learning::ide::checked_url(
                &host,
                &project.sdk_dir,
                &binding.ssh_alias,
                &relative_file,
            )?,
            None::<String>,
        )
        .map_err(|_| failure("LEARN_IDE_OPEN_FAILED", "请安装 VS Code 与 Remote SSH 扩展"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::learning::ide::{alias_matches, ide_url};
    #[test]
    fn ubuntu_probe_never_attests_board_or_no_environment() {
        let mut c = manifest::tests::verified();
        let r = EnvironmentRequest {
            enrollment_id: c.enrollment_id.clone(),
            release_id: c.manifest.release_id.clone(),
            manifest_sha256: c.manifest_sha256.clone(),
            profile_id: c.manifest.environments[0].profile_id.clone(),
        };
        c.manifest.environments[0].target = "ubuntu".into();
        assert!(inspection_profile(&c, &r).is_ok());
        for target in ["board", "none", "mcu"] {
            c.manifest.environments[0].target = target.into();
            assert!(
                inspection_profile(&c, &r).is_err(),
                "Ubuntu probe accepted {target}"
            );
        }
    }

    #[test]
    fn saved_board_transport_changes_invalidate_learning_binding() {
        let w = WorkbenchState::open(std::path::Path::new(":memory:")).unwrap();
        let local =
            LocalLearningState::open(std::path::Path::new(":memory:"), std::env::temp_dir())
                .unwrap();
        w.db.lock().unwrap().execute_batch("INSERT INTO hosts(id,name,address,port,username,auth_type) VALUES(1,'test','127.0.0.1',22,'test','password');
            INSERT INTO devices(id,name,board_uid) VALUES(1,'test board','linux-test-board');
            INSERT INTO projects(id,name,host_id,device_id,sdk_dir,build_command,firmware_path,local_download_dir) VALUES(1,'test',1,1,'/sdk','','','');").unwrap();
        let r = EnvironmentRequest {
            enrollment_id: "e".into(),
            release_id: "r".into(),
            manifest_sha256: "a".repeat(64),
            profile_id: "board".into(),
        };
        for column in [
            "serial_port",
            "serial_number",
            "adb_serial",
            "usb_binding",
            "camera_binding",
            "power_channel",
        ] {
            let b = configuration(&w, 1, "").unwrap().0;
            local.db.lock().unwrap().execute("INSERT OR REPLACE INTO learn_environment_bindings VALUES(7,'e','r',?1,'board',?2)",params![r.manifest_sha256,serde_json::to_string(&b).unwrap()]).unwrap();
            assert!(bound(&local, &w, 7, &r).is_ok());
            w.db.lock()
                .unwrap()
                .execute(
                    &format!("UPDATE devices SET {column}='changed' WHERE id=1"),
                    [],
                )
                .unwrap();
            assert!(
                bound(&local, &w, 7, &r).is_err(),
                "binding ignored {column}"
            );
            let rebound = configuration(&w, 1, "").unwrap().0;
            local.db.lock().unwrap().execute("INSERT OR REPLACE INTO learn_environment_bindings VALUES(7,'e','r',?1,'board',?2)",params![r.manifest_sha256,serde_json::to_string(&rebound).unwrap()]).unwrap();
            assert!(bound(&local, &w, 7, &r).is_ok());
            assert!(
                confirm_inspection_binding(&local, &w, 7, &r, &b.config_digest).is_err(),
                "late inspection survived rebind of {column}"
            );
        }
    }
    #[test]
    fn binding_key_isolates_account_release_and_environment() {
        let db = Connection::open_in_memory().unwrap();
        migrate(&db).unwrap();
        migrate(&db).unwrap();
        let r = EnvironmentRequest {
            enrollment_id: "e".into(),
            release_id: "r".into(),
            manifest_sha256: "a".repeat(64),
            profile_id: "ubuntu".into(),
        };
        let b = EnvironmentBinding {
            project_id: 1,
            project_name: "p".into(),
            host_name: "h".into(),
            workspace_path: "/sdk".into(),
            board_uid: "".into(),
            board_name: "".into(),
            ssh_alias: "".into(),
            config_digest: "b".repeat(64),
            changed: false,
        };
        db.execute(
            "INSERT INTO learn_environment_bindings VALUES(7,'e','r',?1,'ubuntu',?2)",
            params![r.manifest_sha256, serde_json::to_string(&b).unwrap()],
        )
        .unwrap();
        assert!(load(&db, 7, &r).unwrap().is_some());
        assert!(load(&db, 8, &r).unwrap().is_none());
        let mut other = r.clone();
        other.release_id = "r2".into();
        assert!(load(&db, 7, &other).unwrap().is_none());
        other = r;
        other.profile_id = "board".into();
        assert!(load(&db, 7, &other).unwrap().is_none());
    }
    fn host() -> workbench::types::HostConfig {
        serde_json::from_value(serde_json::json!({"id":1,"name":"Ubuntu","address":"build.example","port":2222,"username":"student","authType":"password","privateKeyPath":"","hostFingerprint":"SHA256:test","defaultWorkDir":"/home/student"})).unwrap()
    }
    #[test]
    fn ide_invalid_explicit_port_never_defaults_to_22() {
        let mut h = host();
        h.port = 22;
        assert!(!alias_matches(
            "Host teaching\n HostName build.example\n User student\n Port invalid\n",
            "teaching",
            &h
        ));
    }

    #[test]
    fn explicit_alias_matches_custom_port_and_refuses_target_drift() {
        let h = host();
        let config = "Host teaching\n HostName build.example\n User student\n Port 2222\n";
        assert!(alias_matches(config, "teaching", &h));
        assert!(!alias_matches(
            &config.replace("2222", "22"),
            "teaching",
            &h
        ));
        assert!(!alias_matches(
            &config.replace("student", "root"),
            "teaching",
            &h
        ));
        assert!(!alias_matches(
            &(config.to_string() + "Host *\n HostName other\n"),
            "teaching",
            &h
        ));
        assert!(!alias_matches(
            &(config.to_string() + "Match exec dangerous\n"),
            "teaching",
            &h
        ));
        assert!(ide_url(&h, "/home/student/sdk project", "", "src/main.c").is_err());
        assert_eq!(
            ide_url(&h, "/home/student/sdk project", "teaching", "src/main.c").unwrap(),
            "vscode://vscode-remote/ssh-remote+teaching/home/student/sdk%20project/src/main.c:1"
        );
        assert!(ide_url(&h, "/sdk", "teaching", "../secrets").is_err());
        assert_eq!(
            ide_url(&h, "/sdk", "teaching", "%2e%2e/secrets").unwrap(),
            "vscode://vscode-remote/ssh-remote+teaching/sdk/%252e%252e/secrets:1"
        );
        assert_eq!(
            ide_url(&h, "/sdk", "teaching", "src/a#b?.c").unwrap(),
            "vscode://vscode-remote/ssh-remote+teaching/sdk/src/a%23b%3F.c:1"
        );
    }
    #[test]
    fn flattened_binding_request_accepts_only_contract_fields() {
        let raw = serde_json::json!({"enrollmentId":"e","releaseId":"r","manifestSha256":"a".repeat(64),"profileId":"ubuntu","projectId":1,"sshAlias":"teaching"});
        let parsed: BindRequest = serde_json::from_value(raw.clone()).unwrap();
        assert_eq!(parsed.context.profile_id, "ubuntu");
        let mut bad = raw;
        bad["command"] = serde_json::json!("dangerous");
        assert!(serde_json::from_value::<BindRequest>(bad).is_err());
    }
    #[test]
    fn paths_and_aliases_reject_control_and_traversal() {
        assert!(project_path("/home/student/sdk project"));
        assert!(!project_path("/home/../etc"));
        assert!(!project_path("/sdk\ncommand"));
        assert!(!valid_alias("-oProxyCommand=bad"));
        assert!(!valid_alias("user@host"));
        assert!(valid_alias("my-ubuntu"));
    }
}
