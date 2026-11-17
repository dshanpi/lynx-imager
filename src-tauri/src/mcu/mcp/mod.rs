//! MCU-only MCP channel.
//!
//! This domain deliberately does not depend on the Linux/SDK MCP router.  It
//! exposes only board-scoped power, debug-probe, target-control and UART operations.

pub(crate) mod gateway;
mod ssh_transport;
mod tools;
pub(crate) mod tunnel;

pub use gateway::McuMcpGatewayState;
pub use tunnel::McuMcpTunnelState;

#[cfg(test)]
mod architecture_tests {
    #[test]
    fn mcu_mcp_domain_does_not_import_linux_domains() {
        let sources = [
            include_str!("tools.rs"),
            include_str!("gateway.rs"),
            include_str!("ssh_transport.rs"),
            include_str!("tunnel.rs"),
        ];
        for source in sources {
            assert!(!source.contains("crate::workbench"));
            assert!(!source.contains("crate::gateway"));
            assert!(!source.contains("crate::adb"));
        }
    }

    #[test]
    fn linux_mcp_domain_does_not_import_mcu_domain() {
        let sources = [
            include_str!("../../gateway.rs"),
            include_str!("../../workbench/commands.rs"),
            include_str!("../../workbench/tunnel.rs"),
        ];
        for source in sources {
            assert!(!source.contains("crate::mcu"));
            assert!(!source.contains("lynx_mcu_"));
            assert!(!source.contains("mcu_mcp_"));
        }
    }
}
