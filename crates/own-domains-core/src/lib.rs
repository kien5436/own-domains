mod error;
mod model;
mod proxy;
mod service;
mod store;
mod validation;

pub use error::{CoreError, CoreResult};
pub use model::{
    ApplyResult, ApplyStatus, Diagnostic, DiagnosticStatus, DomainMapping, MappingInput,
    MappingPatch, Settings,
};
pub use proxy::{ProxyConfigOptions, build_proxy_config};
pub use service::DomainService;
pub use store::SettingsStore;
pub use validation::{
    normalize_hostname, validate_hostname, validate_mapping, validate_network, validate_settings,
};

