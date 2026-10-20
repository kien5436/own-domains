use own_domains_cli::{Cli, CliCommand, execute};

#[test]
fn parses_complete_command_surface() {
    for command in ["start", "stop", "status", "list", "doctor"] {
        assert!(
            Cli::try_parse_from(["own-domains", command]).is_ok(),
            "{command}"
        );
    }
    let add = Cli::try_parse_from([
        "own-domains",
        "add",
        "shop.own",
        "--port",
        "3000",
        "--revision",
        "0",
        "--https",
    ])
    .unwrap();
    assert!(matches!(add.command, CliCommand::Add { https: true, .. }));
    for command in ["edit", "enable", "disable", "remove"] {
        let mut args = vec!["own-domains", command, "shop.own", "--revision", "0"];
        if command == "edit" {
            args.extend(["--port", "4000"]);
        }
        assert!(Cli::try_parse_from(args).is_ok(), "{command}");
    }
}

#[test]
fn parser_rejects_missing_values_and_unknown_flags() {
    assert!(Cli::try_parse_from(["own-domains", "add", "shop.own", "--port", "3000"]).is_err());
    assert!(Cli::try_parse_from(["own-domains", "list", "--unknown"]).is_err());
}

#[test]
fn offline_status_and_mutation_are_truthful() {
    let state_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("target")
        .join(format!("od-cli-{}", &uuid::Uuid::new_v4().to_string()[..8]));
    let status = execute(
        Cli::try_parse_from([
            "own-domains",
            "--state-dir",
            state_dir.to_str().unwrap(),
            "status",
        ])
        .unwrap(),
    );
    assert_eq!(status.exit_code, 0);
    assert!(status.stdout.contains("stopped"));
    let add = execute(
        Cli::try_parse_from([
            "own-domains",
            "--state-dir",
            state_dir.to_str().unwrap(),
            "add",
            "shop.own",
            "--port",
            "3000",
            "--revision",
            "0",
        ])
        .unwrap(),
    );
    assert_eq!(add.exit_code, 0);
    assert!(add.stdout.contains("saved; runtime stopped"));
}
