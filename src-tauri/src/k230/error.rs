//! K230 烧录错误类型。

use thiserror::Error;

/// K230 烧录过程中的错误。
///
/// 映射到 `AppError` 返回前端,保持 {code, name, message} 契约。
#[derive(Debug, Error)]
pub enum K230Error {
    /// USB 通信错误(打开/claim/传输失败)
    #[error("USB error: {0}")]
    Usb(String),

    /// 设备未找到(VID/PID 不匹配或已断开)
    #[error("device not found: {0}")]
    DeviceNotFound(String),

    /// 协议错误(握手失败、阶段不匹配、命令响应异常)
    #[error("protocol error: {0}")]
    Protocol(String),

    /// kdimg 镜像解析或校验错误
    #[error("kdimage error: {0}")]
    KdImage(String),

    /// 介质操作错误(擦除/写入/越界)
    #[error("medium error: {0}")]
    Medium(String),

    /// IO 错误(文件读写)
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    /// 操作被取消
    #[error("operation cancelled")]
    Cancelled,

    /// 其他错误
    #[error("{0}")]
    #[allow(dead_code)]
    Other(String),
}
