use serde::{Deserialize, Serialize};
use std::sync::LazyLock;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlatformSpec {
    pub value: String,
    pub label: String,
    pub flasher: String,
    pub protocol: String,
}

static PLATFORM_CATALOG: LazyLock<Vec<PlatformSpec>> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../../resources/workbench-platforms.json"))
        .expect("embedded workbench platform catalog must be valid")
});

pub fn platform_catalog() -> &'static [PlatformSpec] {
    &PLATFORM_CATALOG
}

pub fn platform_spec(value: &str) -> Option<&'static PlatformSpec> {
    PLATFORM_CATALOG.iter().find(|item| item.value == value)
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FlasherDeviceCandidate {
    pub key: String,
    pub flasher_kind: String,
    pub bus: u16,
    pub port: String,
    pub vid: u16,
    pub pid: u16,
    pub mode: String,
    pub chip: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CameraDeviceCandidate {
    pub key: String,
    pub name: String,
    pub description: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TunnelStatus {
    pub project_id: i64,
    pub state: String,
    pub message: String,
    pub local_port: u16,
    pub remote_port: u16,
}

/// Authentication method for an Ubuntu build host.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SshAuthType {
    PrivateKey,
    Password,
}

impl SshAuthType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::PrivateKey => "private_key",
            Self::Password => "password",
        }
    }

    pub fn parse(value: &str) -> Self {
        if value == "password" {
            Self::Password
        } else {
            Self::PrivateKey
        }
    }
}

/// Remote coding-agent CLI available on the Ubuntu host.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AiProvider {
    #[default]
    Codex,
    ClaudeCode,
    DeepseekHarness,
    Custom,
}

impl AiProvider {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::ClaudeCode => "claude_code",
            Self::DeepseekHarness => "deepseek_harness",
            Self::Custom => "custom",
        }
    }

    pub fn parse(value: &str) -> Self {
        match value {
            "claude_code" => Self::ClaudeCode,
            "deepseek_harness" => Self::DeepseekHarness,
            "custom" => Self::Custom,
            _ => Self::Codex,
        }
    }

    pub fn default_executable(&self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::ClaudeCode => "claude",
            Self::DeepseekHarness => "dsh",
            Self::Custom => "",
        }
    }
}

/// Persisted build-host configuration. Secret material is never stored here.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HostConfig {
    pub id: Option<i64>,
    pub name: String,
    pub address: String,
    pub port: u16,
    pub username: String,
    pub auth_type: SshAuthType,
    pub private_key_path: String,
    pub host_fingerprint: String,
    pub default_work_dir: String,
    #[serde(default)]
    pub ai_provider: AiProvider,
    #[serde(default)]
    pub ai_executable: String,
    #[serde(default)]
    pub ai_args_json: String,
}

/// SDK project and its pre-approved build/artifact settings.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectConfig {
    pub id: Option<i64>,
    pub name: String,
    pub host_id: i64,
    #[serde(default)]
    pub device_id: Option<i64>,
    pub sdk_dir: String,
    pub build_command: String,
    pub firmware_path: String,
    pub local_download_dir: String,
    pub verify_sha256: bool,
    #[serde(default = "default_gateway_port")]
    pub local_gateway_port: u16,
    #[serde(default = "default_remote_forward_port")]
    pub remote_forward_port: u16,
    #[serde(default)]
    pub mcp_server_name: String,
    #[serde(default)]
    pub ai_provider: AiProvider,
}

/// Target board configuration used by later serial/power/flash workflow steps.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceConfig {
    pub id: Option<i64>,
    /// Stable identity used by backups and future cross-domain references.
    /// The legacy numeric `id` remains the runtime key during the compatibility phase.
    #[serde(default)]
    pub board_uid: String,
    pub name: String,
    #[serde(default)]
    pub board_image_path: String,
    #[serde(default = "default_chip_vendor")]
    pub chip_vendor: String,
    pub serial_port: String,
    pub serial_vid: u16,
    pub serial_pid: u16,
    pub serial_number: String,
    #[serde(default)]
    pub usb_binding: String,
    #[serde(default)]
    pub camera_binding: String,
    #[serde(default)]
    pub adb_serial: String,
    #[serde(default)]
    pub serial_interface_number: Option<u8>,
    pub baud_rate: u32,
    #[serde(default = "default_serial_console_type")]
    pub serial_console_type: String,
    #[serde(default = "default_serial_login_mode")]
    pub serial_login_mode: String,
    #[serde(default = "default_serial_username")]
    pub serial_login_username: String,
    #[serde(default = "default_serial_login_prompt")]
    pub serial_login_prompt: String,
    #[serde(default = "default_serial_password_prompt")]
    pub serial_password_prompt: String,
    #[serde(default = "default_serial_shell_prompt")]
    pub serial_shell_prompt: String,
    #[serde(default)]
    pub serial_auto_login: bool,
    pub flasher_kind: String,
    pub flasher_path: String,
    pub flasher_args: String,
    pub power_controller: String,
    pub power_controller_vid: u16,
    pub power_controller_pid: u16,
    pub power_controller_serial: String,
    #[serde(default)]
    pub power_controller_interface_number: Option<u8>,
    pub power_baud_rate: u32,
    pub power_profile_json: String,
    pub power_channel: String,
    pub boot_channel: String,
    pub reset_channel: String,
    pub power_off_wait_ms: u64,
    pub power_on_wait_ms: u64,
    pub serial_reconnect_timeout_ms: u64,
    pub boot_timeout_ms: u64,
}

fn default_chip_vendor() -> String {
    "allwinner".to_owned()
}

fn default_serial_console_type() -> String {
    "auto".to_owned()
}

fn default_serial_login_mode() -> String {
    "none".to_owned()
}

fn default_serial_username() -> String {
    "root".to_owned()
}

fn default_serial_login_prompt() -> String {
    "login:".to_owned()
}

fn default_serial_password_prompt() -> String {
    "Password:".to_owned()
}

fn default_serial_shell_prompt() -> String {
    "#".to_owned()
}

fn default_gateway_port() -> u16 {
    8765
}

fn default_remote_forward_port() -> u16 {
    18765
}

/// Actions exposed by every power-controller adapter.
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PowerAction {
    PowerOn,
    PowerOff,
    BootAssert,
    BootRelease,
    ResetPulse,
    GetState,
}

impl PowerAction {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::PowerOn => "power_on",
            Self::PowerOff => "power_off",
            Self::BootAssert => "boot_assert",
            Self::BootRelease => "boot_release",
            Self::ResetPulse => "reset_pulse",
            Self::GetState => "get_state",
        }
    }
}

/// One newline-oriented ASCII CDC protocol profile.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AsciiCdcProfile {
    #[serde(default = "default_line_ending")]
    pub line_ending: String,
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
    #[serde(default)]
    pub power_on: String,
    #[serde(default)]
    pub power_off: String,
    #[serde(default)]
    pub boot_assert: String,
    #[serde(default)]
    pub boot_release: String,
    #[serde(default)]
    pub reset_pulse: String,
    #[serde(default)]
    pub get_state: String,
    #[serde(default = "default_success_pattern")]
    pub success_pattern: String,
    #[serde(default)]
    pub failure_pattern: String,
}

fn default_line_ending() -> String {
    "\\n".to_owned()
}

fn default_timeout_ms() -> u64 {
    1500
}

fn default_success_pattern() -> String {
    "(?i)\\b(ok|success)\\b".to_owned()
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskHistory {
    pub task_id: String,
    pub kind: String,
    pub project_id: Option<i64>,
    pub device_id: Option<i64>,
    pub status: String,
    pub started_at: i64,
    pub finished_at: Option<i64>,
    pub exit_code: Option<i32>,
    pub error: String,
    pub phase: String,
    pub progress_pct: Option<f64>,
    pub details: Option<serde_json::Value>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowStepHistory {
    pub id: i64,
    pub task_id: String,
    pub step_name: String,
    pub status: String,
    pub started_at: i64,
    pub finished_at: Option<i64>,
    pub error: String,
}

/// Ephemeral authentication data supplied for one operation.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SshCredentials {
    pub password: Option<String>,
    pub key_passphrase: Option<String>,
}

impl SshCredentials {
    pub fn is_empty(&self) -> bool {
        self.password.as_deref().unwrap_or_default().is_empty()
            && self
                .key_passphrase
                .as_deref()
                .unwrap_or_default()
                .is_empty()
    }
}

/// Structured event emitted for build and transfer tasks.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskEvent {
    pub task_id: String,
    pub phase: String,
    pub stream: String,
    pub message: String,
    pub current: Option<u64>,
    pub total: Option<u64>,
    pub status: String,
}

/// Result of an SSH connection test.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SshTestResult {
    pub fingerprint: String,
    pub trusted: bool,
    pub audit_session_id: String,
}

/// Result of probing a coding-agent CLI over the configured SSH connection.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiChannelTestResult {
    pub provider: String,
    pub executable: String,
    pub version: String,
    pub audit_session_id: String,
}
