use std::{path::PathBuf, sync::{Arc, Barrier}, thread};

use own_domains_core::{CoreError, DomainService, MappingInput, MappingPatch, SettingsStore};
use uuid::Uuid;

fn temp() -> PathBuf { std::env::temp_dir().join(format!("own-domains-service-{}", Uuid::new_v4())) }
fn input(hostname: &str, port: u32) -> MappingInput { MappingInput { id: None, hostname: hostname.into(), upstream_port: port, tls: false, enabled: true } }

#[test]
fn crud_normalizes_increments_and_preserves_identity() {
    let directory = temp();
    let service = DomainService::new(SettingsStore::new(&directory));
    let created = service.create(input("Shop.OWN.", 3000), 0).unwrap();
    assert_eq!(created.revision, 1);
    assert_eq!(created.mappings[0].hostname, "shop.own");
    let id = created.mappings[0].id.clone();

    let updated = service.update(&id, MappingPatch { upstream_port: Some(4000), tls: Some(true), ..MappingPatch::default() }, 1).unwrap();
    assert_eq!(updated.revision, 2);
    assert_eq!(updated.mappings[0].id, id);
    assert_eq!(updated.mappings[0].upstream_port, 4000);
    assert!(updated.mappings[0].tls);

    let removed = service.remove(&id, 2).unwrap();
    assert_eq!(removed.revision, 3);
    assert!(removed.mappings.is_empty());
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn rejected_mutations_leave_storage_unchanged() {
    let directory = temp();
    let service = DomainService::new(SettingsStore::new(&directory));
    let first = service.create(input("shop.own", 3000), 0).unwrap();
    let before = service.list().unwrap();
    assert!(matches!(service.create(input("SHOP.OWN.", 3001), 1), Err(CoreError::DuplicateHostname { .. })));
    assert!(matches!(service.update("missing", MappingPatch::default(), 1), Err(CoreError::NotFound { .. })));
    assert!(matches!(service.remove("missing", 1), Err(CoreError::NotFound { .. })));
    assert!(service.update(&first.mappings[0].id, MappingPatch { upstream_port: Some(53), ..MappingPatch::default() }, 1).is_err());
    assert_eq!(service.list().unwrap(), before);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn network_updates_are_validated_before_persistence() {
    let directory = temp();
    let service = DomainService::new(SettingsStore::new(&directory));
    let updated = service.update_network("192.168.1.20".into(), vec!["1.1.1.1".into()], 0).unwrap();
    assert_eq!(updated.revision, 1);
    assert_eq!(updated.lan_address, "192.168.1.20");
    let before = service.list().unwrap();
    assert!(service.update_network("192.168.1.20".into(), vec!["192.168.1.20".into()], 1).is_err());
    assert_eq!(service.list().unwrap(), before);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn concurrent_writers_with_the_same_revision_only_commit_once() {
    let directory = temp();
    let service = Arc::new(DomainService::new(SettingsStore::new(&directory)));
    let barrier = Arc::new(Barrier::new(3));
    let handles: Vec<_> = ["one.own", "two.own"].into_iter().map(|hostname| {
        let service = Arc::clone(&service);
        let barrier = Arc::clone(&barrier);
        thread::spawn(move || { barrier.wait(); service.create(input(hostname, 3000), 0) })
    }).collect();
    barrier.wait();
    let results: Vec<_> = handles.into_iter().map(|handle| handle.join().unwrap()).collect();
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(results.iter().filter(|result| matches!(result, Err(CoreError::RevisionConflict { expected: 0, actual: 1 }))).count(), 1);
    let stored = service.list().unwrap();
    assert_eq!(stored.revision, 1);
    assert_eq!(stored.mappings.len(), 1);
    std::fs::remove_dir_all(directory).unwrap();
}
