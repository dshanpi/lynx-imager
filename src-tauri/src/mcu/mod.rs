pub mod boards;
pub mod commands;
mod cube_programmer;
pub(crate) mod jlink;
pub mod mcp;
mod openocd;
mod pyocd;

#[derive(Default)]
pub struct McuState {
    pub operation: tokio::sync::Mutex<()>,
    pub configuration: tokio::sync::Mutex<()>,
}
