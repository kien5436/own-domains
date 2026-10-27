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
use std::path::{Path, PathBuf};

const NRPT_NAMESPACE: &str = ".own";
const NRPT_NAMESERVERS: &str = "127.0.0.1";
const FIREWALL_RULE_NAME: &str = "OwnDomains-LAN";
const CA_FILE_NAME: &str = "ca.crt";
const CA_SUBJECT: &str = "CN=OwnDomains Local CA";

/// One subprocess invocation. `Ok` carries stdout; `Err` carries the failure
/// text (exit status plus stderr).
pub trait CommandRunner: Send + Sync {
    fn run(&self, program: &str, args: &[&str]) -> Result<String, String>;
}

pub struct SystemCommandRunner;

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
}

impl WindowsHostInstaller {
    pub fn new() -> Self {
        Self::with_parts(Box::new(SystemCommandRunner), Box::new(SystemSession))
    }

    pub fn with_parts(runner: Box<dyn CommandRunner>, session: Box<dyn SessionProbe>) -> Self {
        Self { runner, session }
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
        let cert = state.join(CA_FILE_NAME);
        let (thumbprint, generated) = if cert.is_file() {
            let output = self.powershell(&format!(
                "(New-Object Security.Cryptography.X509Certificates.X509Certificate2('{}')).Thumbprint",
                cert.display()
            ))?;
            (output.trim().to_string(), false)
        } else {
            let output = self.powershell(&format!(
                "$cert = New-SelfSignedCertificate -Subject '{CA_SUBJECT}' -CertStoreLocation Cert:\\CurrentUser\\My -KeyExportPolicy Exportable -KeyUsage CertSign,CRLSign,DigitalSignature -Type Custom; Export-Certificate -Cert $cert -FilePath '{}'; $cert.Thumbprint",
                cert.display()
            ))?;
            (output.trim().to_string(), true)
        };
        if thumbprint.is_empty() {
            return Err("unable to determine the CA thumbprint".into());
        }
        self.runner.run(
            "certutil",
            &["-addstore", "-user", "Root", &cert.display().to_string()],
        )?;
        Ok(serde_json::json!({"thumbprint": thumbprint, "generated": generated}))
    }

    fn remove_trust(&self, state: &Path, receipt: &serde_json::Value) -> Result<(), String> {
        let thumbprint = receipt
            .get("thumbprint")
            .and_then(|value| value.as_str())
            .filter(|thumbprint| !thumbprint.is_empty())
            .ok_or_else(|| "trust receipt has no thumbprint".to_string())?;
        self.runner
            .run("certutil", &["-delstore", "-user", "Root", thumbprint])?;
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
