use super::{MacOsArchitecture, MacOsRuntimeBackend, RuntimePaths, SidecarManifest};
use crate::{RuntimeBackend, UnavailableRuntimeBackend};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

pub(crate) fn runtime_backend(
    state_dir: &Path,
    resources: Option<PathBuf>,
    sidecars: Option<PathBuf>,
    settings_only: bool,
) -> Result<Arc<dyn RuntimeBackend>, Box<dyn std::error::Error>> {
    if !settings_only {
        let state_dir = if state_dir.is_absolute() {
            state_dir.to_path_buf()
        } else {
            std::env::current_dir()?.join(state_dir)
        };
        let resources = match resources {
            Some(resources) => resources,
            None => default_resource_directory(&executable_directory(&std::env::current_exe()?)?),
        };
        let manifest_path = sidecars.unwrap_or_else(|| {
            sidecar_manifest_path(&resources, macos_target_triple(macos_architecture()))
        });
        let manifest = std::fs::read(&manifest_path).map_err(|error| {
            format!(
                "unable to read the sidecar manifest {}: {error}",
                manifest_path.display()
            )
        })?;
        let sidecars: SidecarManifest = serde_json::from_slice(&manifest).map_err(|error| {
            format!(
                "unable to parse the sidecar manifest {}: {error}",
                manifest_path.display()
            )
        })?;
        let backup_directory = state_dir.join("backups/setup-1");
        let architecture = macos_architecture();
        let paths = RuntimePaths::new(state_dir, resources, backup_directory, architecture)?;
        return Ok(Arc::new(MacOsRuntimeBackend::try_new(paths, sidecars)?));
    }
    Ok(Arc::new(UnavailableRuntimeBackend))
}

fn executable_directory(executable: &Path) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let executable = executable.canonicalize()?;
    executable
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| "the daemon executable has no parent directory".into())
}

fn default_resource_directory(executable_directory: &Path) -> PathBuf {
    let resources = executable_directory.join("../Resources");
    if resources.join("sidecars").is_dir() {
        resources
    } else {
        executable_directory.to_path_buf()
    }
}

fn sidecar_manifest_path(resources: &Path, target: &str) -> PathBuf {
    let sidecars = resources.join("sidecars");
    let manifest = sidecars.join(format!("manifest-{target}.json"));
    if manifest.is_file() {
        manifest
    } else {
        sidecars.join("manifest.json")
    }
}

fn macos_target_triple(architecture: MacOsArchitecture) -> &'static str {
    match architecture {
        MacOsArchitecture::Arm64 => "aarch64-apple-darwin",
        MacOsArchitecture::X86_64 => "x86_64-apple-darwin",
    }
}

fn macos_architecture() -> MacOsArchitecture {
    #[cfg(target_arch = "aarch64")]
    {
        MacOsArchitecture::Arm64
    }
    #[cfg(target_arch = "x86_64")]
    {
        MacOsArchitecture::X86_64
    }
    #[cfg(not(any(target_arch = "aarch64", target_arch = "x86_64")))]
    {
        compile_error!("the macOS runtime requires an Apple silicon or Intel build")
    }
}

#[cfg(test)]
mod tests {
    use super::executable_directory;

    #[test]
    #[cfg(unix)]
    fn executable_directory_follows_a_symlink_into_the_app_bundle() {
        let root = std::env::temp_dir().join(format!(
            "own-domains-factory-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let contents = root.join("Own Domains.app").join("Contents");
        let macos = contents.join("MacOS");
        std::fs::create_dir_all(&macos).unwrap();
        let daemon = macos.join("own-domains-daemon");
        std::fs::write(&daemon, b"daemon").unwrap();
        let link = root.join("own-domains-daemon");
        std::os::unix::fs::symlink(&daemon, &link).unwrap();

        assert_eq!(
            executable_directory(&link).unwrap(),
            macos.canonicalize().unwrap()
        );

        std::fs::remove_dir_all(root).unwrap();
    }
}
