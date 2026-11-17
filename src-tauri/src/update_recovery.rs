use serde::Serialize;
use std::fs;
use tauri::Manager;

use crate::IpcErrorContract;

const FAILURE_MARKER: &str = "update-delta-failure";
const MAX_MARKER_BYTES: u64 = 128;

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DeltaUpdateFailure {
    pub code: String,
    pub message: String,
}

fn parse_failure_code(value: &str) -> Option<DeltaUpdateFailure> {
    let code = value.trim();
    let message = match code {
        "PRECONDITION" => "当前安装版本或安装目录与增量包不匹配",
        "COPY" => "复制当前程序文件失败，原版本未被替换",
        "PATCH" => "增量补丁校验或应用失败，原版本未被替换",
        "SELF_TEST" => "新版本自检失败，原版本未被替换",
        _ => return None,
    };
    Some(DeltaUpdateFailure {
        code: code.to_string(),
        message: message.to_string(),
    })
}

#[tauri::command]
pub fn consume_update_delta_failure(
    app: tauri::AppHandle,
) -> Result<Option<DeltaUpdateFailure>, IpcErrorContract> {
    let data_dir = app.path().app_data_dir().map_err(|error| {
        IpcErrorContract::from_legacy(-1, "UpdateRecoveryPathError", error.to_string())
    })?;
    let marker = data_dir.join(FAILURE_MARKER);
    let metadata = match fs::symlink_metadata(&marker) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(IpcErrorContract::from_legacy(
                -1,
                "UpdateRecoveryReadError",
                error.to_string(),
            ))
        }
    };
    if !metadata.file_type().is_file()
        || metadata.file_type().is_symlink()
        || metadata.len() > MAX_MARKER_BYTES
    {
        let _ = fs::remove_file(&marker);
        return Err(IpcErrorContract::from_legacy(
            -1,
            "UpdateRecoveryMarkerInvalid",
            "增量更新恢复标记无效",
        ));
    }
    let value = fs::read_to_string(&marker).map_err(|error| {
        IpcErrorContract::from_legacy(-1, "UpdateRecoveryReadError", error.to_string())
    })?;
    fs::remove_file(&marker).map_err(|error| {
        IpcErrorContract::from_legacy(-1, "UpdateRecoveryConsumeError", error.to_string())
    })?;
    parse_failure_code(&value).map(Some).ok_or_else(|| {
        IpcErrorContract::from_legacy(
            -1,
            "UpdateRecoveryMarkerInvalid",
            "增量更新恢复标记包含未知状态",
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_only_fixed_non_sensitive_failure_codes() {
        let failure = parse_failure_code("PATCH\r\n").unwrap();
        assert_eq!(failure.code, "PATCH");
        assert!(failure.message.contains("原版本未被替换"));
        assert!(parse_failure_code("../../secret").is_none());
        assert!(parse_failure_code("arbitrary details").is_none());
    }
}
