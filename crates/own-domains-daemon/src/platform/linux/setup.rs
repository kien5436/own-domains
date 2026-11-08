//! Linux host setup and uninstall.
//!
//! The installer maps onto the shared [`HostInstaller`] contract with one
//! receipt file per system change under `<state_dir>/applied/` plus the
//! install manifest (`install-manifest.json`, written last and removed
//! last). Apply performs each mutation then writes its receipt; uninstall
//! reverses from receipts against a fixed artifact allowlist.

use super::super::sidecar::SidecarManifest;
use super::super::windows::setup::CommandRunner;
use super::super::{
    HostInstaller, SetupAction, SetupApplyResult, SetupFailure, SetupPreview, SetupRequest,
    UninstallRequest,
};
use super::firewall::{
    firewalld_remedy, missing_port_rules, parse_firewalld_ports, parse_ufw_status, ufw_remedy,
};
use super::interfaces::is_eligible_interface_name;
use super::manifest::LinuxArtifactContent;
use super::manifest::{build_linux_manifest, render_linux_manifest};
use super::{ip::LinuxInterface, ip::discover_covered_interfaces, nftables::render_nftables};
use own_domains_core::SettingsStore;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

pub const NFTABLES_CONF_PATH: &str = "/etc/own-domains/nftables.conf";
pub const NFTABLES_UNIT_PATH: &str = "/etc/systemd/system/own-domains-nftables.service";
pub const RESOLVER_UNIT_PATH: &str = "/etc/systemd/system/own-domains-resolver.service";
pub const TRUST_CERT_PATH: &str = "/usr/local/share/ca-certificates/own-domains.crt";
pub const INSTALL_MANIFEST_FILE: &str = "install-manifest.json";

const FIREWALL_CMD_BIN: &str = "/usr/bin/firewall-cmd";
const IP_BIN: &str = "/usr/sbin/ip";
const UFW_BIN: &str = "/usr/sbin/ufw";
const NFT_BIN: &str = "/usr/sbin/nft";
const SYSTEMCTL_BIN: &str = "/usr/bin/systemctl";
const UPDATE_CA_BIN: &str = "/usr/sbin/update-ca-certificates";
const CHOWN_BIN: &str = "/usr/bin/chown";
const STAT_BIN: &str = "/usr/bin/stat";
const ID_BIN: &str = "id";

/// Fixed executables setup shells out to. Checked present before any
/// mutation; the list is injectable for tests.
pub const REQUIRED_BINARIES: &[&str] = &[
    "/usr/sbin/ip",
    "/usr/sbin/nft",
    "/usr/bin/resolvectl",
    "/usr/bin/systemctl",
    "/usr/sbin/update-ca-certificates",
    "/usr/bin/openssl",
    "/usr/bin/ss",
];

/// Rendered artifact: receipt value, installed host path, and the replaced
/// bytes for the rollback journal (`None` for brand-new files). `None`
/// overall means the installed file already matches (no side effects).
type RenderedArtifact = Option<(serde_json::Value, PathBuf, Option<Vec<u8>>)>;

/// Journal entry for one committed artifact, driving rollback.
struct JournalEntry {
    receipt: PathBuf,
    host_path: PathBuf,
    backup: Option<Vec<u8>>,
}

pub struct LinuxHostInstaller {
    runner: Box<dyn CommandRunner>,
    /// Staged Caddy used to generate the internal CA. `None` until the CLI
    /// resolves it from the daemon's bundle layout; generation fails with
    /// an actionable message instead of guessing.
    caddy_exe: Option<PathBuf>,
    /// Filesystem root for host artifacts. `None` means the real `/`;
    /// tests point it at a temp dir so no real mutation happens.
    system_root: Option<PathBuf>,
    /// Explicit target UID (tests). Real runs resolve `SUDO_UID`.
    target_uid: Option<u32>,
    /// Explicit target user name (tests). Real runs resolve `SUDO_USER`.
    target_user_name: Option<String>,
    /// Required binaries override (tests). Real runs check the fixed list.
    required_binaries: Option<Vec<String>>,
}

impl LinuxHostInstaller {
    pub fn new() -> Self {
        Self::with_parts(
            Box::new(super::super::windows::setup::SystemCommandRunner),
            None,
        )
    }

    pub fn with_parts(runner: Box<dyn CommandRunner>, caddy_exe: Option<PathBuf>) -> Self {
        Self {
            runner,
            caddy_exe,
            system_root: None,
            target_uid: None,
            target_user_name: None,
            required_binaries: None,
        }
    }

    /// Redirect host artifact paths under a test root. `None` (default)
    /// writes the real system paths and must only run as root on Linux.
    pub fn with_system_root(mut self, root: PathBuf) -> Self {
        self.system_root = Some(root);
        self
    }

    /// Override the target UID (tests). Real runs resolve `SUDO_UID`.
    pub fn with_target_uid(mut self, uid: u32) -> Self {
        self.target_uid = Some(uid);
        self
    }

    /// Override the target user name (tests). Real runs resolve `SUDO_USER`.
    pub fn with_target_user_name(mut self, name: &str) -> Self {
        self.target_user_name = Some(name.to_string());
        self
    }

    /// Override the required binaries (tests). Real runs check the
    /// fixed `REQUIRED_BINARIES` list.
    pub fn with_required_binaries(mut self, binaries: Vec<String>) -> Self {
        self.required_binaries = Some(binaries);
        self
    }

    /// Whether the process runs as root. Fixed `id -u`, no shell.
    fn is_root(&self) -> Result<bool, SetupFailure> {
        let output = self
            .runner
            .run(ID_BIN, &["-u"])
            .map_err(|error| self.failure("unable to determine privilege", error))?;
        Ok(output.trim() == "0")
    }

    /// State directory contract: absolute, no `..`, and — when it already
    /// exists — owned by the target user with no group/world write bits.
    /// A missing directory is created and chowned to the target user.
    fn check_state_dir(&self, state: &Path, user_id: u32) -> Result<(), SetupFailure> {
        if !state.is_absolute() {
            return Err(self.failure(
                "state directory must be absolute",
                state.display().to_string(),
            ));
        }
        if state
            .components()
            .any(|component| component == std::path::Component::ParentDir)
        {
            return Err(self.failure(
                "state directory must not contain `..`",
                state.display().to_string(),
            ));
        }
        if !state.exists() {
            std::fs::create_dir_all(state).map_err(|error| {
                self.failure("unable to create the state directory", error.to_string())
            })?;
            self.runner
                .run(
                    CHOWN_BIN,
                    &[&format!("{user_id}:{user_id}"), &state.to_string_lossy()],
                )
                .map_err(|error| self.failure("unable to own the state directory", error))?;
            return Ok(());
        }
        #[cfg(unix)]
        {
            let mode = std::fs::metadata(state)
                .map_err(|error| {
                    self.failure("unable to stat the state directory", error.to_string())
                })
                .map(|metadata| {
                    use std::os::unix::fs::PermissionsExt;
                    metadata.permissions().mode()
                })?;
            if mode & 0o022 != 0 {
                return Err(self.failure(
                    "state directory must not be group- or world-writable",
                    format!("{} has mode {:o}", state.display(), mode & 0o777),
                ));
            }
        }
        let owner = self
            .runner
            .run(STAT_BIN, &["-c", "%u", &state.to_string_lossy()])
            .map_err(|error| self.failure("unable to stat the state directory", error))?;
        if owner.trim() != user_id.to_string() {
            return Err(self.failure(
                "state directory is not owned by the target user",
                format!("{} is owned by uid {}", state.display(), owner.trim()),
            ));
        }
        Ok(())
    }

    /// UFW with a default-deny incoming policy must explicitly allow the
    /// runtime's high ports. Inactive or absent UFW passes silently; this
    /// check never mutates firewall state.
    fn check_ufw(&self) -> Result<(), SetupFailure> {
        let output = match self.runner.run(UFW_BIN, &["status"]) {
            Ok(output) => output,
            Err(_) => return Ok(()),
        };
        let policy = parse_ufw_status(&output);
        if !policy.enforced {
            return Ok(());
        }
        let missing = missing_port_rules(&policy.allowed);
        if missing.is_empty() {
            return Ok(());
        }
        Err(self.failure(
            "UFW is active with a default-deny incoming policy and blocks the runtime ports",
            ufw_remedy(&missing),
        ))
    }

    /// firewalld when running must explicitly allow the runtime's high
    /// ports in the default zone. Stopped or absent firewalld passes
    /// silently; this check never mutates firewall state.
    fn check_firewalld(&self) -> Result<(), SetupFailure> {
        let running = match self.runner.run(FIREWALL_CMD_BIN, &["--state"]) {
            Ok(output) => output.trim() == "running",
            Err(_) => return Ok(()),
        };
        if !running {
            return Ok(());
        }
        let zone = self
            .runner
            .run(FIREWALL_CMD_BIN, &["--get-default-zone"])
            .map(|output| output.trim().to_string())
            .unwrap_or_default();
        let zone_arg = format!("--zone={zone}");
        let allowed = parse_firewalld_ports(
            &self
                .runner
                .run(FIREWALL_CMD_BIN, &[&zone_arg, "--list-ports"])
                .unwrap_or_default(),
        );
        let missing = missing_port_rules(&allowed);
        if missing.is_empty() {
            return Ok(());
        }
        Err(self.failure(
            "firewalld is running and blocks the runtime ports",
            firewalld_remedy(&zone, &missing),
        ))
    }

    /// Warn (never fail) when the distribution `nftables.service` is
    /// enabled and its file contains `flush ruleset`: a later
    /// `systemctl reload nftables` would delete the Own Domains table.
    /// Setup does not edit `/etc/nftables.conf` or touch the unit.
    fn check_nftables_service(&self) -> Result<Option<String>, SetupFailure> {
        const PATH: &str = "/etc/nftables.conf";
        const UNIT: &str = "nftables.service";
        let host_path = self.host_path(PATH);
        let contents = match std::fs::read_to_string(&host_path) {
            Ok(contents) => contents,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(self.failure(
                    "unable to read the distribution nftables file",
                    format!("{}: {error}", host_path.display()),
                ));
            }
        };
        if !contents.contains("flush ruleset") {
            return Ok(None);
        }
        let enabled = self
            .runner
            .run(
                SYSTEMCTL_BIN,
                &["show", "-p", "UnitFileState", "--value", UNIT],
            )
            .map(|output| output.trim() == "enabled")
            .unwrap_or(false);
        if !enabled {
            return Ok(None);
        }
        Ok(Some(format!(
            "nftablesServiceConflict: {UNIT} is enabled and {PATH} contains `flush ruleset`; reloading it deletes the Own Domains table (not modified by setup)"
        )))
    }

    /// Every fixed executable must exist before any mutation.
    fn check_binaries(&self) -> Result<(), SetupFailure> {
        let required: Vec<String> = self.required_binaries.clone().unwrap_or_else(|| {
            REQUIRED_BINARIES
                .iter()
                .map(|binary| binary.to_string())
                .collect()
        });
        for binary in required {
            if !Path::new(&binary).is_file() {
                return Err(self.failure("required executable is missing", binary.clone()));
            }
        }
        Ok(())
    }

    /// Target user for owner matches and the manifest. Explicit overrides
    /// win (tests); otherwise `SUDO_USER`/`SUDO_UID` must name a nonzero
    /// user (real `sudo` runs; refined in Task 5).
    fn target_user(&self) -> Result<(String, u32), SetupFailure> {
        let name = self
            .target_user_name
            .clone()
            .or_else(|| std::env::var("SUDO_USER").ok());
        let uid = self.target_uid.or_else(|| {
            std::env::var("SUDO_UID")
                .ok()
                .and_then(|value| value.parse::<u32>().ok())
        });
        match (name, uid) {
            (Some(name), Some(uid)) if !name.is_empty() && uid != 0 => Ok((name, uid)),
            _ => Err(SetupFailure::new(
                "unable to resolve the target user (run setup through sudo)",
            )),
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

    fn failure(&self, what: &str, error: String) -> SetupFailure {
        SetupFailure::new(format!("{what}: {error}"))
    }

    fn write_receipt(&self, path: &Path, value: &serde_json::Value) -> Result<(), SetupFailure> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| {
                self.failure("unable to prepare the receipt directory", error.to_string())
            })?;
        }
        let text = serde_json::to_string_pretty(value)
            .map_err(|error| self.failure("unable to encode the receipt", error.to_string()))?;
        std::fs::write(path, text)
            .map_err(|error| self.failure("unable to write the receipt", error.to_string()))
    }

    fn read_receipt(&self, path: &Path) -> Result<serde_json::Value, SetupFailure> {
        let text = std::fs::read_to_string(path)
            .map_err(|error| self.failure("unable to read the receipt", error.to_string()))?;
        serde_json::from_str(&text)
            .map_err(|error| self.failure("unable to parse the receipt", error.to_string()))
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

    /// Map a fixed system path into the test root when one is set.
    fn host_path(&self, path: &str) -> PathBuf {
        match &self.system_root {
            Some(root) => root.join(path.strip_prefix('/').unwrap_or(path)),
            None => PathBuf::from(path),
        }
    }

    /// Write through temp-file + rename so a crash never leaves a half
    /// file, with an explicit Unix mode (host artifacts 0644, the
    /// install manifest 0600). Best-effort modes elsewhere (tests).
    fn atomic_write_mode(
        &self,
        path: &Path,
        contents: &[u8],
        mode: u32,
    ) -> Result<(), SetupFailure> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| {
                self.failure(
                    "unable to prepare the artifact directory",
                    error.to_string(),
                )
            })?;
        }
        let temp = path.with_extension("tmp-new");
        std::fs::write(&temp, contents).map_err(|error| {
            self.failure(
                "unable to write the artifact",
                format!("{}: {error}", temp.display()),
            )
        })?;
        #[cfg(unix)]
        std::fs::set_permissions(&temp, std::os::unix::fs::PermissionsExt::from_mode(mode))
            .map_err(|error| self.failure("unable to set the artifact mode", error.to_string()))?;
        #[cfg(not(unix))]
        let _ = mode;
        std::fs::rename(&temp, path).map_err(|error| {
            self.failure(
                "unable to install the artifact",
                format!("{}: {error}", path.display()),
            )
        })
    }

    /// Host artifact write: root-owned 0644 on Unix.
    fn atomic_write(&self, path: &Path, contents: &[u8]) -> Result<(), SetupFailure> {
        self.atomic_write_mode(path, contents, 0o644)
    }

    /// Eligible interfaces present on the host now, via fixed `ip`
    /// commands (no shell).
    fn covered_interfaces(&self) -> Result<Vec<LinuxInterface>, SetupFailure> {
        let link = self
            .runner
            .run(IP_BIN, &["-j", "link"])
            .map_err(|error| self.failure("interface discovery failed", error))?;
        let addr = self
            .runner
            .run(IP_BIN, &["-j", "addr"])
            .map_err(|error| self.failure("address discovery failed", error))?;
        discover_covered_interfaces(&link, &addr)
            .map_err(|error| self.failure("interface discovery failed", error.to_string()))
    }

    /// The requested interface must be eligible, present, and carry the
    /// requested LAN address as its primary IPv4 address. Address changes
    /// stay unprivileged; only covering a new interface needs setup.
    fn validate_selection(
        &self,
        interfaces: &[LinuxInterface],
        request: &SetupRequest,
    ) -> Result<(), SetupFailure> {
        if !is_eligible_interface_name(&request.interface) {
            return Err(self.failure(
                "selected interface is not eligible for coverage",
                request.interface.clone(),
            ));
        }
        let found = interfaces
            .iter()
            .find(|item| item.name == request.interface);
        let Some(found) = found else {
            return Err(self.failure("selected interface is absent", request.interface.clone()));
        };
        let lan: std::net::Ipv4Addr = request.lan_address.parse().map_err(|_| {
            self.failure(
                "selected LAN address is not valid IPv4",
                request.lan_address.clone(),
            )
        })?;
        if found.primary_ipv4 != Some(lan) {
            return Err(self.failure(
                "selected LAN address is not the primary address of the selected interface",
                format!(
                    "{} is not primary on {}",
                    request.lan_address, request.interface
                ),
            ));
        }
        Ok(())
    }

    /// Per-receipt upsert with content drift detection: `render` gets
    /// whether a receipt already exists and returns `None` when the
    /// installed artifact already matches (no side effects performed),
    /// `Some((value, host path, replaced bytes))` after writing. New
    /// receipts and changed artifacts land in `written` (driving unit
    /// reloads); matches land in `unchanged`.
    fn apply_receipt(
        &self,
        state: &Path,
        journal: &mut Vec<JournalEntry>,
        written: &mut Vec<PathBuf>,
        unchanged: &mut Vec<PathBuf>,
        name: &str,
        render: impl FnOnce(
            bool,
        )
            -> Result<Option<(serde_json::Value, PathBuf, Option<Vec<u8>>)>, String>,
    ) -> Result<(), SetupFailure> {
        let receipt = self.receipt(state, name);
        let exists = receipt.is_file();
        match render(exists)
            .map_err(|error| self.failure(&format!("{name} setup failed"), error))?
        {
            Some((value, host_path, backup)) => {
                self.write_receipt(&receipt, &value)?;
                journal.push(JournalEntry {
                    receipt: receipt.clone(),
                    host_path,
                    backup,
                });
                written.push(receipt);
                Ok(())
            }
            None => {
                unchanged.push(receipt);
                Ok(())
            }
        }
    }

    fn remove_file(&self, path: &Path) -> Result<(), String> {
        match std::fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(format!("unable to remove {}: {error}", path.display())),
        }
    }

    /// Render the ruleset and commit it when it differs from the
    /// installed file. Returns `None` when nothing changed (no `nft`
    /// invocation); otherwise the receipt value, host path, and the
    /// replaced bytes (None for brand-new files) for the journal.
    fn apply_nftables(
        &self,
        receipt_exists: bool,
        interfaces: &[String],
        user_id: u32,
    ) -> Result<RenderedArtifact, String> {
        let ruleset = render_nftables(interfaces, user_id).map_err(|error| error.to_string())?;
        let path = self.host_path(NFTABLES_CONF_PATH);
        if path.is_file() {
            if !receipt_exists {
                return Err(format!(
                    "foreign file at {NFTABLES_CONF_PATH} (no nftables.json receipt): refusing to overwrite"
                ));
            }
            if std::fs::read(&path)
                .map(|bytes| bytes == ruleset.as_bytes())
                .unwrap_or(false)
            {
                return Ok(None);
            }
        }
        let backup = self.backup_existing(NFTABLES_CONF_PATH)?;
        self.atomic_write(&path, ruleset.as_bytes())
            .map_err(|failure| failure.message.clone())?;
        self.runner
            .run(NFT_BIN, &["-c", "-f", &path.to_string_lossy()])
            .inspect_err(|_| {
                let _ = self.remove_file(&path);
            })?;
        Ok(Some((
            serde_json::json!({
                "path": NFTABLES_CONF_PATH,
                "sha256": format!("{:x}", sha2::Sha256::digest(ruleset.as_bytes())),
            }),
            path,
            backup,
        )))
    }

    /// Write a unit file when it differs from the installed one. Same
    /// journal contract as `apply_nftables`.
    fn apply_unit(
        &self,
        receipt_exists: bool,
        receipt_name: &str,
        path: &str,
        content: &str,
    ) -> Result<RenderedArtifact, String> {
        let host_path = self.host_path(path);
        if host_path.is_file() {
            if !receipt_exists {
                return Err(format!(
                    "foreign file at {path} (no {receipt_name} receipt): refusing to overwrite"
                ));
            }
            if std::fs::read(&host_path)
                .map(|bytes| bytes == content.as_bytes())
                .unwrap_or(false)
            {
                return Ok(None);
            }
        }
        let backup = self.backup_existing(path)?;
        self.atomic_write(&host_path, content.as_bytes())
            .map_err(|failure| failure.message.clone())?;
        Ok(Some((
            serde_json::json!({
                "path": path,
                "sha256": format!("{:x}", sha2::Sha256::digest(content.as_bytes())),
            }),
            host_path,
            backup,
        )))
    }

    fn enable_start_unit(&self, name: &str) -> Result<(), String> {
        self.runner.run(SYSTEMCTL_BIN, &["enable", name])?;
        self.runner.run(SYSTEMCTL_BIN, &["restart", name])?;
        Ok(())
    }

    fn daemon_reload(&self) -> Result<(), String> {
        self.runner.run(SYSTEMCTL_BIN, &["daemon-reload"])?;
        Ok(())
    }

    /// Root-owned backup directory for replaced artifacts. Created on
    /// demand; root ownership is natural on the real path (setup runs as
    /// root) and best-effort in tests.
    fn backups_dir(&self) -> Result<PathBuf, SetupFailure> {
        let dir = self.host_path("/etc/own-domains/backups");
        std::fs::create_dir_all(&dir).map_err(|error| {
            self.failure("unable to prepare the backup directory", error.to_string())
        })?;
        #[cfg(unix)]
        std::fs::set_permissions(&dir, std::os::unix::fs::PermissionsExt::from_mode(0o700))
            .map_err(|error| {
                self.failure("unable to set the backup directory mode", error.to_string())
            })?;
        Ok(dir)
    }

    /// Copy the current bytes of an owned artifact into the backup dir
    /// before overwriting. Returns the previous bytes for the rollback
    /// journal (`None` when the file is brand-new).
    fn backup_existing(&self, system_path: &str) -> Result<Option<Vec<u8>>, String> {
        let host_path = self.host_path(system_path);
        if !host_path.is_file() {
            return Ok(None);
        }
        let previous = std::fs::read(&host_path)
            .map_err(|error| format!("unable to read {system_path} for backup: {error}"))?;
        let name = Path::new(system_path)
            .file_name()
            .ok_or_else(|| format!("unable to name a backup for {system_path}"))?;
        let backups = self
            .backups_dir()
            .map_err(|failure| failure.message.clone())?;
        let dest = backups.join(format!("{}.prev", name.to_string_lossy()));
        std::fs::write(&dest, &previous)
            .map_err(|error| format!("unable to write {}: {error}", dest.display()))?;
        #[cfg(unix)]
        std::fs::set_permissions(&dest, std::os::unix::fs::PermissionsExt::from_mode(0o600))
            .map_err(|error| format!("unable to set the backup mode: {error}"))?;
        Ok(Some(previous))
    }

    /// Best-effort rollback of a failed apply: restore backups (or remove
    /// brand-new files) in reverse order, then drop the receipts they
    /// came with. Entries that cannot be undone keep their receipt so a
    /// later uninstall can resume, and land in `unresolved`.
    fn rollback_journal(&self, journal: &[JournalEntry]) -> Vec<String> {
        let mut unresolved = Vec::new();
        for entry in journal.iter().rev() {
            let restored = match &entry.backup {
                Some(previous) => std::fs::write(&entry.host_path, previous).map_err(|error| {
                    format!("unable to restore {}: {error}", entry.host_path.display())
                }),
                None => self.remove_file(&entry.host_path).map_err(|error| {
                    format!("unable to remove {}: {error}", entry.host_path.display())
                }),
            };
            let restored = match restored {
                Ok(()) => true,
                Err(detail) => {
                    unresolved.push(format!("{detail} (receipt kept for uninstall resume)"));
                    false
                }
            };
            if restored {
                if let Err(error) = std::fs::remove_file(&entry.receipt) {
                    unresolved.push(format!(
                        "unable to delete {}: {error}",
                        entry.receipt.display()
                    ));
                }
            } else {
                unresolved.push(format!(
                    "unable to roll back {} (receipt kept for uninstall resume)",
                    entry.host_path.display()
                ));
            }
        }
        unresolved
    }
}

impl Default for LinuxHostInstaller {
    fn default() -> Self {
        Self::new()
    }
}

impl HostInstaller for LinuxHostInstaller {
    fn preview_setup(&self, request: &SetupRequest) -> Result<SetupPreview, SetupFailure> {
        let artifacts = [
            ("linux-nftables", NFTABLES_CONF_PATH),
            ("linux-nftables-unit", NFTABLES_UNIT_PATH),
            ("linux-resolver-unit", RESOLVER_UNIT_PATH),
            ("linux-trust", TRUST_CERT_PATH),
        ];
        let mut actions: Vec<SetupAction> = artifacts
            .iter()
            .map(|(kind, path)| SetupAction {
                kind: kind.to_string(),
                path: PathBuf::from(path),
                backup_path: None,
            })
            .collect();
        actions.push(SetupAction {
            kind: "linux-manifest".into(),
            path: request.state_directory.join(INSTALL_MANIFEST_FILE),
            backup_path: None,
        });
        Ok(SetupPreview { actions })
    }

    fn apply_setup(&self, request: &SetupRequest) -> Result<SetupApplyResult, SetupFailure> {
        // Per-receipt change detection below makes a second identical
        // apply a no-op (every artifact matches); preflight always runs.
        if !self.is_root()? {
            return Err(SetupFailure::new(
                "Linux setup --apply must run as root through sudo",
            ));
        }
        // Target user for the nftables owner match and the manifest.
        let (user_name, user_id) = self.target_user()?;
        self.check_state_dir(&request.state_directory, user_id)?;
        self.check_binaries()?;
        self.check_ufw()?;
        self.check_firewalld()?;
        let discovered = self.covered_interfaces()?;
        self.validate_selection(&discovered, request)?;
        let interfaces: Vec<String> = discovered.into_iter().map(|item| item.name).collect();
        let mut journal: Vec<JournalEntry> = Vec::new();
        let mut written = Vec::new();
        let mut unchanged = Vec::new();
        let outcome = self
            .apply_receipt(
                &request.state_directory,
                &mut journal,
                &mut written,
                &mut unchanged,
                "nftables.json",
                |exists| self.apply_nftables(exists, &interfaces, user_id),
            )
            .and_then(|()| {
                self.apply_receipt(
                    &request.state_directory,
                    &mut journal,
                    &mut written,
                    &mut unchanged,
                    "nftables-unit.json",
                    |exists| {
                        self.apply_unit(
                            exists,
                            "nftables-unit.json",
                            NFTABLES_UNIT_PATH,
                            &super::systemd::render_nftables_unit(),
                        )
                    },
                )
            })
            .and_then(|()| {
                self.apply_receipt(
                    &request.state_directory,
                    &mut journal,
                    &mut written,
                    &mut unchanged,
                    "resolver-unit.json",
                    |exists| {
                        self.apply_unit(
                            exists,
                            "resolver-unit.json",
                            RESOLVER_UNIT_PATH,
                            &super::systemd::render_resolver_unit(),
                        )
                    },
                )
            })
            .and_then(|()| {
                self.apply_receipt(
                    &request.state_directory,
                    &mut journal,
                    &mut written,
                    &mut unchanged,
                    "trust.json",
                    |exists| self.apply_trust(exists, &request.state_directory),
                )
            })
            .and_then(|()| {
                self.write_install_manifest(
                    &request.state_directory,
                    &user_name,
                    user_id,
                    &interfaces,
                )
            })
            .and_then(|()| {
                if written.is_empty() {
                    return Ok(());
                }
                self.daemon_reload()
                    .map_err(|error| self.failure("daemon-reload failed", error))
                    .and_then(|()| {
                        self.enable_start_unit("own-domains-nftables.service")
                            .map_err(|error| self.failure("nftables unit setup failed", error))
                    })
                    .and_then(|()| {
                        self.enable_start_unit("own-domains-resolver.service")
                            .map_err(|error| self.failure("resolver unit setup failed", error))
                    })
            });
        if let Err(mut error) = outcome {
            error.unresolved = self.rollback_journal(&journal);
            return Err(error);
        }
        let mut warnings = Vec::new();
        if let Some(warning) = self.check_nftables_service()? {
            warnings.push(warning);
        }
        // Record the selected LAN address so a later `start` serves it.
        // Receipts make a persist failure retry-safe: re-running apply
        // skips the host mutations as unchanged and retries the write.
        self.persist_lan_address(&request.state_directory, &request.lan_address)?;
        Ok(SetupApplyResult {
            written,
            unchanged,
            backups: Vec::new(),
            warnings,
        })
    }

    fn preview_uninstall(&self, request: &UninstallRequest) -> Result<SetupPreview, SetupFailure> {
        let mut actions = Vec::new();
        // Reverse install order: trust, units, nftables.
        for (kind, receipt) in [
            ("remove-linux-trust", "trust.json"),
            ("remove-linux-resolver-unit", "resolver-unit.json"),
            ("remove-linux-nftables-unit", "nftables-unit.json"),
            ("remove-linux-nftables", "nftables.json"),
        ] {
            let path = self.receipt(&request.state_directory, receipt);
            if path.is_file() {
                actions.push(SetupAction {
                    kind: kind.into(),
                    path,
                    backup_path: None,
                });
            }
        }
        let manifest = request.state_directory.join(INSTALL_MANIFEST_FILE);
        if manifest.is_file() {
            actions.push(SetupAction {
                kind: "remove-linux-manifest".into(),
                path: manifest,
                backup_path: None,
            });
        }
        Ok(SetupPreview { actions })
    }

    fn apply_uninstall(
        &self,
        request: &UninstallRequest,
    ) -> Result<SetupApplyResult, SetupFailure> {
        // Validate the manifest before touching anything: a missing or
        // corrupt manifest refuses with the fixed paths for manual
        // inspection instead of guessing. The one exception is a fully
        // clean state (no receipts, no owned paths): that is an
        // already-uninstalled no-op, mirroring the Windows second-run.
        let manifest_path = request.state_directory.join(INSTALL_MANIFEST_FILE);
        match std::fs::read(&manifest_path) {
            Ok(bytes) => {
                super::manifest::parse_linux_manifest(&bytes).map_err(|error| {
                    self.failure(
                        "install manifest is invalid; inspect or remove by hand",
                        format!("{}: {error}", manifest_path.display()),
                    )
                })?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                if self.uninstall_residue(&request.state_directory) {
                    return Err(self.failure(
                        "install manifest is missing; inspect the fixed paths by hand",
                        format!("{} and /etc/own-domains", manifest_path.display()),
                    ));
                }
                return Ok(SetupApplyResult {
                    written: Vec::new(),
                    unchanged: Vec::new(),
                    backups: Vec::new(),
                    warnings: Vec::new(),
                });
            }
            Err(error) => {
                return Err(self.failure(
                    "unable to read the install manifest",
                    format!("{}: {error}", manifest_path.display()),
                ));
            }
        }
        self.remove_trust_checked(&request.state_directory)?;
        self.remove_resolver_checked(&request.state_directory)?;
        self.remove_nftables_unit_checked(&request.state_directory)?;
        self.remove_nftables_checked(&request.state_directory)?;
        // Manifest last: only after every host artifact is gone.
        std::fs::remove_file(&manifest_path).map_err(|error| {
            self.failure(
                "unable to remove the install manifest",
                format!("{}: {error}", manifest_path.display()),
            )
        })?;
        if request.purge {
            std::fs::remove_dir_all(&request.state_directory).map_err(|error| {
                self.failure("unable to purge the state directory", error.to_string())
            })?;
        }
        Ok(SetupApplyResult {
            written: Vec::new(),
            unchanged: Vec::new(),
            backups: Vec::new(),
            warnings: Vec::new(),
        })
    }
}

impl LinuxHostInstaller {
    /// Any receipt or owned host path still present. Used to tell a
    /// dirty manifest-less state (refuse) from a clean one (no-op).
    fn uninstall_residue(&self, state: &Path) -> bool {
        for name in [
            "nftables.json",
            "nftables-unit.json",
            "resolver-unit.json",
            "trust.json",
        ] {
            if self.receipt(state, name).is_file() {
                return true;
            }
        }
        for path in [
            NFTABLES_CONF_PATH,
            NFTABLES_UNIT_PATH,
            RESOLVER_UNIT_PATH,
            TRUST_CERT_PATH,
        ] {
            if self.host_path(path).is_file() {
                return true;
            }
        }
        false
    }

    fn stop_disable_unit(&self, name: &str) -> Result<(), String> {
        // Stop/disable tolerate absent units (already-removed is success).
        for args in [["stop", name], ["disable", name]] {
            if let Err(error) = self.runner.run(SYSTEMCTL_BIN, &args) {
                let message = error.to_string();
                if !message.contains("not loaded")
                    && !message.contains("No such file")
                    && !message.contains("not-found")
                {
                    return Err(error);
                }
            }
        }
        Ok(())
    }

    fn remove_nftables_checked(&self, state: &Path) -> Result<(), SetupFailure> {
        let receipt = self.receipt(state, "nftables.json");
        if !receipt.is_file() {
            return Ok(());
        }
        let value = self.read_receipt(&receipt)?;
        let path = value
            .get("path")
            .and_then(|value| value.as_str())
            .filter(|path| *path == NFTABLES_CONF_PATH)
            .ok_or_else(|| self.failure("nftables.json receipt is invalid", "".into()))?;
        let host_path = self.host_path(path);
        if host_path.is_file() {
            let expected = value
                .get("sha256")
                .and_then(|value| value.as_str())
                .unwrap_or("");
            let actual = std::fs::read(&host_path)
                .map(|bytes| format!("{:x}", Sha256::digest(&bytes)))
                .unwrap_or_default();
            if !expected.is_empty() && expected != actual {
                return Err(self.failure(
                    "nftables.conf changed since setup; refusing to remove a foreign file",
                    path.into(),
                ));
            }
            self.remove_file(&host_path)
                .map_err(|error| self.failure("nftables.conf removal failed", error))?;
            // Drop the now-empty table; a missing table is already-removed.
            let _ = self
                .runner
                .run(NFT_BIN, &["delete", "table", "ip", "own_domains"]);
        }
        std::fs::remove_file(&receipt)
            .map_err(|error| self.failure("unable to delete the receipt", error.to_string()))
    }

    fn remove_unit_checked(&self, state: &Path, receipt_name: &str) -> Result<(), SetupFailure> {
        let receipt = self.receipt(state, receipt_name);
        if !receipt.is_file() {
            return Ok(());
        }
        let value = self.read_receipt(&receipt)?;
        let unit = if receipt_name.starts_with("resolver") {
            "own-domains-resolver.service"
        } else {
            "own-domains-nftables.service"
        };
        self.stop_disable_unit(unit)
            .map_err(|error| self.failure(&format!("{receipt_name} removal failed"), error))?;
        let path = value
            .get("path")
            .and_then(|value| value.as_str())
            .unwrap_or("");
        if !path.is_empty() {
            let host_path = self.host_path(path);
            if host_path.is_file() {
                self.remove_file(&host_path).map_err(|error| {
                    self.failure(&format!("{receipt_name} removal failed"), error)
                })?;
            }
        }
        std::fs::remove_file(&receipt)
            .map_err(|error| self.failure("unable to delete the receipt", error.to_string()))
    }

    fn remove_nftables_unit_checked(&self, state: &Path) -> Result<(), SetupFailure> {
        self.remove_unit_checked(state, "nftables-unit.json")
    }

    fn remove_resolver_checked(&self, state: &Path) -> Result<(), SetupFailure> {
        self.remove_unit_checked(state, "resolver-unit.json")?;
        // Revert and delete the dummy link when it still exists.
        let _ = self
            .runner
            .run("/usr/bin/resolvectl", &["revert", "own-domains0"]);
        let _ = self.runner.run(IP_BIN, &["link", "delete", "own-domains0"]);
        Ok(())
    }

    fn remove_trust_checked(&self, state: &Path) -> Result<(), SetupFailure> {
        let receipt = self.receipt(state, "trust.json");
        if !receipt.is_file() {
            return Ok(());
        }
        let value = self.read_receipt(&receipt)?;
        self.remove_trust(&value)
            .map_err(|error| self.failure("trust.json removal failed", error))?;
        std::fs::remove_file(&receipt)
            .map_err(|error| self.failure("unable to delete the receipt", error.to_string()))
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

    /// Provision Caddy's internal CA inside the state dir, generating it
    /// with the staged Caddy when absent (mirrors the Windows helper: Caddy
    /// writes the root pair slightly before the intermediates, so
    /// readiness requires all four files).
    fn ensure_caddy_ca(&self, state: &Path) -> Result<PathBuf, String> {
        let files = Self::caddy_ca_files(state);
        if files.iter().all(|path| path.is_file()) {
            return Ok(files[0].clone());
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
                return Ok(files[0].clone());
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

    fn apply_trust(&self, receipt_exists: bool, state: &Path) -> Result<RenderedArtifact, String> {
        let cert = self.ensure_caddy_ca(state)?;
        let pem = std::fs::read(&cert)
            .map_err(|error| format!("unable to read {}: {error}", cert.display()))?;
        let der = super::trust::certificate_der(&pem).map_err(|error| error.to_string())?;
        let fingerprint = super::trust::certificate_sha256(&der);
        let dest = self.host_path(TRUST_CERT_PATH);
        if dest.is_file() {
            if !receipt_exists {
                return Err(format!(
                    "foreign file at {TRUST_CERT_PATH} (no trust.json receipt): refusing to overwrite"
                ));
            }
            if std::fs::read(&dest)
                .map(|bytes| bytes == pem)
                .unwrap_or(false)
            {
                return Ok(None);
            }
        }
        let backup = self.backup_existing(TRUST_CERT_PATH)?;
        self.atomic_write(&dest, &pem)
            .map_err(|failure| failure.message.clone())?;
        self.runner.run(UPDATE_CA_BIN, &[]).inspect_err(|_| {
            let _ = self.remove_file(&dest);
        })?;
        Ok(Some((
            serde_json::json!({
                "path": TRUST_CERT_PATH,
                "sha256": format!("{:x}", Sha256::digest(&pem)),
                "fingerprint": fingerprint,
            }),
            dest,
            backup,
        )))
    }

    /// Staged sidecar versions and digests, read from the `manifest.json`
    /// beside the staged Caddy binary (fetch output layout).
    fn staged_sidecars(&self) -> Result<SidecarManifest, String> {
        let caddy = self.caddy_exe.clone().filter(|exe| exe.is_file()).ok_or_else(|| {
            "Caddy sidecar not found: run setup from the installed layout or beside staged sidecars".to_string()
        })?;
        let manifest = caddy
            .parent()
            .ok_or_else(|| "staged Caddy has no parent directory".to_string())?
            .join("manifest.json");
        let bytes = std::fs::read(&manifest)
            .map_err(|error| format!("unable to read {}: {error}", manifest.display()))?;
        serde_json::from_slice(&bytes)
            .map_err(|error| format!("unable to parse {}: {error}", manifest.display()))
    }

    /// Install manifest, written after every host receipt and removed
    /// after every host artifact on uninstall. Owner-only, target-owned.
    fn write_install_manifest(
        &self,
        state: &Path,
        user_name: &str,
        user_id: u32,
        interfaces: &[String],
    ) -> Result<(), SetupFailure> {
        use super::manifest::LinuxArtifactKind;
        let sidecars = self
            .staged_sidecars()
            .map_err(|error| self.failure("unable to read the staged sidecars", error))?;
        let mut buffers = Vec::new();
        for path in [
            NFTABLES_CONF_PATH,
            NFTABLES_UNIT_PATH,
            RESOLVER_UNIT_PATH,
            TRUST_CERT_PATH,
        ] {
            buffers.push(std::fs::read(self.host_path(path)).map_err(|error| {
                self.failure("unable to hash the installed artifact", error.to_string())
            })?);
        }
        let kinds = [
            LinuxArtifactKind::Nftables,
            LinuxArtifactKind::SystemdUnit,
            LinuxArtifactKind::SystemdUnit,
            LinuxArtifactKind::TrustCertificate,
        ];
        let paths = [
            NFTABLES_CONF_PATH,
            NFTABLES_UNIT_PATH,
            RESOLVER_UNIT_PATH,
            TRUST_CERT_PATH,
        ];
        let contents: Vec<LinuxArtifactContent<'_>> = paths
            .iter()
            .zip(kinds)
            .zip(buffers.iter())
            .map(|((path, kind), content)| LinuxArtifactContent {
                path,
                kind,
                content,
            })
            .collect();
        let ca_pem = std::fs::read(Self::caddy_ca_files(state)[0].clone()).map_err(|error| {
            self.failure("unable to read the CA certificate", error.to_string())
        })?;
        let ca_der = super::trust::certificate_der(&ca_pem).map_err(|error| {
            self.failure("unable to parse the CA certificate", error.to_string())
        })?;
        let manifest = build_linux_manifest(
            user_name,
            user_id,
            interfaces,
            sidecars,
            &super::trust::certificate_sha256(&ca_der),
            &contents,
        );
        let text = render_linux_manifest(&manifest)
            .map_err(|error| self.failure("unable to render the manifest", error.to_string()))?;
        let manifest_path = state.join(INSTALL_MANIFEST_FILE);
        if manifest_path.is_file()
            && std::fs::read(&manifest_path)
                .map(|bytes| bytes == text.as_bytes())
                .unwrap_or(false)
        {
            return Ok(());
        }
        self.atomic_write_mode(&manifest_path, text.as_bytes(), 0o600)?;
        // Target-owned even when root writes it. Best-effort in tests
        // (fake runner); required on the real path.
        let _ = self.runner.run(
            "/usr/bin/chown",
            &[
                &format!("{user_id}:{user_id}"),
                &manifest_path.to_string_lossy(),
            ],
        );
        Ok(())
    }

    fn remove_trust(&self, receipt: &serde_json::Value) -> Result<(), String> {
        let expected = receipt
            .get("fingerprint")
            .and_then(|value| value.as_str())
            .filter(|fingerprint| !fingerprint.is_empty())
            .ok_or_else(|| "trust receipt has no fingerprint".to_string())?;
        let dest = self.host_path(TRUST_CERT_PATH);
        if dest.is_file() {
            let installed = std::fs::read(&dest)
                .map_err(|error| format!("unable to read {}: {error}", dest.display()))?;
            let der =
                super::trust::certificate_der(&installed).map_err(|error| error.to_string())?;
            if super::trust::certificate_sha256(&der) != expected {
                return Err(format!(
                    "caFingerprintMismatch: {} does not match the installed certificate",
                    dest.display()
                ));
            }
            self.remove_file(&dest)?;
            self.runner.run(UPDATE_CA_BIN, &[])?;
        } else {
            // File gone but the bundle may still trust the fingerprint:
            // refresh and recheck (idempotent trust removal).
            self.runner.run(UPDATE_CA_BIN, &[])?;
        }
        Ok(())
    }
}
