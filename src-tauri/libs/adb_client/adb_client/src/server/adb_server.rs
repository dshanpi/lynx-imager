use crate::ADBTransport;
use crate::Result;
use crate::RustADBError;
use crate::server::tcp_server_transport::TCPServerTransport;
use std::collections::HashMap;
use std::net::SocketAddrV4;
use std::process::Command;

/// Represents an ADB Server
#[derive(Debug, Default)]
pub struct ADBServer {
    /// Internal [`TcpStream`], lazily initialized
    pub(crate) transport: Option<TCPServerTransport>,
    /// Address to connect to
    pub(crate) socket_addr: Option<SocketAddrV4>,
    /// adb-server start envs
    pub(crate) envs: HashMap<String, String>,
    /// Path to adb binary
    /// If not set, will use adb from PATH
    pub(crate) adb_path: Option<String>,
    /// Passive clients must not change the shared server lifecycle.
    connect_existing_only: bool,
}

impl ADBServer {
    /// Instantiates a new [`ADBServer`]
    #[must_use]
    pub fn new(address: SocketAddrV4) -> Self {
        Self {
            transport: None,
            socket_addr: Some(address),
            envs: HashMap::new(),
            adb_path: None,
            connect_existing_only: false,
        }
    }

    /// Connect only to an existing server, without launching an ADB executable.
    /// Connection failures are returned unchanged; callers own server lifecycle.
    #[must_use]
    pub fn new_existing(address: SocketAddrV4) -> Self {
        let mut server = Self::new(address);
        server.connect_existing_only = true;
        server
    }

    /// Instantiates a new [`ADBServer`] with a custom adb path
    #[must_use]
    pub fn new_from_path(address: SocketAddrV4, adb_path: Option<String>) -> Self {
        Self {
            transport: None,
            socket_addr: Some(address),
            envs: HashMap::new(),
            adb_path,
            connect_existing_only: false,
        }
    }

    /// Get the server address
    #[must_use]
    pub const fn socket_addr(&self) -> Option<SocketAddrV4> {
        self.socket_addr
    }

    /// Start an instance of `adb-server`
    pub fn start(envs: &HashMap<String, String>, adb_path: &Option<String>) {
        // ADB Server is local, we start it if not already running
        let mut command = Command::new(adb_path.as_deref().unwrap_or("adb"));
        command.arg("start-server");
        for (env_k, env_v) in envs {
            command.env(env_k, env_v);
        }

        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt;
            // Do not show a prompt on Windows
            command.creation_flags(0x08000000);
        }

        let child = command.spawn();
        match child {
            Ok(mut child) => {
                if let Err(e) = child.wait() {
                    log::error!("error while starting adb server: {e}");
                }
            }
            Err(e) => log::error!("error while starting adb server: {e}"),
        }
    }

    /// Returns the current selected transport
    pub(crate) fn get_transport(&mut self) -> Result<&mut TCPServerTransport> {
        self.transport
            .as_mut()
            .ok_or(RustADBError::IOError(std::io::Error::new(
                std::io::ErrorKind::NotConnected,
                "server connection not initialized",
            )))
    }

    /// Connect to underlying transport
    pub(crate) fn connect(&mut self) -> Result<&mut TCPServerTransport> {
        self.connect_with_starter(Self::start)
    }

    fn connect_with_starter(
        &mut self,
        starter: impl FnOnce(&HashMap<String, String>, &Option<String>),
    ) -> Result<&mut TCPServerTransport> {
        let mut is_local_ip = false;
        let mut transport = if let Some(addr) = &self.socket_addr {
            let ip = addr.ip();
            if ip.is_loopback() || ip.is_unspecified() {
                is_local_ip = true;
            }
            TCPServerTransport::new(*addr)
        } else {
            is_local_ip = true;
            TCPServerTransport::default()
        };

        if is_local_ip && !self.connect_existing_only {
            starter(&self.envs, &self.adb_path);
        }

        transport.connect()?;
        self.transport = Some(transport);

        self.get_transport()
    }
}

impl Drop for ADBServer {
    fn drop(&mut self) {
        if let Some(transport) = &mut self.transport {
            let _ = transport.disconnect();
        }
    }
}

#[cfg(test)]
mod passive_tests {
    use super::*;
    use std::{cell::Cell, net::TcpListener};

    #[test]
    fn passive_existing_server_never_calls_starter() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = match listener.local_addr().unwrap() {
            std::net::SocketAddr::V4(address) => address,
            _ => unreachable!(),
        };
        let mut server = ADBServer::new_existing(address);
        server
            .connect_with_starter(|_, _| panic!("passive probe started adb"))
            .unwrap();
    }

    #[test]
    fn passive_refused_connection_never_calls_starter() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = match listener.local_addr().unwrap() {
            std::net::SocketAddr::V4(address) => address,
            _ => unreachable!(),
        };
        drop(listener);
        let mut server = ADBServer::new_existing(address);
        assert!(
            server
                .connect_with_starter(|_, _| panic!("failed probe started adb"))
                .is_err()
        );
    }

    #[test]
    fn legacy_constructor_preserves_one_start_call() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = match listener.local_addr().unwrap() {
            std::net::SocketAddr::V4(address) => address,
            _ => unreachable!(),
        };
        let calls = Cell::new(0);
        let mut server = ADBServer::new(address);
        server
            .connect_with_starter(|_, _| calls.set(calls.get() + 1))
            .unwrap();
        assert_eq!(calls.get(), 1);
        assert!(!ADBServer::default().connect_existing_only);
        assert!(!ADBServer::new_from_path(address, None).connect_existing_only);
    }
}
