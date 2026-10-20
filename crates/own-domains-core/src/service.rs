use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex, OnceLock, Weak},
};

use crate::{
    CoreError, CoreResult, MappingInput, MappingPatch, Settings, SettingsStore, validate_mapping,
    validate_network,
};

static STORE_LOCKS: OnceLock<Mutex<HashMap<PathBuf, Weak<Mutex<()>>>>> = OnceLock::new();

pub struct DomainService {
    store: SettingsStore,
    mutation_lock: Arc<Mutex<()>>,
}

impl DomainService {
    pub fn new(store: SettingsStore) -> Self {
        let key = store.lock_key();
        let locks = STORE_LOCKS.get_or_init(|| Mutex::new(HashMap::new()));
        let mut locks = locks.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let mutation_lock = locks.get(&key).and_then(Weak::upgrade).unwrap_or_else(|| {
            let lock = Arc::new(Mutex::new(()));
            locks.insert(key, Arc::downgrade(&lock));
            lock
        });
        drop(locks);
        Self { store, mutation_lock }
    }

    pub fn list(&self) -> CoreResult<Settings> {
        self.store.load()
    }

    pub fn create(&self, input: MappingInput, expected_revision: u64) -> CoreResult<Settings> {
        self.mutate(expected_revision, move |mut settings| {
            let mapping = validate_mapping(input)?;
            if settings.mappings.iter().any(|item| item.hostname == mapping.hostname) {
                return Err(CoreError::DuplicateHostname { hostname: mapping.hostname });
            }
            settings.mappings.push(mapping);
            Ok(settings)
        })
    }

    pub fn update(&self, id: &str, patch: MappingPatch, expected_revision: u64) -> CoreResult<Settings> {
        let id = id.to_string();
        self.mutate(expected_revision, move |mut settings| {
            let index = settings.mappings.iter().position(|mapping| mapping.id == id).ok_or_else(|| CoreError::NotFound {
                entity: "domain mapping",
                id: id.clone(),
            })?;
            let current = &settings.mappings[index];
            let mapping = validate_mapping(MappingInput {
                id: Some(current.id.clone()),
                hostname: patch.hostname.unwrap_or_else(|| current.hostname.clone()),
                upstream_port: patch.upstream_port.unwrap_or_else(|| u32::from(current.upstream_port)),
                tls: patch.tls.unwrap_or(current.tls),
                enabled: patch.enabled.unwrap_or(current.enabled),
            })?;
            if settings.mappings.iter().enumerate().any(|(item_index, item)| item_index != index && item.hostname == mapping.hostname) {
                return Err(CoreError::DuplicateHostname { hostname: mapping.hostname });
            }
            settings.mappings[index] = mapping;
            Ok(settings)
        })
    }

    pub fn remove(&self, id: &str, expected_revision: u64) -> CoreResult<Settings> {
        let id = id.to_string();
        self.mutate(expected_revision, move |mut settings| {
            let original_length = settings.mappings.len();
            settings.mappings.retain(|mapping| mapping.id != id);
            if settings.mappings.len() == original_length {
                return Err(CoreError::NotFound { entity: "domain mapping", id });
            }
            Ok(settings)
        })
    }

    pub fn update_network(&self, lan_address: String, upstream_dns: Vec<String>, expected_revision: u64) -> CoreResult<Settings> {
        validate_network(&lan_address, &upstream_dns)?;
        self.mutate(expected_revision, move |mut settings| {
            settings.lan_address = lan_address;
            settings.upstream_dns = upstream_dns;
            Ok(settings)
        })
    }

    fn mutate(&self, expected_revision: u64, operation: impl FnOnce(Settings) -> CoreResult<Settings>) -> CoreResult<Settings> {
        let _guard = self.mutation_lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let current = self.store.load()?;
        if current.revision != expected_revision {
            return Err(CoreError::RevisionConflict { expected: expected_revision, actual: current.revision });
        }
        let mut updated = operation(current)?;
        updated.revision = expected_revision + 1;
        self.store.save(&updated, Some(expected_revision))?;
        Ok(updated)
    }
}
