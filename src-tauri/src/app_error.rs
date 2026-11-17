use serde::{Deserialize, Serialize};

/// Forward-compatible IPC error shape. Existing commands keep serializing their
/// legacy error values; new or migrated boundaries can add stable metadata
/// without changing `code`, `name`, or `message`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct IpcErrorContract {
    pub code: i32,
    pub name: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub machine_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retryable: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Map<String, serde_json::Value>>,
}

impl IpcErrorContract {
    pub fn from_legacy(code: i32, name: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code,
            name: name.into(),
            message: message.into(),
            machine_code: None,
            retryable: None,
            details: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppError {
    pub code: i32,
    pub name: String,
    pub message: String,
}

impl AppError {
    pub fn internal(message: impl Into<String>) -> Self {
        Self {
            code: -1,
            name: "InternalError".to_string(),
            message: message.into(),
        }
    }
}

impl From<String> for AppError {
    fn from(message: String) -> Self {
        Self {
            code: -1,
            name: "OperationFailed".to_string(),
            message,
        }
    }
}

impl From<&str> for AppError {
    fn from(message: &str) -> Self {
        Self::from(message.to_string())
    }
}

impl From<crate::efex::error::EfexError> for AppError {
    fn from(value: crate::efex::error::EfexError) -> Self {
        Self {
            code: value.code,
            name: value.name,
            message: value.message,
        }
    }
}

impl From<AppError> for IpcErrorContract {
    fn from(value: AppError) -> Self {
        Self::from_legacy(value.code, value.name, value.message)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_error_fields_remain_exact_and_extension_fields_are_optional() {
        let contract = IpcErrorContract::from(AppError::from("same visible message"));
        assert_eq!(contract.code, -1);
        assert_eq!(contract.name, "OperationFailed");
        assert_eq!(contract.message, "same visible message");
        assert_eq!(
            serde_json::to_value(contract).unwrap(),
            serde_json::json!({
                "code": -1,
                "name": "OperationFailed",
                "message": "same visible message"
            })
        );
    }
}
