use super::{
    CommandRunner, HostArtifact, HostCommand, HostCommandOutput, HostFileKind, HostInstallManifest,
    HostSetupPlan, RuntimePaths,
};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    time::Duration,
};

const PF_CONF: &str = "/etc/pf.conf";
const PF_ANCHOR: &str = "/etc/pf.anchors/dev.owndomains";
const BEGIN_MARKER: &str = "# BEGIN OWN DOMAINS\n";
const END_MARKER: &str = "# END OWN DOMAINS\n";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SetupFile {
    pub content: Vec<u8>,
    pub mode: u32,
    pub owner_id: u32,
    pub kind: HostFileKind,
}

pub trait SetupHost: Send + Sync {
    fn inspect_file(&self, path: &Path) -> Result<Option<SetupFile>, String>;
    fn atomic_write(&self, path: &Path, file: &SetupFile) -> Result<(), String>;
    fn write_backup(&self, path: &Path, file: &SetupFile) -> Result<(), String> {
        self.atomic_write(path, file)
    }
    fn remove_file(&self, path: &Path) -> Result<(), String>;
    fn remove_tree(&self, path: &Path) -> Result<(), String>;
    fn run(&self, command: &HostCommand) -> Result<HostCommandOutput, String>;
    fn certificate_trusted(&self, certificate: &Path) -> Result<bool, String>;
    fn pf_enabled(&self) -> Result<bool, String>;
    fn final_probe(&self) -> Result<(), String>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SetupAction {
    pub kind: String,
    pub path: PathBuf,
    pub backup_path: Option<PathBuf>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SetupPreview {
    pub actions: Vec<SetupAction>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SetupApplyResult {
    pub written: Vec<PathBuf>,
    pub unchanged: Vec<PathBuf>,
    pub backups: Vec<PathBuf>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SetupFailure {
    pub message: String,
    pub unresolved: Vec<String>,
}

impl std::fmt::Display for SetupFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}", self.message)
    }
}

impl std::error::Error for SetupFailure {}

struct PlannedArtifact<'a> {
    artifact: &'a HostArtifact,
    desired: SetupFile,
    previous: Option<SetupFile>,
    backup_path: Option<PathBuf>,
}

pub fn preview_setup(
    plan: &HostSetupPlan,
    host: &dyn SetupHost,
) -> Result<SetupPreview, SetupFailure> {
    let planned = plan_setup(plan, host)?;
    Ok(SetupPreview {
        actions: planned
            .into_iter()
            .map(|item| SetupAction {
                kind: if item.previous.as_ref() == Some(&item.desired) {
                    "unchanged"
                } else if item.previous.is_some() {
                    "update"
                } else {
                    "create"
                }
                .into(),
                path: item.artifact.path.clone(),
                backup_path: item.backup_path,
            })
            .collect(),
    })
}

pub fn apply_setup(
    plan: &HostSetupPlan,
    host: &dyn SetupHost,
) -> Result<SetupApplyResult, SetupFailure> {
    let planned = plan_setup(plan, host)?;
    let trusted_before = host
        .certificate_trusted(&plan.paths.ca_certificate)
        .map_err(failure)?;
    let pf_enabled_before = host.pf_enabled().map_err(failure)?;
    let mut journal: Vec<(PathBuf, Option<SetupFile>)> = Vec::new();
    let mut result = SetupApplyResult {
        written: vec![],
        unchanged: vec![],
        backups: vec![],
    };
    let operation = (|| -> Result<(), String> {
        for item in planned {
            if item.previous.as_ref() == Some(&item.desired) {
                result.unchanged.push(item.artifact.path.clone());
                continue;
            }
            if let (Some(previous), Some(backup_path)) = (&item.previous, &item.backup_path)
                && host.inspect_file(backup_path)?.is_none()
            {
                host.write_backup(backup_path, previous)?;
                journal.push((backup_path.clone(), None));
                result.backups.push(backup_path.clone());
            }
            journal.push((item.artifact.path.clone(), item.previous.clone()));
            host.atomic_write(&item.artifact.path, &item.desired)?;
            result.written.push(item.artifact.path.clone());
        }
        for command in &plan.commands {
            if trusted_before
                && command.program == std::path::Path::new("/usr/bin/security")
                && command
                    .args
                    .first()
                    .is_some_and(|arg| arg == "add-trusted-cert")
            {
                continue;
            }
            let output = host.run(command)?;
            if output.status != 0 {
                return Err(command_error(command, &output));
            }
        }
        host.final_probe()?;
        Ok(())
    })();
    if let Err(message) = operation {
        let unresolved = rollback_setup(plan, host, journal, trusted_before, pf_enabled_before);
        return Err(SetupFailure {
            message,
            unresolved,
        });
    }
    Ok(result)
}

pub fn preview_uninstall(
    paths: &RuntimePaths,
    purge: bool,
    host: &dyn SetupHost,
) -> Result<SetupPreview, SetupFailure> {
    let manifest =
        read_manifest(paths, host)?.ok_or_else(|| failure("setup manifest is missing"))?;
    let mut actions = manifest
        .artifacts
        .iter()
        .rev()
        .map(|artifact| {
            let backup = backup_path(&manifest, &artifact.path);
            let has_backup = host.inspect_file(&backup).ok().flatten().is_some();
            SetupAction {
                kind: if has_backup { "restore" } else { "remove" }.into(),
                path: artifact.path.clone(),
                backup_path: has_backup.then_some(backup),
            }
        })
        .collect::<Vec<_>>();
    actions.push(SetupAction {
        kind: "remove".into(),
        path: paths.install_manifest.clone(),
        backup_path: None,
    });
    actions.push(SetupAction {
        kind: if purge { "purge" } else { "preserve" }.into(),
        path: paths.state_directory.clone(),
        backup_path: None,
    });
    Ok(SetupPreview { actions })
}

pub fn apply_uninstall(
    paths: &RuntimePaths,
    purge: bool,
    host: &dyn SetupHost,
) -> Result<SetupApplyResult, SetupFailure> {
    let preview = preview_uninstall(paths, purge, host)?;
    let mut result = SetupApplyResult {
        written: vec![],
        unchanged: vec![],
        backups: vec![],
    };
    if host
        .certificate_trusted(&paths.ca_certificate)
        .unwrap_or(false)
    {
        let fingerprint = certificate_sha256(host, &paths.ca_certificate).map_err(failure)?;
        let delete_trust = command(
            "/usr/bin/security",
            [
                "delete-certificate",
                "-Z",
                fingerprint.as_str(),
                "-t",
                "/Library/Keychains/System.keychain",
            ],
        );
        let output = host.run(&delete_trust).map_err(failure)?;
        if output.status != 0 {
            return Err(failure(command_error(&delete_trust, &output)));
        }
    }
    for action in &preview.actions {
        match action.kind.as_str() {
            "restore" => {
                let backup = host
                    .inspect_file(
                        action
                            .backup_path
                            .as_deref()
                            .expect("restore action has backup"),
                    )
                    .map_err(failure)?
                    .ok_or_else(|| failure("uninstall backup disappeared"))?;
                host.atomic_write(&action.path, &backup).map_err(failure)?;
                result.written.push(action.path.clone());
            }
            "remove" => {
                host.remove_file(&action.path).map_err(failure)?;
                result.written.push(action.path.clone());
            }
            "purge" => {
                host.remove_tree(&action.path).map_err(failure)?;
                result.written.push(action.path.clone());
            }
            "preserve" => result.unchanged.push(action.path.clone()),
            _ => unreachable!(),
        }
    }
    let reload = command("/sbin/pfctl", ["-f", PF_CONF]);
    let output = host.run(&reload).map_err(failure)?;
    if output.status != 0 {
        return Err(failure(command_error(&reload, &output)));
    }
    Ok(result)
}

fn plan_setup<'a>(
    plan: &'a HostSetupPlan,
    host: &dyn SetupHost,
) -> Result<Vec<PlannedArtifact<'a>>, SetupFailure> {
    let existing_manifest = read_manifest(&plan.paths, host)?;
    let owned = existing_manifest
        .as_ref()
        .map(|manifest| {
            manifest
                .artifacts
                .iter()
                .map(|item| item.path.clone())
                .collect::<HashSet<_>>()
        })
        .unwrap_or_default();
    let mut planned = Vec::new();
    for artifact in &plan.artifacts {
        let previous = host.inspect_file(&artifact.path).map_err(failure)?;
        if previous
            .as_ref()
            .is_some_and(|file| file.kind != HostFileKind::Regular)
        {
            return Err(failure(format!(
                "setup path is not a regular file: {}",
                artifact.path.display()
            )));
        }
        let desired_content = if artifact.managed_block {
            merge_managed_block(
                previous
                    .as_ref()
                    .map(|file| file.content.as_slice())
                    .unwrap_or_default(),
                artifact.content.as_bytes(),
            )?
        } else {
            artifact.content.as_bytes().to_vec()
        };
        let desired = SetupFile {
            content: desired_content,
            mode: artifact.mode,
            owner_id: artifact.owner_id,
            kind: HostFileKind::Regular,
        };
        let is_manifest = artifact.path == plan.paths.install_manifest;
        if previous.is_some()
            && existing_manifest.is_none()
            && !artifact.managed_block
            && !is_manifest
        {
            return Err(failure(format!(
                "unowned setup conflict: {}",
                artifact.path.display()
            )));
        }
        if previous.is_some()
            && existing_manifest.is_some()
            && !artifact.managed_block
            && !is_manifest
            && !owned.contains(&artifact.path)
        {
            return Err(failure(format!(
                "unowned setup conflict: {}",
                artifact.path.display()
            )));
        }
        let backup_path = previous
            .as_ref()
            .filter(|_| artifact.backup_existing)
            .map(|_| backup_path(&plan.manifest, &artifact.path));
        planned.push(PlannedArtifact {
            artifact,
            desired,
            previous,
            backup_path,
        });
    }
    Ok(planned)
}

fn read_manifest(
    paths: &RuntimePaths,
    host: &dyn SetupHost,
) -> Result<Option<HostInstallManifest>, SetupFailure> {
    let Some(file) = host
        .inspect_file(&paths.install_manifest)
        .map_err(failure)?
    else {
        return Ok(None);
    };
    if file.kind != HostFileKind::Regular {
        return Err(failure("setup manifest is not a regular file"));
    }
    serde_json::from_slice(&file.content)
        .map(Some)
        .map_err(|error| failure(format!("setup manifest is corrupt: {error}")))
}

fn merge_managed_block(previous: &[u8], block: &[u8]) -> Result<Vec<u8>, SetupFailure> {
    let previous =
        std::str::from_utf8(previous).map_err(|_| failure("PF configuration is not UTF-8"))?;
    let block = std::str::from_utf8(block).map_err(|_| failure("managed PF block is not UTF-8"))?;
    let without_block = match (previous.find(BEGIN_MARKER), previous.find(END_MARKER)) {
        (None, None) => previous.to_owned(),
        (Some(start), Some(end)) if start <= end => {
            let end = end + END_MARKER.len();
            format!("{}{}", &previous[..start], &previous[end..])
        }
        _ => {
            return Err(failure(
                "PF configuration contains an incomplete Own Domains marker block",
            ));
        }
    };
    let insertion = managed_block_insertion_offset(&without_block);
    let mut merged = String::with_capacity(without_block.len() + block.len() + 1);
    merged.push_str(&without_block[..insertion]);
    if !merged.is_empty() && !merged.ends_with('\n') {
        merged.push('\n');
    }
    merged.push_str(block);
    merged.push_str(&without_block[insertion..]);
    Ok(merged.into_bytes())
}

fn managed_block_insertion_offset(previous: &str) -> usize {
    let mut offset = 0;
    let mut after_translation = None;
    let mut before_filter = None;
    for line in previous.split_inclusive('\n') {
        let keyword = line.trim_start().split_ascii_whitespace().next();
        if keyword.is_some_and(is_translation_rule) {
            after_translation = Some(offset + line.len());
        } else if before_filter.is_none() && keyword.is_some_and(is_filter_or_queue_rule) {
            before_filter = Some(offset);
        }
        offset += line.len();
    }
    after_translation
        .or(before_filter)
        .unwrap_or(previous.len())
}

fn is_translation_rule(keyword: &str) -> bool {
    matches!(
        keyword,
        "rdr-anchor" | "nat-anchor" | "binat-anchor" | "rdr" | "nat" | "binat"
    )
}

fn is_filter_or_queue_rule(keyword: &str) -> bool {
    matches!(
        keyword,
        "dummynet-anchor" | "queue" | "anchor" | "pass" | "block" | "match"
    )
}

fn rollback_setup(
    plan: &HostSetupPlan,
    host: &dyn SetupHost,
    mut journal: Vec<(PathBuf, Option<SetupFile>)>,
    trusted_before: bool,
    pf_enabled_before: bool,
) -> Vec<String> {
    let mut unresolved = Vec::new();
    if !trusted_before
        && host
            .certificate_trusted(&plan.paths.ca_certificate)
            .unwrap_or(false)
    {
        match certificate_sha256(host, &plan.paths.ca_certificate) {
            Ok(fingerprint) => {
                let delete_trust = command(
                    "/usr/bin/security",
                    [
                        "delete-certificate",
                        "-Z",
                        fingerprint.as_str(),
                        "-t",
                        "/Library/Keychains/System.keychain",
                    ],
                );
                match host.run(&delete_trust) {
                    Ok(output) if output.status == 0 => {}
                    Ok(output) => unresolved.push(format!(
                        "remove trust: {}",
                        command_error(&delete_trust, &output)
                    )),
                    Err(error) => unresolved.push(format!("remove trust: {error}")),
                }
            }
            Err(error) => unresolved.push(format!("remove trust: {error}")),
        }
    }
    while let Some((path, previous)) = journal.pop() {
        let restored = match previous {
            Some(file) => host.atomic_write(&path, &file),
            None => host.remove_file(&path),
        };
        if let Err(error) = restored {
            unresolved.push(format!("restore {}: {error}", path.display()));
        }
    }
    let pf = if host
        .inspect_file(Path::new(PF_ANCHOR))
        .ok()
        .flatten()
        .is_some()
    {
        command("/sbin/pfctl", ["-a", "dev.owndomains", "-f", PF_ANCHOR])
    } else {
        command("/sbin/pfctl", ["-a", "dev.owndomains", "-F", "all"])
    };
    if let Err(error) = host.run(&pf) {
        unresolved.push(format!("restore PF anchor: {error}"));
    }
    if !pf_enabled_before {
        let disable = command("/sbin/pfctl", ["-d"]);
        if let Err(error) = host.run(&disable) {
            unresolved.push(format!("restore PF state: {error}"));
        }
    }
    unresolved
}

fn backup_path(manifest: &HostInstallManifest, path: &Path) -> PathBuf {
    manifest
        .backup_directory
        .join(path.strip_prefix("/").unwrap_or(path))
}

fn certificate_sha256(host: &dyn SetupHost, path: &Path) -> Result<String, String> {
    let file = host
        .inspect_file(path)?
        .ok_or_else(|| format!("{} is missing", path.display()))?;
    let pem = std::str::from_utf8(&file.content)
        .map_err(|error| format!("unable to read {}: {error}", path.display()))?;
    let body = pem
        .lines()
        .filter(|line| !line.trim_start().starts_with("-----"))
        .map(str::trim)
        .collect::<String>();
    let der = STANDARD
        .decode(body.as_bytes())
        .map_err(|error| format!("unable to decode {}: {error}", path.display()))?;
    Ok(format!("{:x}", Sha256::digest(der)))
}

fn command<const N: usize>(program: &str, args: [&str; N]) -> HostCommand {
    HostCommand {
        program: program.into(),
        args: args.into_iter().map(String::from).collect(),
        timeout: Duration::from_secs(10),
    }
}

fn command_error(command: &HostCommand, output: &HostCommandOutput) -> String {
    let detail = if output.stderr.is_empty() {
        &output.stdout
    } else {
        &output.stderr
    };
    format!(
        "{} failed: {}",
        command.program.display(),
        String::from_utf8_lossy(detail)
            .chars()
            .take(240)
            .collect::<String>()
    )
}

fn failure(message: impl ToString) -> SetupFailure {
    SetupFailure {
        message: message.to_string(),
        unresolved: vec![],
    }
}

#[allow(dead_code)]
fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

const MAX_SETUP_FILE_BYTES: u64 = 4 * 1024 * 1024;
const SYSTEM_SETUP_TIMEOUT: Duration = Duration::from_secs(5);

pub struct SystemSetupHost;

impl SystemSetupHost {
    fn inspect_metadata(path: &Path) -> Result<Option<(HostFileKind, u32, u32, u64)>, String> {
        let metadata = match std::fs::symlink_metadata(path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(format!("unable to inspect {}: {error}", path.display()));
            }
        };
        let file_type = metadata.file_type();
        let kind = if file_type.is_file() {
            HostFileKind::Regular
        } else if file_type.is_symlink() {
            HostFileKind::Symlink
        } else {
            HostFileKind::Other
        };
        #[cfg(unix)]
        let (owner_id, mode) = {
            use std::os::unix::fs::MetadataExt;
            (metadata.uid(), metadata.mode() & 0o7777)
        };
        #[cfg(not(unix))]
        let (owner_id, mode) = (0, 0);
        Ok(Some((kind, owner_id, mode, metadata.len())))
    }

    fn run_fixed(program: &str, args: &[&str]) -> Result<HostCommandOutput, String> {
        super::SystemCommandRunner.run(&HostCommand {
            program: program.into(),
            args: args.iter().map(|arg| (*arg).to_string()).collect(),
            timeout: SYSTEM_SETUP_TIMEOUT,
        })
    }
}

impl SetupHost for SystemSetupHost {
    fn inspect_file(&self, path: &Path) -> Result<Option<SetupFile>, String> {
        let Some((kind, owner_id, mode, length)) = Self::inspect_metadata(path)? else {
            return Ok(None);
        };
        if length > MAX_SETUP_FILE_BYTES {
            return Err(format!(
                "{} exceeds the {MAX_SETUP_FILE_BYTES}-byte setup file limit",
                path.display()
            ));
        }
        let content = if kind == HostFileKind::Regular {
            std::fs::read(path)
                .map_err(|error| format!("unable to read {}: {error}", path.display()))?
        } else {
            Vec::new()
        };
        Ok(Some(SetupFile {
            content,
            mode,
            owner_id,
            kind,
        }))
    }

    fn atomic_write(&self, path: &Path, file: &SetupFile) -> Result<(), String> {
        use std::io::Write;
        let parent = path
            .parent()
            .ok_or_else(|| format!("setup path has no parent: {}", path.display()))?;
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("unable to create {}: {error}", parent.display()))?;
        let file_name = path
            .file_name()
            .ok_or_else(|| format!("setup path has no file name: {}", path.display()))?;
        let temporary = parent.join(format!(
            ".{}.tmp-{}-{}",
            file_name.to_string_lossy(),
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|duration| duration.as_nanos())
                .unwrap_or_default()
        ));
        let result = (|| -> Result<(), String> {
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(file.mode);
            }
            let mut handle = options
                .open(&temporary)
                .map_err(|error| format!("unable to create {}: {error}", temporary.display()))?;
            handle
                .write_all(&file.content)
                .map_err(|error| format!("unable to write {}: {error}", temporary.display()))?;
            handle
                .sync_all()
                .map_err(|error| format!("unable to flush {}: {error}", temporary.display()))?;
            drop(handle);
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&temporary, std::fs::Permissions::from_mode(file.mode))
                    .map_err(|error| {
                        format!(
                            "unable to set permissions on {}: {error}",
                            temporary.display()
                        )
                    })?;
                let temporary_c = std::ffi::CString::new(temporary.as_os_str().as_encoded_bytes())
                    .map_err(|_| "setup temporary path contains a NUL byte".to_string())?;
                if unsafe { libc::chown(temporary_c.as_ptr(), file.owner_id, !0) } != 0 {
                    return Err(format!(
                        "unable to set owner on {}: {}",
                        temporary.display(),
                        std::io::Error::last_os_error()
                    ));
                }
            }
            std::fs::rename(&temporary, path).map_err(|error| {
                format!(
                    "unable to replace {} with {}: {error}",
                    path.display(),
                    temporary.display()
                )
            })
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&temporary);
        }
        result
    }

    fn remove_file(&self, path: &Path) -> Result<(), String> {
        match std::fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(format!("unable to remove {}: {error}", path.display())),
        }
    }

    fn remove_tree(&self, path: &Path) -> Result<(), String> {
        let metadata = match std::fs::symlink_metadata(path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(format!("unable to inspect {}: {error}", path.display())),
        };
        if metadata.is_dir() && !metadata.file_type().is_symlink() {
            std::fs::remove_dir_all(path)
                .map_err(|error| format!("unable to remove {}: {error}", path.display()))
        } else {
            self.remove_file(path)
        }
    }

    fn run(&self, command: &HostCommand) -> Result<HostCommandOutput, String> {
        super::SystemCommandRunner.run(command)
    }

    fn certificate_trusted(&self, certificate: &Path) -> Result<bool, String> {
        let certificate = certificate.to_string_lossy();
        Self::run_fixed(
            "/usr/bin/security",
            &["verify-cert", "-c", certificate.as_ref(), "-p", "ssl"],
        )
        .map(|output| output.status == 0)
    }

    fn pf_enabled(&self) -> Result<bool, String> {
        match Self::run_fixed("/sbin/pfctl", &["-s", "info"]) {
            Ok(output) => Ok(output.status == 0
                && String::from_utf8_lossy(&output.stdout).contains("Status: Enabled")),
            Err(_) => Ok(false),
        }
    }

    fn final_probe(&self) -> Result<(), String> {
        let resolver = self
            .inspect_file(Path::new("/etc/resolver/own"))?
            .ok_or_else(|| "resolver file is missing after setup".to_string())?;
        if resolver.kind != HostFileKind::Regular
            || resolver.content != b"nameserver 127.0.0.1\nport 5354\n"
        {
            return Err("resolver file does not match the owned configuration".into());
        }
        let anchor = Self::run_fixed("/sbin/pfctl", &["-a", "dev.owndomains", "-s", "nat"])?;
        if anchor.status != 0 || anchor.stdout.is_empty() {
            return Err("PF anchor is not active after setup".into());
        }
        let main = Self::run_fixed("/sbin/pfctl", &["-s", "nat"])?;
        if main.status != 0
            || !String::from_utf8_lossy(&main.stdout).contains("rdr-anchor \"dev.owndomains\"")
        {
            return Err("PF main ruleset does not reference the Own Domains anchor".into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BLOCK: &str = concat!(
        "# BEGIN OWN DOMAINS\n",
        "rdr-anchor \"dev.owndomains\"\n",
        "load anchor \"dev.owndomains\" from \"/etc/pf.anchors/dev.owndomains\"\n",
        "# END OWN DOMAINS\n",
    );

    #[test]
    fn inserts_managed_block_before_filter_anchor() {
        let previous = concat!(
            "scrub-anchor \"com.apple/*\"\n",
            "nat-anchor \"com.apple/*\"\n",
            "rdr-anchor \"com.apple/*\"\n",
            "dummynet-anchor \"com.apple/*\"\n",
            "anchor \"com.apple/*\"\n",
            "load anchor \"com.apple\" from \"/etc/pf.anchors/com.apple\"\n",
        );

        let merged =
            String::from_utf8(merge_managed_block(previous.as_bytes(), BLOCK.as_bytes()).unwrap())
                .unwrap();

        let block = merged.find(BLOCK).unwrap();
        let filter_anchor = merged.find("\nanchor \"com.apple/*\"").unwrap() + 1;
        assert!(
            block < filter_anchor,
            "managed block must precede filter rules:\n{merged}"
        );
    }

    #[test]
    fn moves_existing_block_before_filter_anchor() {
        let previous = format!(
            "scrub-anchor \"com.apple/*\"\nrdr-anchor \"com.apple/*\"\nanchor \"com.apple/*\"\n{BLOCK}"
        );

        let merged =
            String::from_utf8(merge_managed_block(previous.as_bytes(), BLOCK.as_bytes()).unwrap())
                .unwrap();

        assert_eq!(merged.matches(BEGIN_MARKER).count(), 1);
        let block = merged.find(BLOCK).unwrap();
        let filter_anchor = merged.find("\nanchor \"com.apple/*\"").unwrap() + 1;
        assert!(
            block < filter_anchor,
            "existing block must move before filter rules:\n{merged}"
        );
    }

    #[test]
    fn inserts_before_first_filter_when_no_translation_rule_exists() {
        let merged = String::from_utf8(
            merge_managed_block(b"set skip on lo0\npass all\n", BLOCK.as_bytes()).unwrap(),
        )
        .unwrap();

        let block = merged.find(BLOCK).unwrap();
        let pass = merged.find("\npass all").unwrap() + 1;
        assert!(
            block < pass,
            "managed block must precede filter rules:\n{merged}"
        );
    }

    #[test]
    fn merging_the_managed_block_is_idempotent() {
        let previous = concat!(
            "scrub-anchor \"com.apple/*\"\n",
            "rdr-anchor \"com.apple/*\"\n",
            "anchor \"com.apple/*\"\n",
        );
        let once = merge_managed_block(previous.as_bytes(), BLOCK.as_bytes()).unwrap();
        let twice = merge_managed_block(&once, BLOCK.as_bytes()).unwrap();

        assert_eq!(once, twice);
    }
}
