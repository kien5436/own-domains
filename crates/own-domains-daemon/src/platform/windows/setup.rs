//! Windows host setup and uninstall.
//!
//! The installer maps onto the shared [`HostInstaller`] contract with one
//! receipt file per system change under `<state_dir>/applied/`
//! (`nrpt.json`, `trust.json`, `firewall.json`). Apply performs the mutation
//! then writes the receipt; uninstall reverses from receipts. The macOS
//! `interface` field is unused here: NRPT routes by namespace, not by
//! service order.

use super::super::{
    HostInstaller, SetupAction, SetupApplyResult, SetupFailure, SetupPreview, SetupRequest,
    UninstallRequest,
};
use own_domains_core::SettingsStore;
use std::path::{Path, PathBuf};

const NRPT_NAMESPACE: &str = ".own";
const NRPT_NAMESERVERS: &str = "127.0.0.1";
const FIREWALL_RULE_NAME: &str = "OwnDomains-LAN";
const CA_FILE_NAME: &str = "ca.crt";
/// Trust receipt schema. v1 held a self-signed cert unrelated to the
/// serving CA; v2 holds the thumbprint of Caddy's own generated root.
const TRUST_SCHEMA_VERSION: u32 = 2;

/// One subprocess invocation. `Ok` carries stdout; `Err` carries the failure
/// text (exit status plus stderr).
pub trait CommandRunner: Send + Sync {
    fn run(&self, program: &str, args: &[&str]) -> Result<String, String>;
    /// Start a long-lived child (Caddy for CA generation). `terminate`
    /// blocks until the child has exited.
    fn spawn(&self, program: &str, args: &[&str]) -> Result<Box<dyn RunningProcess>, String>;
}

/// A child started through [`CommandRunner::spawn`].
pub trait RunningProcess: Send {
    fn is_running(&mut self) -> Result<bool, String>;
    fn terminate(&mut self) -> Result<(), String>;
}

pub struct SystemCommandRunner;

struct SystemChild(std::process::Child);

impl RunningProcess for SystemChild {
    fn is_running(&mut self) -> Result<bool, String> {
        match self.0.try_wait() {
            Ok(None) => Ok(true),
            Ok(Some(_)) => Ok(false),
            Err(error) => Err(format!("unable to poll child process: {error}")),
        }
    }

    fn terminate(&mut self) -> Result<(), String> {
        self.0
            .kill()
            .map_err(|error| format!("unable to terminate child process: {error}"))?;
        let _ = self.0.wait();
        Ok(())
    }
}

impl CommandRunner for SystemCommandRunner {
    fn run(&self, program: &str, args: &[&str]) -> Result<String, String> {
        let output = std::process::Command::new(program)
            .args(args)
            .output()
            .map_err(|error| format!("unable to run {program}: {error}"))?;
        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).into_owned())
        } else {
            Err(format!(
                "{program} failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ))
        }
    }

    fn spawn(&self, program: &str, args: &[&str]) -> Result<Box<dyn RunningProcess>, String> {
        std::process::Command::new(program)
            .args(args)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map(|child| Box::new(SystemChild(child)) as Box<dyn RunningProcess>)
            .map_err(|error| format!("unable to start {program}: {error}"))
    }
}

/// Whether the process runs in the interactive user session. Per-user trust
/// writes fail outside it (SSH sessions get `0x80070032`), so the installer
/// refuses instead of writing to the wrong store.
pub trait SessionProbe: Send + Sync {
    fn is_interactive(&self) -> bool;
}

pub struct SystemSession;

#[cfg(windows)]
impl SessionProbe for SystemSession {
    fn is_interactive(&self) -> bool {
        use windows::Win32::System::Console::GetConsoleWindow;
        unsafe { !GetConsoleWindow().is_invalid() }
    }
}

#[cfg(not(windows))]
impl SessionProbe for SystemSession {
    fn is_interactive(&self) -> bool {
        unimplemented!("Windows-only session probe")
    }
}

pub struct WindowsHostInstaller {
    runner: Box<dyn CommandRunner>,
    session: Box<dyn SessionProbe>,
    /// Staged Caddy used to generate the internal CA. `None` until the CLI
    /// resolves it from the daemon's bundle layout; generation fails with
    /// an actionable message instead of guessing.
    caddy_exe: Option<PathBuf>,
}

impl WindowsHostInstaller {
    pub fn new() -> Self {
        Self::with_parts(Box::new(SystemCommandRunner), Box::new(SystemSession), None)
    }

    pub fn with_parts(
        runner: Box<dyn CommandRunner>,
        session: Box<dyn SessionProbe>,
        caddy_exe: Option<PathBuf>,
    ) -> Self {
        Self {
            runner,
            session,
            caddy_exe,
        }
    }

    /// Override the staged Caddy binary (install layout resolution).
    pub fn with_caddy(mut self, caddy_exe: PathBuf) -> Self {
        self.caddy_exe = Some(caddy_exe);
        self
    }

    fn applied_dir(&self, state: &Path) -> PathBuf {
        state.join("applied")
    }

    fn receipt(&self, state: &Path, name: &str) -> PathBuf {
        self.applied_dir(state).join(name)
    }

    fn require_interactive(&self) -> Result<(), SetupFailure> {
        if self.session.is_interactive() {
            Ok(())
        } else {
            Err(SetupFailure::new(
                "run setup from the interactive console: per-user trust changes are rejected over remote sessions",
            ))
        }
    }

    fn failure(&self, what: &str, error: String) -> SetupFailure {
        SetupFailure::new(format!("{what}: {error}"))
    }

    fn powershell(&self, command: &str) -> Result<String, String> {
        self.runner
            .run("powershell", &["-NoProfile", "-Command", command])
    }

    fn ensure_applied_dir(&self, state: &Path) -> Result<(), SetupFailure> {
        std::fs::create_dir_all(self.applied_dir(state)).map_err(|error| {
            self.failure("unable to prepare the receipt directory", error.to_string())
        })
    }

    /// The auto-assigned name of our NRPT rule, if one routes `.own` today.
    /// `Get-`/`Remove-` key on `-Name` (there is no `-Namespace` filter on
    /// this build), so every lookup filters the full list instead.
    fn nrpt_rule_name(&self) -> Result<Option<String>, String> {
        let output = self.powershell(
            "try { Get-DnsClientNrptRule -ErrorAction Stop | Where-Object { $_.Namespace -contains '.own' } | Select-Object -ExpandProperty Name | Select-Object -First 1 } catch { }",
        )?;
        let name = output.trim().to_string();
        Ok((!name.is_empty()).then_some(name))
    }

    fn nrpt_rule_names(&self) -> Result<Vec<String>, String> {
        let output = self.powershell(
            "try { Get-DnsClientNrptRule -ErrorAction Stop | Select-Object -ExpandProperty Name } catch { }",
        )?;
        Ok(output
            .lines()
            .map(|line| line.trim().to_string())
            .filter(|line| !line.is_empty())
            .collect())
    }

    fn apply_nrpt(&self) -> Result<serde_json::Value, String> {
        if let Some(name) = self.nrpt_rule_name()? {
            return Ok(serde_json::json!({
                "namespace": NRPT_NAMESPACE,
                "nameservers": [NRPT_NAMESERVERS],
                "name": name,
            }));
        }
        self.powershell(&format!(
            "Add-DnsClientNrptRule -Namespace {NRPT_NAMESPACE} -NameServers {NRPT_NAMESERVERS} -DisplayName OwnDomains"
        ))?;
        let name = self
            .nrpt_rule_name()?
            .ok_or_else(|| "NRPT rule was not created".to_string())?;
        Ok(serde_json::json!({
            "namespace": NRPT_NAMESPACE,
            "nameservers": [NRPT_NAMESERVERS],
            "name": name,
        }))
    }

    fn remove_nrpt(&self, receipt: &serde_json::Value) -> Result<(), String> {
        let name = receipt
            .get("name")
            .and_then(|value| value.as_str())
            .filter(|name| !name.is_empty())
            .ok_or_else(|| "NRPT receipt has no name".to_string())?;
        if self.nrpt_rule_names()?.iter().any(|known| known == name) {
            // Names arrive brace-wrapped (`{guid}`), which PowerShell would
            // parse as a script block unquoted.
            let quoted = format!("'{}'", name.replace('\'', "''"));
            self.powershell(&format!("Remove-DnsClientNrptRule -Name {quoted} -Force"))?;
        }
        Ok(())
    }

    fn apply_trust(&self, state: &Path) -> Result<serde_json::Value, String> {
        let generated = !Self::caddy_ca_files(state)
            .iter()
            .all(|path| path.is_file());
        let (cert, _key) = Self::ca_paths(state);
        self.ensure_caddy_ca(state)?;
        // Single-line thumbprint of the file Caddy actually generated: the
        // receipt must hold exactly this, otherwise `certutil -delstore`
        // exits 0 while deleting nothing and the CA leaks on uninstall.
        let output = self.powershell(&format!(
            "(New-Object Security.Cryptography.X509Certificates.X509Certificate2('{}')).Thumbprint",
            cert.display()
        ))?;
        let thumbprint = output.trim().to_string();
        if thumbprint.is_empty() {
            return Err("unable to determine the CA thumbprint".into());
        }
        self.runner.run(
            "certutil",
            &["-addstore", "-user", "Root", &cert.display().to_string()],
        )?;
        // Caddy serves the leaf alone on this platform; without the
        // intermediate in the per-user store, clients cannot complete the
        // chain to the trusted root.
        let intermediate = cert
            .parent()
            .map(|dir| dir.join("intermediate.crt"))
            .ok_or_else(|| "unable to locate the CA intermediate".to_string())?;
        self.runner.run(
            "certutil",
            &[
                "-addstore",
                "-user",
                "CA",
                &intermediate.display().to_string(),
            ],
        )?;
        // Phone/setup-route copy of the trusted root.
        std::fs::copy(&cert, state.join(CA_FILE_NAME)).map_err(|error| {
            format!(
                "unable to publish {}: {error}",
                state.join(CA_FILE_NAME).display()
            )
        })?;
        Ok(serde_json::json!({
            "schemaVersion": TRUST_SCHEMA_VERSION,
            "thumbprint": thumbprint,
            "generated": generated
        }))
    }

    fn remove_trust(&self, state: &Path, receipt: &serde_json::Value) -> Result<(), String> {
        let thumbprint = receipt
            .get("thumbprint")
            .and_then(|value| value.as_str())
            .filter(|thumbprint| !thumbprint.is_empty())
            .ok_or_else(|| "trust receipt has no thumbprint".to_string())?;
        self.runner
            .run("certutil", &["-delstore", "-user", "Root", thumbprint])?;
        let intermediate = state.join("caddy-data/pki/authorities/local/intermediate.crt");
        if intermediate.is_file()
            && let Ok(output) = self.powershell(&format!(
                "(New-Object Security.Cryptography.X509Certificates.X509Certificate2('{}')).Thumbprint",
                intermediate.display()
            ))
        {
            let intermediate_thumbprint = output.trim().to_string();
            if !intermediate_thumbprint.is_empty() {
                let _ = self.runner.run(
                    "certutil",
                    &[
                        "-delstore",
                        "-user",
                        "CA",
                        intermediate_thumbprint.as_str(),
                    ],
                );
            }
        }
        if receipt
            .get("generated")
            .and_then(|value| value.as_bool())
            .unwrap_or(false)
        {
            let cert = state.join(CA_FILE_NAME);
            match std::fs::remove_file(&cert) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(format!("unable to remove {}: {error}", cert.display()));
                }
            }
        }
        Ok(())
    }

    fn apply_firewall(&self, lan: &str) -> Result<serde_json::Value, String> {
        let show = self
            .runner
            .run(
                "netsh",
                &[
                    "advfirewall",
                    "firewall",
                    "show",
                    "rule",
                    &format!("name={FIREWALL_RULE_NAME}"),
                ],
            )
            .unwrap_or_default();
        if show.contains("No rules match") || show.trim().is_empty() {
            self.runner.run(
                "netsh",
                &[
                    "advfirewall",
                    "firewall",
                    "add",
                    "rule",
                    &format!("name={FIREWALL_RULE_NAME}"),
                    "dir=in",
                    "action=allow",
                    "protocol=TCP",
                    "localport=80,443",
                ],
            )?;
        }
        Ok(serde_json::json!({"rule": FIREWALL_RULE_NAME, "ports": [80, 443], "lan": lan}))
    }

    fn remove_firewall(&self) -> Result<(), String> {
        let show = self
            .runner
            .run(
                "netsh",
                &[
                    "advfirewall",
                    "firewall",
                    "show",
                    "rule",
                    &format!("name={FIREWALL_RULE_NAME}"),
                ],
            )
            .unwrap_or_default();
        if !show.contains("No rules match") && !show.trim().is_empty() {
            self.runner.run(
                "netsh",
                &[
                    "advfirewall",
                    "firewall",
                    "delete",
                    "rule",
                    &format!("name={FIREWALL_RULE_NAME}"),
                ],
            )?;
        }
        Ok(())
    }

    fn write_receipt(&self, path: &Path, value: &serde_json::Value) -> Result<(), SetupFailure> {
        let text = serde_json::to_string_pretty(value)
            .map_err(|error| self.failure("unable to encode the receipt", error.to_string()))?;
        std::fs::write(path, text)
            .map_err(|error| self.failure("unable to write the receipt", error.to_string()))
    }

    fn apply_receipt(
        &self,
        state: &Path,
        written: &mut Vec<PathBuf>,
        unchanged: &mut Vec<PathBuf>,
        name: &str,
        mutate: impl FnOnce() -> Result<serde_json::Value, String>,
    ) -> Result<(), SetupFailure> {
        let receipt = self.receipt(state, name);
        if receipt.is_file() {
            unchanged.push(receipt);
            return Ok(());
        }
        let value =
            mutate().map_err(|error| self.failure(&format!("{name} setup failed"), error))?;
        self.write_receipt(&receipt, &value)?;
        written.push(receipt);
        Ok(())
    }

    fn read_receipt(&self, path: &Path) -> Result<serde_json::Value, SetupFailure> {
        let text = std::fs::read_to_string(path)
            .map_err(|error| self.failure("unable to read the receipt", error.to_string()))?;
        serde_json::from_str(&text)
            .map_err(|error| self.failure("unable to parse the receipt", error.to_string()))
    }

    /// Drop a pre-v2 trust receipt so `apply_setup` regenerates from
    /// Caddy's real CA. Returns the superseded thumbprint for best-effort
    /// cleanup; unparseable receipts are dropped without one.
    fn migrate_trust_receipt(&self, state: &Path) -> Result<Option<String>, SetupFailure> {
        let path = self.receipt(state, "trust.json");
        if !path.is_file() {
            return Ok(None);
        }
        let current: Option<serde_json::Value> = std::fs::read_to_string(&path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok());
        let is_current = current
            .as_ref()
            .and_then(|value| value.get("schemaVersion"))
            .and_then(|value| value.as_u64())
            == Some(TRUST_SCHEMA_VERSION as u64);
        if is_current {
            return Ok(None);
        }
        let thumbprint = current
            .as_ref()
            .and_then(|value| value.get("thumbprint"))
            .and_then(|value| value.as_str())
            .filter(|thumbprint| !thumbprint.is_empty())
            .map(str::to_string);
        std::fs::remove_file(&path).map_err(|error| {
            self.failure("unable to migrate the trust receipt", error.to_string())
        })?;
        Ok(thumbprint)
    }

    fn caddy_data_directory(state: &Path) -> PathBuf {
        state.join("caddy-data")
    }

    fn caddy_ca_files(state: &Path) -> [PathBuf; 4] {
        let dir = Self::caddy_data_directory(state).join("pki/authorities/local");
        [
            dir.join("root.crt"),
            dir.join("root.key"),
            dir.join("intermediate.crt"),
            dir.join("intermediate.key"),
        ]
    }

    fn ca_paths(state: &Path) -> (PathBuf, PathBuf) {
        let files = Self::caddy_ca_files(state);
        (files[0].clone(), files[1].clone())
    }

    /// Provision Caddy's internal CA inside the state dir, generating it
    /// with the staged Caddy when absent. The runtime proxy points at the
    /// same storage root, so the trusted root is the one that actually
    /// signs served certificates (a self-signed stand-in would never
    /// validate).
    fn ensure_caddy_ca(&self, state: &Path) -> Result<(), String> {
        // Caddy writes the root pair slightly before the intermediates, so
        // readiness requires all four files — stopping at the root pair
        // leaves a CA that cannot complete chains.
        let files = Self::caddy_ca_files(state);
        if files.iter().all(|path| path.is_file()) {
            return Ok(());
        }
        if files.iter().any(|path| path.is_file()) {
            // Half-written CA: drop and regenerate rather than trusting a
            // mismatched set.
            std::fs::remove_dir_all(Self::caddy_data_directory(state))
                .map_err(|error| format!("unable to clear incomplete CA storage: {error}"))?;
        }
        let caddy = self.caddy_exe.clone().filter(|exe| exe.is_file()).ok_or_else(|| {
            "Caddy sidecar not found: run setup from the installed layout or beside staged sidecars".to_string()
        })?;
        let config_path = state.join("caddy-ca-gen.json");
        let config = serde_json::json!({
            "admin": {"disabled": true},
            "storage": {
                "module": "file_system",
                "root": Self::caddy_data_directory(state).to_string_lossy(),
            },
            "apps": {
                "pki": {"certificate_authorities": {"local": {}}},
                "tls": {"automation": {"policies": [{"subjects": ["setup.own"], "issuers": [{"module": "internal"}]}]}}
            }
        });
        std::fs::write(
            &config_path,
            serde_json::to_vec_pretty(&config).map_err(|error| error.to_string())?,
        )
        .map_err(|error| format!("unable to write {}: {error}", config_path.display()))?;
        let mut child = self.runner.spawn(
            caddy
                .to_str()
                .ok_or_else(|| "Caddy path is not valid Unicode".to_string())?,
            &["run", "--config", &config_path.to_string_lossy()],
        )?;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        loop {
            if files.iter().all(|path| path.is_file()) {
                let _ = child.terminate();
                let _ = std::fs::remove_file(&config_path);
                return Ok(());
            }
            match child.is_running() {
                Ok(true) => {}
                Ok(false) => {
                    let _ = std::fs::remove_file(&config_path);
                    return Err(
                        "Caddy exited before generating the root certificate and key".into(),
                    );
                }
                Err(error) => {
                    let _ = std::fs::remove_file(&config_path);
                    return Err(error);
                }
            }
            if std::time::Instant::now() >= deadline {
                let _ = child.terminate();
                let _ = std::fs::remove_file(&config_path);
                return Err("timed out while generating the Caddy root certificate and key".into());
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }

    fn persist_lan_address(&self, state: &Path, lan_address: &str) -> Result<(), SetupFailure> {
        let store = SettingsStore::new(state);
        let mut settings = store
            .load()
            .map_err(|error| self.failure("unable to load settings", error.to_string()))?;
        settings.lan_address = lan_address.to_string();
        let revision = settings.revision;
        store
            .save(&settings, Some(revision))
            .map_err(|error| self.failure("unable to persist LAN address", error.to_string()))
    }
}

impl Default for WindowsHostInstaller {
    fn default() -> Self {
        Self::new()
    }
}

impl HostInstaller for WindowsHostInstaller {
    fn preview_setup(&self, request: &SetupRequest) -> Result<SetupPreview, SetupFailure> {
        Ok(SetupPreview {
            actions: ["nrpt.json", "trust.json", "firewall.json"]
                .iter()
                .map(|name| SetupAction {
                    kind: format!("windows-{}", name.trim_end_matches(".json")),
                    path: self.receipt(&request.state_directory, name),
                    backup_path: None,
                })
                .collect(),
        })
    }

    fn apply_setup(&self, request: &SetupRequest) -> Result<SetupApplyResult, SetupFailure> {
        self.require_interactive()?;
        self.ensure_applied_dir(&request.state_directory)?;
        // Legacy trust receipts (v1) tracked a self-signed cert that never
        // signed anything; drop the receipt so this apply generates and
        // trusts Caddy's real CA, and best-effort remove the superseded
        // entry afterwards.
        let legacy_thumbprint = self.migrate_trust_receipt(&request.state_directory)?;
        let mut written = Vec::new();
        let mut unchanged = Vec::new();
        self.apply_receipt(
            &request.state_directory,
            &mut written,
            &mut unchanged,
            "nrpt.json",
            || self.apply_nrpt(),
        )?;
        self.apply_receipt(
            &request.state_directory,
            &mut written,
            &mut unchanged,
            "trust.json",
            || self.apply_trust(&request.state_directory),
        )?;
        self.apply_receipt(
            &request.state_directory,
            &mut written,
            &mut unchanged,
            "firewall.json",
            || self.apply_firewall(&request.lan_address),
        )?;
        if let Some(old) = legacy_thumbprint {
            let _ = self
                .runner
                .run("certutil", &["-delstore", "-user", "Root", &old]);
        }
        // Record the selected LAN address so a later `start` serves it.
        // Receipts make a persist failure retry-safe: re-running apply
        // skips the host mutations as unchanged and retries the write.
        self.persist_lan_address(&request.state_directory, &request.lan_address)?;
        Ok(SetupApplyResult {
            written,
            unchanged,
            backups: Vec::new(),
        })
    }

    fn preview_uninstall(&self, request: &UninstallRequest) -> Result<SetupPreview, SetupFailure> {
        let mut actions = Vec::new();
        for name in ["firewall.json", "trust.json", "nrpt.json"] {
            let receipt = self.receipt(&request.state_directory, name);
            if receipt.is_file() {
                actions.push(SetupAction {
                    kind: format!("remove-windows-{}", name.trim_end_matches(".json")),
                    path: receipt,
                    backup_path: None,
                });
            }
        }
        Ok(SetupPreview { actions })
    }

    fn apply_uninstall(
        &self,
        request: &UninstallRequest,
    ) -> Result<SetupApplyResult, SetupFailure> {
        self.require_interactive()?;
        self.remove_firewall_checked(&request.state_directory)?;
        self.remove_trust_checked(&request.state_directory)?;
        self.remove_nrpt_checked(&request.state_directory)?;
        if request.purge {
            std::fs::remove_dir_all(&request.state_directory).map_err(|error| {
                self.failure("unable to purge the state directory", error.to_string())
            })?;
        }
        Ok(SetupApplyResult {
            written: Vec::new(),
            unchanged: Vec::new(),
            backups: Vec::new(),
        })
    }
}

impl WindowsHostInstaller {
    fn remove_firewall_checked(&self, state: &Path) -> Result<(), SetupFailure> {
        let receipt = self.receipt(state, "firewall.json");
        if !receipt.is_file() {
            return Ok(());
        }
        self.remove_firewall()
            .map_err(|error| self.failure("firewall.json removal failed", error))?;
        std::fs::remove_file(&receipt)
            .map_err(|error| self.failure("unable to delete the receipt", error.to_string()))
    }

    fn remove_trust_checked(&self, state: &Path) -> Result<(), SetupFailure> {
        let receipt = self.receipt(state, "trust.json");
        if !receipt.is_file() {
            return Ok(());
        }
        let value = self.read_receipt(&receipt)?;
        self.remove_trust(state, &value)
            .map_err(|error| self.failure("trust.json removal failed", error))?;
        std::fs::remove_file(&receipt)
            .map_err(|error| self.failure("unable to delete the receipt", error.to_string()))
    }

    fn remove_nrpt_checked(&self, state: &Path) -> Result<(), SetupFailure> {
        let receipt = self.receipt(state, "nrpt.json");
        if !receipt.is_file() {
            return Ok(());
        }
        let value = self.read_receipt(&receipt)?;
        self.remove_nrpt(&value)
            .map_err(|error| self.failure("nrpt.json removal failed", error))?;
        std::fs::remove_file(&receipt)
            .map_err(|error| self.failure("unable to delete the receipt", error.to_string()))
    }
}
