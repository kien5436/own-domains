use own_domains_core::{
    DnsConfigOptions, ProxyConfigOptions, Settings, build_corefile, build_proxy_config,
};
use serde_json::json;
use std::{
    net::Ipv4Addr,
    path::{Path, PathBuf},
    time::Duration,
};

use super::{
    CandidateConfiguration, ConfigurationError, ConfigurationFiles, ConfigurationValidator,
    DEFAULT_DNS_VALIDATION_PORT, HostCommand, RuntimePaths,
};

const FILE_MODE: u32 = 0o600;
const VALIDATION_SETTLE: Duration = Duration::from_millis(500);

pub struct ConfigurationStager<'a> {
    paths: &'a RuntimePaths,
    files: &'a dyn ConfigurationFiles,
    validator: &'a dyn ConfigurationValidator,
    validation_port: u16,
}

impl<'a> ConfigurationStager<'a> {
    pub fn new(
        paths: &'a RuntimePaths,
        files: &'a dyn ConfigurationFiles,
        validator: &'a dyn ConfigurationValidator,
    ) -> Self {
        Self {
            paths,
            files,
            validator,
            validation_port: DEFAULT_DNS_VALIDATION_PORT,
        }
    }

    pub fn with_validation_port(mut self, port: u16) -> Self {
        self.validation_port = port;
        self
    }

    pub fn stage(&self, settings: &Settings) -> Result<CandidateConfiguration, ConfigurationError> {
        if self.validation_port == 0 || [53, 5354, 8080, 8443, 4781].contains(&self.validation_port)
        {
            return Err(ConfigurationError::Validation(format!(
                "DNS validation port {} conflicts with a runtime port",
                self.validation_port
            )));
        }

        let dns_options = DnsConfigOptions::for_settings(settings)
            .map_err(|error| ConfigurationError::Validation(error.to_string()))?;
        let corefile = build_corefile(settings, &dns_options)
            .map_err(|error| ConfigurationError::Validation(error.to_string()))?;
        let validation_options = DnsConfigOptions {
            listen_addresses: vec![Ipv4Addr::LOCALHOST],
            port: self.validation_port,
            upstream_port: 53,
            ttl_seconds: 60,
        };
        let validation_corefile = build_corefile(settings, &validation_options)
            .map_err(|error| ConfigurationError::Validation(error.to_string()))?;

        let admin_socket = self.caddy_admin_socket();
        let proxy_options = ProxyConfigOptions::for_runtime(settings, &admin_socket)
            .map_err(|error| ConfigurationError::Validation(error.to_string()))?;
        let mut proxy = build_proxy_config(settings, &proxy_options)
            .map_err(|error| ConfigurationError::Validation(error.to_string()))?;
        proxy["storage"] = json!({
            "module": "file_system",
            "root": self.paths.caddy_data_directory.to_string_lossy(),
        });
        let mut proxy_bytes = serde_json::to_vec_pretty(&proxy)
            .map_err(|error| ConfigurationError::Promotion(error.to_string()))?;

        proxy_bytes.push(b'\n');

        let corefile_path = self.paths.candidate_directory.join("Corefile");
        let proxy_path = self.paths.candidate_directory.join("caddy.json");
        let validation_corefile_path = self.paths.candidate_directory.join("Corefile.validation");
        let revision_path = self.paths.candidate_directory.join("revision");

        self.files
            .create_dir_all(&self.paths.candidate_directory)
            .map_err(ConfigurationError::Promotion)?;
        self.files
            .write_atomic(&corefile_path, corefile.as_bytes(), FILE_MODE)
            .map_err(ConfigurationError::Promotion)?;
        self.files
            .write_atomic(&proxy_path, &proxy_bytes, FILE_MODE)
            .map_err(ConfigurationError::Promotion)?;
        self.files
            .write_atomic(
                &validation_corefile_path,
                validation_corefile.as_bytes(),
                FILE_MODE,
            )
            .map_err(ConfigurationError::Promotion)?;
        self.files
            .write_atomic(
                &revision_path,
                settings.revision.to_string().as_bytes(),
                FILE_MODE,
            )
            .map_err(ConfigurationError::Promotion)?;

        Ok(CandidateConfiguration {
            revision: settings.revision,
            corefile,
            proxy,
            corefile_path,
            proxy_path,
            validation_corefile_path,
        })
    }

    pub fn validate(&self, candidate: &CandidateConfiguration) -> Result<(), ConfigurationError> {
        let caddy = self.caddy_validation_command(&candidate.proxy_path);
        self.validator.run(&caddy).map_err(|message| {
            ConfigurationError::ValidationFailure(format!("Caddy validation failed: {message}"))
        })?;

        let coredns = self.coredns_validation_command(&candidate.validation_corefile_path);
        self.validator
            .expect_running(&coredns, VALIDATION_SETTLE)
            .map_err(|message| {
                ConfigurationError::ValidationFailure(format!(
                    "CoreDNS validation failed: {message}"
                ))
            })
    }

    pub fn promote(&self, candidate: &CandidateConfiguration) -> Result<u64, ConfigurationError> {
        let corefile = self.read(&candidate.corefile_path)?;
        let proxy = self.read(&candidate.proxy_path)?;
        let applied_revision = self.paths.applied_directory.join("revision");
        let previous = self.snapshot_applied()?;

        self.files
            .create_dir_all(&self.paths.applied_directory)
            .map_err(ConfigurationError::Promotion)?;
        self.prepare_previous(previous.as_ref())?;
        if let Err(error) = self.write_applied(&corefile, &proxy, candidate.revision) {
            self.restore_from(previous.as_ref())?;
            return Err(error);
        }

        let revision = self
            .files
            .read(&applied_revision)
            .map_err(ConfigurationError::Promotion)?;
        if revision.as_deref() != Some(candidate.revision.to_string().as_bytes()) {
            let _ = self.restore_from(previous.as_ref());
            return Err(ConfigurationError::Promotion(
                "applied revision was not recorded".into(),
            ));
        }

        Ok(candidate.revision)
    }

    pub fn restore(&self) -> Result<Option<u64>, ConfigurationError> {
        let previous_corefile = self
            .files
            .read(&self.previous_directory().join("Corefile"))
            .map_err(ConfigurationError::Promotion)?;
        let previous_proxy = self
            .files
            .read(&self.previous_directory().join("caddy.json"))
            .map_err(ConfigurationError::Promotion)?;
        let previous_revision = self
            .files
            .read(&self.previous_directory().join("revision"))
            .map_err(ConfigurationError::Promotion)?;

        let Some(corefile) = previous_corefile else {
            return Ok(None);
        };
        let proxy = previous_proxy.ok_or_else(|| {
            ConfigurationError::Promotion("previous configuration is incomplete".into())
        })?;
        let revision = previous_revision.ok_or_else(|| {
            ConfigurationError::Promotion("previous configuration is incomplete".into())
        })?;
        let revision = parse_revision(&revision)?;

        self.files
            .create_dir_all(&self.paths.applied_directory)
            .map_err(ConfigurationError::Promotion)?;
        self.write_applied(&corefile, &proxy, revision)?;
        self.files
            .remove_tree(&self.previous_directory())
            .map_err(ConfigurationError::Promotion)?;
        Ok(Some(revision))
    }

    pub fn caddy_admin_socket(&self) -> PathBuf {
        self.paths.runtime_directory.join("caddy-admin.sock")
    }

    pub fn applied_corefile(&self) -> PathBuf {
        self.paths.applied_directory.join("Corefile")
    }

    pub fn applied_proxy(&self) -> PathBuf {
        self.paths.applied_directory.join("caddy.json")
    }

    pub fn previous_directory(&self) -> PathBuf {
        self.paths.runtime_directory.join("previous")
    }

    fn caddy_validation_command(&self, path: &Path) -> HostCommand {
        HostCommand {
            program: self.paths.caddy_binary.clone(),
            args: vec![
                "validate".into(),
                "--config".into(),
                path.to_string_lossy().into_owned(),
            ],
            timeout: Duration::from_secs(15),
        }
    }

    fn coredns_validation_command(&self, path: &Path) -> HostCommand {
        HostCommand {
            program: self.paths.coredns_binary.clone(),
            args: vec!["-conf".into(), path.to_string_lossy().into_owned()],
            timeout: Duration::from_secs(3),
        }
    }

    fn read(&self, path: &Path) -> Result<Vec<u8>, ConfigurationError> {
        self.files
            .read(path)
            .map_err(ConfigurationError::Promotion)?
            .ok_or_else(|| {
                ConfigurationError::Promotion(format!(
                    "candidate configuration is missing: {}",
                    path.display()
                ))
            })
    }

    fn snapshot_applied(&self) -> Result<Option<AppliedConfiguration>, ConfigurationError> {
        let corefile = self
            .files
            .read(&self.applied_corefile())
            .map_err(ConfigurationError::Promotion)?;
        let proxy = self
            .files
            .read(&self.applied_proxy())
            .map_err(ConfigurationError::Promotion)?;
        let revision = self
            .files
            .read(&self.paths.applied_directory.join("revision"))
            .map_err(ConfigurationError::Promotion)?;
        match (corefile, proxy, revision) {
            (None, None, None) => Ok(None),
            (Some(corefile), Some(proxy), Some(revision)) => Ok(Some(AppliedConfiguration {
                corefile,
                proxy,
                revision,
            })),
            _ => Err(ConfigurationError::Promotion(
                "the applied configuration is incomplete".into(),
            )),
        }
    }

    fn prepare_previous(
        &self,
        previous: Option<&AppliedConfiguration>,
    ) -> Result<(), ConfigurationError> {
        self.files
            .remove_tree(&self.previous_directory())
            .map_err(ConfigurationError::Promotion)?;
        let Some(snapshot) = previous else {
            return Ok(());
        };
        self.files
            .create_dir_all(&self.previous_directory())
            .map_err(ConfigurationError::Promotion)?;
        for (name, contents) in [
            ("Corefile", snapshot.corefile.as_slice()),
            ("caddy.json", snapshot.proxy.as_slice()),
            ("revision", snapshot.revision.as_slice()),
        ] {
            self.files
                .write_atomic(&self.previous_directory().join(name), contents, FILE_MODE)
                .map_err(ConfigurationError::Promotion)?;
        }
        Ok(())
    }

    fn write_applied(
        &self,
        corefile: &[u8],
        proxy: &[u8],
        revision: u64,
    ) -> Result<(), ConfigurationError> {
        self.files
            .write_atomic(&self.applied_corefile(), corefile, FILE_MODE)
            .map_err(ConfigurationError::Promotion)?;
        self.files
            .write_atomic(&self.applied_proxy(), proxy, FILE_MODE)
            .map_err(ConfigurationError::Promotion)?;
        self.files
            .write_atomic(
                &self.paths.applied_directory.join("revision"),
                revision.to_string().as_bytes(),
                FILE_MODE,
            )
            .map_err(ConfigurationError::Promotion)
    }

    fn restore_from(
        &self,
        previous: Option<&AppliedConfiguration>,
    ) -> Result<(), ConfigurationError> {
        match previous {
            Some(snapshot) => {
                self.files
                    .write_atomic(&self.applied_corefile(), &snapshot.corefile, FILE_MODE)
                    .map_err(ConfigurationError::Promotion)?;
                self.files
                    .write_atomic(&self.applied_proxy(), &snapshot.proxy, FILE_MODE)
                    .map_err(ConfigurationError::Promotion)?;
                self.files
                    .write_atomic(
                        &self.paths.applied_directory.join("revision"),
                        &snapshot.revision,
                        FILE_MODE,
                    )
                    .map_err(ConfigurationError::Promotion)?;
            }
            None => {
                for path in [
                    self.applied_corefile(),
                    self.applied_proxy(),
                    self.paths.applied_directory.join("revision"),
                ] {
                    let _ = self.files.remove_file(&path);
                }
            }
        }
        Ok(())
    }
}

struct AppliedConfiguration {
    corefile: Vec<u8>,
    proxy: Vec<u8>,
    revision: Vec<u8>,
}

fn parse_revision(bytes: &[u8]) -> Result<u64, ConfigurationError> {
    std::str::from_utf8(bytes)
        .ok()
        .and_then(|text| text.trim().parse().ok())
        .ok_or_else(|| ConfigurationError::Promotion("applied revision is invalid".into()))
}
