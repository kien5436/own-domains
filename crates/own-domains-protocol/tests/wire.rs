use own_domains_core::{
    ApplyResult, ApplyStatus, Diagnostic, DiagnosticStatus, MappingInput, MappingPatch, Settings,
};
use own_domains_protocol::*;

#[test]
fn commands_use_tagged_camel_case_wire_format() {
    let commands = [
        Command::Handshake,
        Command::Snapshot,
        Command::Subscribe {
            after_sequence: Some(4),
        },
        Command::Start,
        Command::Stop,
        Command::List,
        Command::Create {
            input: MappingInput {
                id: None,
                hostname: "shop.own".into(),
                upstream_port: 3000,
                tls: true,
                enabled: true,
            },
            expected_revision: 1,
        },
        Command::Update {
            id: "id".into(),
            patch: MappingPatch::default(),
            expected_revision: 1,
        },
        Command::Remove {
            id: "id".into(),
            expected_revision: 1,
        },
        Command::UpdateNetwork {
            lan_address: "192.168.1.2".into(),
            upstream_dns: vec!["1.1.1.1".into()],
            expected_revision: 1,
        },
        Command::Doctor,
    ];
    let expected = [
        "handshake",
        "snapshot",
        "subscribe",
        "start",
        "stop",
        "list",
        "create",
        "update",
        "remove",
        "updateNetwork",
        "doctor",
    ];
    for (command, command_type) in commands.into_iter().zip(expected) {
        let json = serde_json::to_value(&command).unwrap();
        assert_eq!(json["type"], command_type);
        assert_eq!(serde_json::from_value::<Command>(json).unwrap(), command);
    }
}

#[test]
fn validates_versions_and_echoes_request_ids() {
    for received in [0, 1, 3] {
        assert_eq!(
            validate_version(received),
            Err(ProtocolError::IncompatibleVersion {
                supported: 2,
                received
            })
        );
    }
    assert!(serde_json::from_str::<Command>(r#"{"type":"future"}"#).is_err());
    let response = ResponseEnvelope::ok("request-42", CommandResult::Acknowledged);
    assert_eq!(
        serde_json::to_value(response).unwrap()["requestId"],
        "request-42"
    );
}

#[test]
fn mutation_results_distinguish_offline_and_degraded_runtime_application() {
    let offline = CommandResult::Mutation(MutationResult {
        settings: Settings::default(),
        apply: None,
    });
    let offline_value = serde_json::to_value(&offline).unwrap();
    assert_eq!(offline_value["type"], "mutation");
    assert!(offline_value["payload"]["apply"].is_null());
    assert_eq!(
        serde_json::from_value::<CommandResult>(offline_value).unwrap(),
        offline
    );

    let degraded = CommandResult::Mutation(MutationResult {
        settings: Settings {
            revision: 2,
            ..Settings::default()
        },
        apply: Some(ApplyResult {
            desired_revision: 2,
            applied_revision: 1,
            status: ApplyStatus::Error,
            message: Some("candidate rejected".into()),
        }),
    });
    let degraded_value = serde_json::to_value(&degraded).unwrap();
    assert_eq!(degraded_value["payload"]["apply"]["status"], "error");
    assert_eq!(
        serde_json::to_value(RuntimeStatus::Degraded).unwrap(),
        "degraded"
    );
}

#[test]
fn snapshot_has_all_resynchronization_fields() {
    let snapshot = StateSnapshot {
        settings: Settings::default(),
        runtime_status: RuntimeStatus::Stopped,
        desired_revision: 2,
        applied_revision: 1,
        diagnostics: vec![Diagnostic {
            code: "ok".into(),
            status: DiagnosticStatus::Ok,
            message: "ready".into(),
            remedy: None,
        }],
        latest_event_sequence: 9,
    };
    let value = serde_json::to_value(snapshot).unwrap();
    for key in [
        "settings",
        "runtimeStatus",
        "desiredRevision",
        "appliedRevision",
        "diagnostics",
        "latestEventSequence",
    ] {
        assert!(value.get(key).is_some(), "missing {key}");
    }
}
