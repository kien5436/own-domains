use own_domains_core::{ApplyResult, ApplyStatus, MappingInput, MappingPatch, Settings};
use own_domains_protocol::*;
use serde_json::json;
#[test]
fn desktop_wire_contract_matches_rust_serialization() {
    let snapshot = StateSnapshot {
        settings: Settings::default(),
        runtime_status: RuntimeStatus::Stopped,
        desired_revision: 0,
        applied_revision: 0,
        diagnostics: vec![],
        latest_event_sequence: 0,
    };
    let commands = vec![
        Command::Snapshot,
        Command::Start,
        Command::Stop,
        Command::List,
        Command::Doctor,
        Command::Create {
            input: MappingInput {
                id: Some("one".into()),
                hostname: "shop.own".into(),
                upstream_port: 3000,
                tls: false,
                enabled: true,
            },
            expected_revision: 0,
        },
        Command::Update {
            id: "one".into(),
            patch: MappingPatch {
                hostname: Some("shop.own".into()),
                upstream_port: Some(4000),
                tls: Some(true),
                enabled: Some(false),
            },
            expected_revision: 1,
        },
        Command::Remove {
            id: "one".into(),
            expected_revision: 2,
        },
        Command::UpdateNetwork {
            lan_address: "192.168.1.20".into(),
            upstream_dns: vec!["1.1.1.1".into()],
            expected_revision: 3,
        },
    ];
    let results = vec![
        CommandResult::Snapshot(snapshot.clone()),
        CommandResult::Settings(Settings::default()),
        CommandResult::Mutation(MutationResult {
            settings: Settings::default(),
            apply: None,
        }),
        CommandResult::Mutation(MutationResult {
            settings: Settings {
                revision: 1,
                ..Settings::default()
            },
            apply: Some(ApplyResult {
                desired_revision: 1,
                applied_revision: 0,
                status: ApplyStatus::Error,
                message: Some("candidate rejected".into()),
            }),
        }),
        CommandResult::Diagnostics(vec![]),
        CommandResult::Runtime(RuntimeStatus::Stopped),
        CommandResult::Apply(ApplyResult {
            desired_revision: 1,
            applied_revision: 0,
            status: ApplyStatus::Pending,
            message: None,
        }),
        CommandResult::Acknowledged,
        CommandResult::Subscribed,
        CommandResult::Handshake {
            protocol_version: PROTOCOL_VERSION,
        },
    ];
    let events = vec![
        Event::Snapshot(snapshot.clone()),
        Event::ResyncRequired { snapshot },
        Event::SettingsChanged { revision: 1 },
        Event::RuntimeChanged {
            status: RuntimeStatus::Degraded,
        },
        Event::DiagnosticsChanged {
            diagnostics: vec![],
        },
    ];
    let errors = vec![
        ProtocolError::IncompatibleVersion {
            supported: PROTOCOL_VERSION,
            received: 1,
        },
        ProtocolError::RevisionConflict {
            expected: 1,
            actual: 2,
        },
        ProtocolError::DuplicateHostname {
            hostname: "shop.own".into(),
        },
        ProtocolError::AlreadyRunning {
            path: "/tmp/state".into(),
        },
        ProtocolError::Validation {
            message: "invalid".into(),
        },
        ProtocolError::NotFound {
            message: "missing".into(),
        },
        ProtocolError::RuntimeUnavailable {
            message: "unavailable".into(),
        },
        ProtocolError::RuntimeFailure {
            message: "failed".into(),
        },
        ProtocolError::Storage {
            message: "storage".into(),
        },
        ProtocolError::InvalidRequest {
            message: "invalid request".into(),
        },
    ];
    let value = json!({ "commands": commands, "results": results, "events": events.into_iter().enumerate().map(|(index, event)| EventEnvelope { protocol_version: PROTOCOL_VERSION, sequence: index as u64, event }).collect::<Vec<_>>(), "errors": errors, "subscription": Command::Subscribe { after_sequence: Some(4) } });
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/ipc/desktop-contract.json");
    if std::env::var_os("UPDATE_IPC_FIXTURE").is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            format!("{}\n", serde_json::to_string_pretty(&value).unwrap()),
        )
        .unwrap();
    }
    let expected: serde_json::Value = serde_json::from_slice(
        &std::fs::read(path).expect("Generate fixture with UPDATE_IPC_FIXTURE=1"),
    )
    .unwrap();
    assert_eq!(value, expected);
    assert_eq!(value["commands"][5]["payload"]["expected_revision"], 0);
    assert_eq!(value["subscription"]["payload"]["after_sequence"], 4);
}
