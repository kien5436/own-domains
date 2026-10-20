mod error;
mod model;
mod validation;

pub use error::{CoreError, CoreResult};
pub use model::{
    ApplyResult, ApplyStatus, Diagnostic, DiagnosticStatus, DomainMapping, MappingInput,
    MappingPatch, Settings,
};
pub use validation::{
    normalize_hostname, validate_hostname, validate_mapping, validate_network, validate_settings,
};

