//! Shared runtime orchestration pieces for Unix adapters.
//!
//! Readiness kinds, the readiness/setup-server contracts, the setup server
//! itself, and the adapter bundle. The start/reconcile/stop/inspect
//! orchestration moves here once the shared runtime paths land.

use std::{net::SocketAddr, path::PathBuf, sync::Mutex};

use own_domains_core::Settings;

use crate::SetupServer;
use crate::platform::HostInspector;
use crate::probe::CaddyAdmin;

use super::{
    config::{ConfigurationFiles, ConfigurationValidator},
    process::{ProcessHost, ProcessRecords, Waiter},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReadinessKind {
    Dns,
    CaddyAdmin,
    Setup,
    Http,
}

impl ReadinessKind {
    pub fn component(self) -> &'static str {
        match self {
            Self::Dns => "CoreDNS",
            Self::CaddyAdmin => "Caddy admin",
            Self::Setup => "setup server",
            Self::Http => "Caddy HTTP",
        }
    }
}

pub trait RuntimeReadiness: Send + Sync {
    fn check(&self, kind: ReadinessKind, settings: &Settings) -> Result<(), String>;
}

pub trait SetupServerControl: Send + Sync {
    fn start(&self, address: SocketAddr, certificate: PathBuf) -> Result<(), String>;
    fn stop(&self) -> Result<(), String>;
    fn is_running(&self) -> bool;
}

pub struct SystemSetupServer {
    server: Mutex<Option<SetupServer>>,
}

impl SystemSetupServer {
    pub fn new() -> Self {
        Self {
            server: Mutex::new(None),
        }
    }
}

impl Default for SystemSetupServer {
    fn default() -> Self {
        Self::new()
    }
}

impl SetupServerControl for SystemSetupServer {
    fn start(&self, address: SocketAddr, certificate: PathBuf) -> Result<(), String> {
        let mut server = self.server.lock().unwrap();
        if server.is_some() {
            return Ok(());
        }
        let bound = SetupServer::bind(address, certificate)
            .map_err(|error| format!("unable to start the setup server: {error}"))?;
        *server = Some(bound);
        Ok(())
    }

    fn stop(&self) -> Result<(), String> {
        let mut guard = self.server.lock().unwrap();
        if let Some(server) = guard.as_ref() {
            server
                .shutdown()
                .map_err(|error| format!("unable to stop the setup server: {error}"))?;
        }
        *guard = None;
        Ok(())
    }

    fn is_running(&self) -> bool {
        self.server.lock().unwrap().is_some()
    }
}

#[derive(Clone, Copy)]
pub struct RuntimeAdapters<'a> {
    pub inspector: &'a dyn HostInspector,
    pub files: &'a dyn ConfigurationFiles,
    pub validator: &'a dyn ConfigurationValidator,
    pub process_host: &'a dyn ProcessHost,
    pub records: &'a dyn ProcessRecords,
    pub waiter: &'a dyn Waiter,
    pub caddy: &'a dyn CaddyAdmin,
    pub setup: &'a dyn SetupServerControl,
    pub readiness: &'a dyn RuntimeReadiness,
}
