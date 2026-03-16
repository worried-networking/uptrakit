use super::*;
use clap::Parser;
use uptrakit_openapi_client::Uuid;

// Sub-enum re-imports for tests that destructure into nested variants.
use commands::autodiscovery::IgnoresCommands;
use commands::settings::{
    AuthenticationCommands, CertificateCommands, MqttCommands, MqttLimitCommands, NatsCommands,
    NetworkCommands, OidcCommands, RegistrationCommands,
};

/// Test UUID constants for readability.
const HOST_UUID: &str = "a1a2a3a4-b1b2-c1c2-d1d2-e1e2e3e4e5e6";
const ITEM_UUID: &str = "b1b2b3b4-c1c2-d1d2-e1e2-f1f2f3f4f5f6";
const SVC_UUID: &str = "c1c2c3c4-d1d2-e1e2-f1f2-a1a2a3a4a5a6";
const TASK_UUID: &str = "d1d2d3d4-e1e2-f1f2-a1a2-b1b2b3b4b5b6";
const HIST_UUID: &str = "e1e2e3e4-f1f2-a1a2-b1b2-c1c2c3c4c5c6";
const MQTT_UUID: &str = "01020304-0506-0708-090a-0b0c0d0e0f10";
const OIDC_UUID: &str = "11121314-1516-1718-191a-1b1c1d1e1f20";
const TARGET_UUID: &str = "aa000000-bb00-cc00-dd00-ee0000000001";
const SOURCE_UUID: &str = "aa000000-bb00-cc00-dd00-ee0000000002";
const PC_UUID: &str = "aa100000-bb00-cc00-dd00-ee0000000001";
const IGNORE_UUID: &str = "aa200000-bb00-cc00-dd00-ee0000000001";
const ET_UUID: &str = "aa300000-bb00-cc00-dd00-ee0000000001";
const SYS_ET_UUID: &str = "aa500000-bb00-cc00-dd00-ee0000000001";

/// Parse a UUID constant (safe in tests).
fn uuid(s: &str) -> Uuid {
    s.parse().expect("test UUID constant should be valid")
}

#[test]
fn verbose_flag_parses() {
    let args = Cli::try_parse_from(["uptrakit", "-v", "-v", "-v"]).expect("should parse -v flags");
    assert_eq!(args.verbose, 3);
}

#[test]
fn version_parses_without_subcommand() {
    let args = Cli::try_parse_from(["uptrakit", "--version"]).expect("should parse");
    assert!(args.version);
    assert!(args.command.is_none());
}

#[test]
fn hosts_list_parses() {
    let args = Cli::try_parse_from(["uptrakit", "hosts", "list"]).expect("should parse");
    assert!(matches!(
        args.command,
        Some(Commands::Hosts {
            command: HostsCommands::List { .. }
        })
    ));
}

#[test]
fn hosts_list_with_pagination() {
    let args = Cli::try_parse_from([
        "uptrakit",
        "hosts",
        "list",
        "--page",
        "2",
        "--per-page",
        "50",
    ])
    .expect("should parse");
    match args.command {
        Some(Commands::Hosts {
            command: HostsCommands::List { page, per_page },
        }) => {
            assert_eq!(page, Some(2));
            assert_eq!(per_page, Some(50));
        }
        _ => panic!("expected Hosts List"),
    }
}

#[test]
fn hosts_show_parses() {
    let args = Cli::try_parse_from(["uptrakit", "hosts", "show", HOST_UUID]).expect("should parse");
    match args.command {
        Some(Commands::Hosts {
            command: HostsCommands::Show { id },
        }) => {
            assert_eq!(id, uuid(HOST_UUID));
        }
        _ => panic!("expected Hosts Show"),
    }
}

#[test]
fn software_items_list_parses() {
    let args = Cli::try_parse_from(["uptrakit", "software-items", "list"]).expect("should parse");
    assert!(matches!(
        args.command,
        Some(Commands::SoftwareItems {
            command: SoftwareItemsCommands::List { .. }
        })
    ));
}

#[test]
fn software_items_show_parses() {
    let args = Cli::try_parse_from(["uptrakit", "software-items", "show", ITEM_UUID])
        .expect("should parse");
    match args.command {
        Some(Commands::SoftwareItems {
            command: SoftwareItemsCommands::Show { id },
        }) => {
            assert_eq!(id, uuid(ITEM_UUID));
        }
        _ => panic!("expected SoftwareItems Show"),
    }
}

#[test]
fn check_all_parses() {
    let args = Cli::try_parse_from(["uptrakit", "check", "all"]).expect("should parse");
    assert!(matches!(
        args.command,
        Some(Commands::Check {
            command: CheckCommands::All
        })
    ));
}

#[test]
fn check_item_parses() {
    let args = Cli::try_parse_from(["uptrakit", "check", "item", ITEM_UUID]).expect("should parse");
    match args.command {
        Some(Commands::Check {
            command: CheckCommands::Item { item_id, host },
        }) => {
            assert_eq!(item_id, uuid(ITEM_UUID));
            assert!(host.is_none());
        }
        _ => panic!("expected Check Item"),
    }
}

#[test]
fn check_item_with_host_parses() {
    let args = Cli::try_parse_from(["uptrakit", "check", "item", ITEM_UUID, "--host", HOST_UUID])
        .expect("should parse");
    match args.command {
        Some(Commands::Check {
            command: CheckCommands::Item { item_id, host },
        }) => {
            assert_eq!(item_id, uuid(ITEM_UUID));
            assert_eq!(host, Some(uuid(HOST_UUID)));
        }
        _ => panic!("expected Check Item"),
    }
}

#[test]
fn update_trigger_parses() {
    let args = Cli::try_parse_from([
        "uptrakit",
        "update",
        "trigger",
        ITEM_UUID,
        HOST_UUID,
        "--to-version",
        "2.0.0",
    ])
    .expect("should parse");
    match args.command {
        Some(Commands::Update {
            command:
                UpdateCommands::Trigger {
                    item_id,
                    host_id,
                    to_version,
                    release_tag,
                    release_url,
                    follow,
                    interactive,
                },
        }) => {
            assert_eq!(item_id, uuid(ITEM_UUID));
            assert_eq!(host_id, uuid(HOST_UUID));
            assert_eq!(to_version, "2.0.0");
            assert!(release_tag.is_none());
            assert!(release_url.is_none());
            assert!(!follow);
            assert!(!interactive);
        }
        _ => panic!("expected Update Trigger"),
    }
}

#[test]
fn update_trigger_with_release_info_parses() {
    let args = Cli::try_parse_from([
        "uptrakit",
        "update",
        "trigger",
        ITEM_UUID,
        HOST_UUID,
        "--to-version",
        "2.0.0",
        "--release-tag",
        "v2.0.0",
        "--release-url",
        "https://example.com/releases/v2.0.0",
    ])
    .expect("should parse");
    match args.command {
        Some(Commands::Update {
            command:
                UpdateCommands::Trigger {
                    release_tag,
                    release_url,
                    ..
                },
        }) => {
            assert_eq!(release_tag.as_deref(), Some("v2.0.0"));
            assert_eq!(
                release_url.as_deref(),
                Some("https://example.com/releases/v2.0.0")
            );
        }
        _ => panic!("expected Update Trigger"),
    }
}

#[test]
fn history_list_parses() {
    let args = Cli::try_parse_from(["uptrakit", "history", "list"]).expect("should parse");
    assert!(matches!(
        args.command,
        Some(Commands::History {
            command: HistoryCommands::List { .. }
        })
    ));
}

#[test]
fn history_list_with_filters() {
    let args = Cli::try_parse_from([
        "uptrakit",
        "history",
        "list",
        "--host",
        HOST_UUID,
        "--software-item",
        ITEM_UUID,
        "--status",
        "completed",
    ])
    .expect("should parse");
    match args.command {
        Some(Commands::History {
            command:
                HistoryCommands::List {
                    host,
                    software_item,
                    status,
                    ..
                },
        }) => {
            assert_eq!(host, Some(uuid(HOST_UUID)));
            assert_eq!(software_item, Some(uuid(ITEM_UUID)));
            assert_eq!(status.as_deref(), Some("completed"));
        }
        _ => panic!("expected History List"),
    }
}

#[test]
fn history_show_parses() {
    let args =
        Cli::try_parse_from(["uptrakit", "history", "show", HIST_UUID]).expect("should parse");
    match args.command {
        Some(Commands::History {
            command: HistoryCommands::Show { id },
        }) => {
            assert_eq!(id, uuid(HIST_UUID));
        }
        _ => panic!("expected History Show"),
    }
}

#[test]
fn history_tail_parses() {
    let args =
        Cli::try_parse_from(["uptrakit", "history", "tail", HIST_UUID]).expect("should parse");
    match args.command {
        Some(Commands::History {
            command: HistoryCommands::Tail { id },
        }) => {
            assert_eq!(id, uuid(HIST_UUID));
        }
        _ => panic!("expected History Tail"),
    }
}

#[test]
fn update_trigger_follow_flag_parses() {
    let args = Cli::try_parse_from([
        "uptrakit",
        "update",
        "trigger",
        ITEM_UUID,
        HOST_UUID,
        "--to-version",
        "2.0.0",
        "--follow",
    ])
    .expect("should parse");
    match args.command {
        Some(Commands::Update {
            command: UpdateCommands::Trigger { follow, .. },
        }) => {
            assert!(follow);
        }
        _ => panic!("expected Update Trigger with follow"),
    }
}

#[test]
fn update_trigger_interactive_flag_parses() {
    let args = Cli::try_parse_from([
        "uptrakit",
        "update",
        "trigger",
        ITEM_UUID,
        HOST_UUID,
        "--to-version",
        "2.0.0",
        "--interactive",
    ])
    .expect("should parse");
    match args.command {
        Some(Commands::Update {
            command: UpdateCommands::Trigger { interactive, .. },
        }) => {
            assert!(interactive);
        }
        _ => panic!("expected Update Trigger with interactive"),
    }
}

#[test]
fn update_batch_host_parses() {
    let args =
        Cli::try_parse_from(["uptrakit", "update", "batch-host", HOST_UUID]).expect("should parse");
    match args.command {
        Some(Commands::Update {
            command:
                UpdateCommands::BatchHost {
                    host_id,
                    category,
                    exclude,
                    follow,
                },
        }) => {
            assert_eq!(host_id, uuid(HOST_UUID));
            assert!(category.is_none());
            assert!(exclude.is_empty());
            assert!(!follow);
        }
        _ => panic!("expected Update BatchHost"),
    }
}

#[test]
fn update_batch_host_with_options_parses() {
    let args = Cli::try_parse_from([
        "uptrakit",
        "update",
        "batch-host",
        HOST_UUID,
        "--category",
        "security",
        "--exclude",
        ITEM_UUID,
        "--follow",
    ])
    .expect("should parse");
    match args.command {
        Some(Commands::Update {
            command:
                UpdateCommands::BatchHost {
                    category,
                    exclude,
                    follow,
                    ..
                },
        }) => {
            assert_eq!(category.as_deref(), Some("security"));
            assert_eq!(exclude.len(), 1);
            assert_eq!(exclude[0], uuid(ITEM_UUID));
            assert!(follow);
        }
        _ => panic!("expected Update BatchHost"),
    }
}

#[test]
fn update_batch_item_parses() {
    let args = Cli::try_parse_from([
        "uptrakit",
        "update",
        "batch-item",
        ITEM_UUID,
        "--to-version",
        "3.0.0",
    ])
    .expect("should parse");
    match args.command {
        Some(Commands::Update {
            command:
                UpdateCommands::BatchItem {
                    item_id,
                    to_version,
                    host,
                    follow,
                },
        }) => {
            assert_eq!(item_id, uuid(ITEM_UUID));
            assert_eq!(to_version, "3.0.0");
            assert!(host.is_empty());
            assert!(!follow);
        }
        _ => panic!("expected Update BatchItem"),
    }
}

#[test]
fn update_batch_item_with_hosts_parses() {
    let args = Cli::try_parse_from([
        "uptrakit",
        "update",
        "batch-item",
        ITEM_UUID,
        "--to-version",
        "3.0.0",
        "--host",
        HOST_UUID,
        "--follow",
    ])
    .expect("should parse");
    match args.command {
        Some(Commands::Update {
            command: UpdateCommands::BatchItem { host, follow, .. },
        }) => {
            assert_eq!(host.len(), 1);
            assert_eq!(host[0], uuid(HOST_UUID));
            assert!(follow);
        }
        _ => panic!("expected Update BatchItem"),
    }
}

// -- update-batches --

#[test]
fn update_batches_list_parses() {
    let args = Cli::try_parse_from(["uptrakit", "update-batches", "list"]).expect("should parse");
    assert!(matches!(
        args.command,
        Some(Commands::UpdateBatches {
            command: UpdateBatchesCommands::List { .. }
        })
    ));
}

#[test]
fn update_batches_list_with_filters() {
    let args = Cli::try_parse_from([
        "uptrakit",
        "update-batches",
        "list",
        "--status",
        "in_progress",
        "--page",
        "2",
        "--per-page",
        "10",
    ])
    .expect("should parse");
    match args.command {
        Some(Commands::UpdateBatches {
            command:
                UpdateBatchesCommands::List {
                    status,
                    page,
                    per_page,
                },
        }) => {
            assert_eq!(status.as_deref(), Some("in_progress"));
            assert_eq!(page, Some(2));
            assert_eq!(per_page, Some(10));
        }
        _ => panic!("expected UpdateBatches List"),
    }
}

#[test]
fn update_batches_show_parses() {
    let args = Cli::try_parse_from(["uptrakit", "update-batches", "show", HIST_UUID])
        .expect("should parse");
    match args.command {
        Some(Commands::UpdateBatches {
            command: UpdateBatchesCommands::Show { id },
        }) => {
            assert_eq!(id, uuid(HIST_UUID));
        }
        _ => panic!("expected UpdateBatches Show"),
    }
}

#[test]
fn update_batches_follow_parses() {
    let args = Cli::try_parse_from(["uptrakit", "update-batches", "follow", HIST_UUID])
        .expect("should parse");
    match args.command {
        Some(Commands::UpdateBatches {
            command: UpdateBatchesCommands::Follow { id },
        }) => {
            assert_eq!(id, uuid(HIST_UUID));
        }
        _ => panic!("expected UpdateBatches Follow"),
    }
}

#[test]
fn scheduler_list_parses() {
    let args = Cli::try_parse_from(["uptrakit", "scheduler", "list"]).expect("should parse");
    assert!(matches!(
        args.command,
        Some(Commands::Scheduler {
            command: SchedulerCommands::List
        })
    ));
}

#[test]
fn scheduler_show_parses() {
    let args =
        Cli::try_parse_from(["uptrakit", "scheduler", "show", TASK_UUID]).expect("should parse");
    match args.command {
        Some(Commands::Scheduler {
            command: SchedulerCommands::Show { id },
        }) => {
            assert_eq!(id, uuid(TASK_UUID));
        }
        _ => panic!("expected Scheduler Show"),
    }
}

#[test]
fn scheduler_trigger_parses() {
    let args =
        Cli::try_parse_from(["uptrakit", "scheduler", "trigger", TASK_UUID]).expect("should parse");
    match args.command {
        Some(Commands::Scheduler {
            command: SchedulerCommands::Trigger { id },
        }) => {
            assert_eq!(id, uuid(TASK_UUID));
        }
        _ => panic!("expected Scheduler Trigger"),
    }
}

#[test]
fn services_list_parses() {
    let args = Cli::try_parse_from(["uptrakit", "services", "list"]).expect("should parse");
    assert!(matches!(
        args.command,
        Some(Commands::Services {
            command: ServicesCommands::List { .. }
        })
    ));
}

#[test]
fn services_list_with_filters() {
    let args = Cli::try_parse_from([
        "uptrakit",
        "services",
        "list",
        "--capability",
        "software_discovery",
        "--status",
        "pending",
        "--page",
        "2",
        "--per-page",
        "50",
    ])
    .expect("should parse");
    match args.command {
        Some(Commands::Services {
            command:
                ServicesCommands::List {
                    capability,
                    status,
                    page,
                    per_page,
                },
        }) => {
            assert_eq!(capability.as_deref(), Some("software_discovery"));
            assert_eq!(status.as_deref(), Some("pending"));
            assert_eq!(page, Some(2));
            assert_eq!(per_page, Some(50));
        }
        _ => panic!("expected Services List"),
    }
}

#[test]
fn services_show_parses() {
    let args =
        Cli::try_parse_from(["uptrakit", "services", "show", SVC_UUID]).expect("should parse");
    match args.command {
        Some(Commands::Services {
            command: ServicesCommands::Show { id },
        }) => {
            assert_eq!(id, uuid(SVC_UUID));
        }
        _ => panic!("expected Services Show"),
    }
}

#[test]
fn services_approve_parses() {
    let args =
        Cli::try_parse_from(["uptrakit", "services", "approve", SVC_UUID]).expect("should parse");
    match args.command {
        Some(Commands::Services {
            command: ServicesCommands::Approve { id },
        }) => {
            assert_eq!(id, uuid(SVC_UUID));
        }
        _ => panic!("expected Services Approve"),
    }
}

#[test]
fn services_reject_parses() {
    let args =
        Cli::try_parse_from(["uptrakit", "services", "reject", SVC_UUID]).expect("should parse");
    match args.command {
        Some(Commands::Services {
            command: ServicesCommands::Reject { id },
        }) => {
            assert_eq!(id, uuid(SVC_UUID));
        }
        _ => panic!("expected Services Reject"),
    }
}

#[test]
fn services_remove_parses() {
    let args =
        Cli::try_parse_from(["uptrakit", "services", "remove", SVC_UUID]).expect("should parse");
    match args.command {
        Some(Commands::Services {
            command: ServicesCommands::Remove { id },
        }) => {
            assert_eq!(id, uuid(SVC_UUID));
        }
        _ => panic!("expected Services Remove"),
    }
}

#[test]
fn services_merge_parses() {
    let args = Cli::try_parse_from(["uptrakit", "services", "merge", TARGET_UUID, SOURCE_UUID])
        .expect("should parse");
    match args.command {
        Some(Commands::Services {
            command:
                ServicesCommands::Merge {
                    target_id,
                    source_id,
                },
        }) => {
            assert_eq!(target_id, uuid(TARGET_UUID));
            assert_eq!(source_id, uuid(SOURCE_UUID));
        }
        _ => panic!("expected Services Merge"),
    }
}

#[test]
fn global_options_parse_with_commands() {
    let args = Cli::try_parse_from([
        "uptrakit",
        "--server",
        "https://example.com",
        "--token",
        "my-token",
        "--insecure",
        "--output",
        "json",
        "hosts",
        "list",
    ])
    .expect("should parse");
    assert_eq!(args.server.as_deref(), Some("https://example.com"));
    assert_eq!(args.token.as_deref(), Some("my-token"));
    assert!(args.insecure);
    assert_eq!(args.output, OutputFormat::Json);
}

// -- Settings tests --

#[test]
fn settings_show_parses() {
    let args = Cli::try_parse_from(["uptrakit", "settings", "show"]).expect("should parse");
    assert!(matches!(
        args.command,
        Some(Commands::Settings {
            command: SettingsCommands::Show
        })
    ));
}

#[test]
fn settings_registration_show_parses() {
    let args = Cli::try_parse_from(["uptrakit", "settings", "registration", "show"])
        .expect("should parse");
    assert!(matches!(
        args.command,
        Some(Commands::Settings {
            command: SettingsCommands::Registration {
                command: RegistrationCommands::Show
            }
        })
    ));
}

#[test]
fn settings_registration_update_parses() {
    let args = Cli::try_parse_from([
        "uptrakit",
        "settings",
        "registration",
        "update",
        "--mode",
        "invite",
        "--token",
        "my-token",
        "--require-token-for-oidc",
        "true",
    ])
    .expect("should parse");
    match args.command {
        Some(Commands::Settings {
            command:
                SettingsCommands::Registration {
                    command:
                        RegistrationCommands::Update {
                            mode,
                            token,
                            require_token_for_oidc,
                        },
                },
        }) => {
            assert_eq!(
                mode,
                uptrakit_openapi_client::types::registration::RegistrationMode::Invite
            );
            assert_eq!(token.as_deref(), Some("my-token"));
            assert_eq!(require_token_for_oidc, Some(true));
        }
        _ => panic!("expected Settings Registration Update"),
    }
}

#[test]
fn settings_authentication_show_parses() {
    let args = Cli::try_parse_from(["uptrakit", "settings", "authentication", "show"])
        .expect("should parse");
    assert!(matches!(
        args.command,
        Some(Commands::Settings {
            command: SettingsCommands::Authentication {
                command: AuthenticationCommands::Show
            }
        })
    ));
}

#[test]
fn settings_authentication_update_parses() {
    let args = Cli::try_parse_from([
        "uptrakit",
        "settings",
        "authentication",
        "update",
        "--password-auth-enabled",
        "false",
    ])
    .expect("should parse");
    match args.command {
        Some(Commands::Settings {
            command:
                SettingsCommands::Authentication {
                    command:
                        AuthenticationCommands::Update {
                            password_auth_enabled,
                        },
                },
        }) => {
            assert_eq!(password_auth_enabled, Some(false));
        }
        _ => panic!("expected Settings Authentication Update"),
    }
}

#[test]
fn settings_certificates_show_parses() {
    let args = Cli::try_parse_from(["uptrakit", "settings", "certificates", "show"])
        .expect("should parse");
    assert!(matches!(
        args.command,
        Some(Commands::Settings {
            command: SettingsCommands::Certificates {
                command: CertificateCommands::Show
            }
        })
    ));
}

#[test]
fn settings_certificates_update_parses() {
    let args = Cli::try_parse_from([
        "uptrakit",
        "settings",
        "certificates",
        "update",
        "--lifetime-hours",
        "8760",
        "--renewal-window-hours",
        "72",
    ])
    .expect("should parse");
    match args.command {
        Some(Commands::Settings {
            command:
                SettingsCommands::Certificates {
                    command:
                        CertificateCommands::Update {
                            lifetime_hours,
                            renewal_window_hours,
                        },
                },
        }) => {
            assert_eq!(lifetime_hours, Some(8760));
            assert_eq!(renewal_window_hours, Some(72));
        }
        _ => panic!("expected Settings Certificates Update"),
    }
}

#[test]
fn settings_network_show_parses() {
    let args =
        Cli::try_parse_from(["uptrakit", "settings", "network", "show"]).expect("should parse");
    assert!(matches!(
        args.command,
        Some(Commands::Settings {
            command: SettingsCommands::Network {
                command: NetworkCommands::Show
            }
        })
    ));
}

#[test]
fn settings_network_update_parses() {
    let args = Cli::try_parse_from([
        "uptrakit",
        "settings",
        "network",
        "update",
        "--trusted-proxies",
        "10.0.0.0/8,172.16.0.0/12",
        "--real-ip-header",
        "X-Real-IP",
    ])
    .expect("should parse");
    match args.command {
        Some(Commands::Settings {
            command:
                SettingsCommands::Network {
                    command:
                        NetworkCommands::Update {
                            trusted_proxies,
                            real_ip_header,
                            ..
                        },
                },
        }) => {
            assert_eq!(trusted_proxies.as_deref(), Some("10.0.0.0/8,172.16.0.0/12"));
            assert_eq!(real_ip_header.as_deref(), Some("X-Real-IP"));
        }
        _ => panic!("expected Settings Network Update"),
    }
}

#[test]
fn settings_rotate_ca_parses() {
    let args = Cli::try_parse_from(["uptrakit", "settings", "rotate-ca"]).expect("should parse");
    assert!(matches!(
        args.command,
        Some(Commands::Settings {
            command: SettingsCommands::RotateCa
        })
    ));
}

#[test]
fn settings_renew_server_cert_parses() {
    let args =
        Cli::try_parse_from(["uptrakit", "settings", "renew-server-cert"]).expect("should parse");
    assert!(matches!(
        args.command,
        Some(Commands::Settings {
            command: SettingsCommands::RenewServerCert
        })
    ));
}

#[test]
fn settings_mqtt_list_parses() {
    let args = Cli::try_parse_from(["uptrakit", "settings", "mqtt", "list"]).expect("should parse");
    assert!(matches!(
        args.command,
        Some(Commands::Settings {
            command: SettingsCommands::Mqtt {
                command: MqttCommands::List
            }
        })
    ));
}

#[test]
fn settings_mqtt_show_parses() {
    let args = Cli::try_parse_from(["uptrakit", "settings", "mqtt", "show", MQTT_UUID])
        .expect("should parse");
    match args.command {
        Some(Commands::Settings {
            command:
                SettingsCommands::Mqtt {
                    command: MqttCommands::Show { id },
                },
        }) => {
            assert_eq!(id, uuid(MQTT_UUID));
        }
        _ => panic!("expected Settings Mqtt Show"),
    }
}

#[test]
fn settings_mqtt_create_parses() {
    let args = Cli::try_parse_from([
        "uptrakit",
        "settings",
        "mqtt",
        "create",
        "--url",
        "mqtt://broker:1883",
        "--enabled",
        "true",
        "--client-id",
        "uptrakit-1",
    ])
    .expect("should parse");
    match args.command {
        Some(Commands::Settings {
            command:
                SettingsCommands::Mqtt {
                    command:
                        MqttCommands::Create {
                            url,
                            enabled,
                            client_id,
                            ..
                        },
                },
        }) => {
            assert_eq!(url.as_deref(), Some("mqtt://broker:1883"));
            assert_eq!(enabled, Some(true));
            assert_eq!(client_id.as_deref(), Some("uptrakit-1"));
        }
        _ => panic!("expected Settings Mqtt Create"),
    }
}

#[test]
fn settings_mqtt_update_parses() {
    let args = Cli::try_parse_from([
        "uptrakit",
        "settings",
        "mqtt",
        "update",
        MQTT_UUID,
        "--enabled",
        "false",
        "--host",
        "new-broker",
    ])
    .expect("should parse");
    match args.command {
        Some(Commands::Settings {
            command:
                SettingsCommands::Mqtt {
                    command:
                        MqttCommands::Update {
                            id, enabled, host, ..
                        },
                },
        }) => {
            assert_eq!(id, uuid(MQTT_UUID));
            assert_eq!(enabled, Some(false));
            assert_eq!(host.as_deref(), Some("new-broker"));
        }
        _ => panic!("expected Settings Mqtt Update"),
    }
}

#[test]
fn settings_mqtt_delete_parses() {
    let args = Cli::try_parse_from(["uptrakit", "settings", "mqtt", "delete", MQTT_UUID])
        .expect("should parse");
    match args.command {
        Some(Commands::Settings {
            command:
                SettingsCommands::Mqtt {
                    command: MqttCommands::Delete { id },
                },
        }) => {
            assert_eq!(id, uuid(MQTT_UUID));
        }
        _ => panic!("expected Settings Mqtt Delete"),
    }
}

#[test]
fn settings_mqtt_limit_show_parses() {
    let args = Cli::try_parse_from(["uptrakit", "settings", "mqtt", "limit", "show"])
        .expect("should parse");
    assert!(matches!(
        args.command,
        Some(Commands::Settings {
            command: SettingsCommands::Mqtt {
                command: MqttCommands::Limit {
                    command: MqttLimitCommands::Show
                }
            }
        })
    ));
}

#[test]
fn settings_mqtt_limit_update_parses() {
    let args = Cli::try_parse_from([
        "uptrakit", "settings", "mqtt", "limit", "update", "--max", "10",
    ])
    .expect("should parse");
    match args.command {
        Some(Commands::Settings {
            command:
                SettingsCommands::Mqtt {
                    command:
                        MqttCommands::Limit {
                            command: MqttLimitCommands::Update { max },
                        },
                },
        }) => {
            assert_eq!(max, 10);
        }
        _ => panic!("expected Settings Mqtt Limit Update"),
    }
}

#[test]
fn settings_oidc_list_parses() {
    let args = Cli::try_parse_from(["uptrakit", "settings", "oidc", "list"]).expect("should parse");
    assert!(matches!(
        args.command,
        Some(Commands::Settings {
            command: SettingsCommands::Oidc {
                command: OidcCommands::List
            }
        })
    ));
}

#[test]
fn settings_oidc_show_parses() {
    let args = Cli::try_parse_from(["uptrakit", "settings", "oidc", "show", OIDC_UUID])
        .expect("should parse");
    match args.command {
        Some(Commands::Settings {
            command:
                SettingsCommands::Oidc {
                    command: OidcCommands::Show { id },
                },
        }) => {
            assert_eq!(id, uuid(OIDC_UUID));
        }
        _ => panic!("expected Settings Oidc Show"),
    }
}

#[test]
fn settings_oidc_create_parses() {
    let args = Cli::try_parse_from([
        "uptrakit",
        "settings",
        "oidc",
        "create",
        "--name",
        "Google",
        "--slug",
        "google",
        "--issuer-url",
        "https://accounts.google.com",
        "--client-id",
        "cid-123",
        "--client-secret",
        "cs-456",
    ])
    .expect("should parse");
    match args.command {
        Some(Commands::Settings {
            command:
                SettingsCommands::Oidc {
                    command:
                        OidcCommands::Create {
                            name,
                            slug,
                            issuer_url,
                            client_id,
                            client_secret,
                            ..
                        },
                },
        }) => {
            assert_eq!(name, "Google");
            assert_eq!(slug, "google");
            assert_eq!(issuer_url, "https://accounts.google.com");
            assert_eq!(client_id, "cid-123");
            assert_eq!(client_secret, "cs-456");
        }
        _ => panic!("expected Settings Oidc Create"),
    }
}

#[test]
fn settings_oidc_update_parses() {
    let args = Cli::try_parse_from([
        "uptrakit",
        "settings",
        "oidc",
        "update",
        OIDC_UUID,
        "--name",
        "Google Workspace",
        "--auto-create-users",
        "false",
    ])
    .expect("should parse");
    match args.command {
        Some(Commands::Settings {
            command:
                SettingsCommands::Oidc {
                    command:
                        OidcCommands::Update {
                            id,
                            name,
                            auto_create_users,
                            ..
                        },
                },
        }) => {
            assert_eq!(id, uuid(OIDC_UUID));
            assert_eq!(name.as_deref(), Some("Google Workspace"));
            assert_eq!(auto_create_users, Some(false));
        }
        _ => panic!("expected Settings Oidc Update"),
    }
}

#[test]
fn settings_oidc_delete_parses() {
    let args = Cli::try_parse_from(["uptrakit", "settings", "oidc", "delete", OIDC_UUID])
        .expect("should parse");
    match args.command {
        Some(Commands::Settings {
            command:
                SettingsCommands::Oidc {
                    command: OidcCommands::Delete { id },
                },
        }) => {
            assert_eq!(id, uuid(OIDC_UUID));
        }
        _ => panic!("expected Settings Oidc Delete"),
    }
}

#[test]
fn settings_oidc_activate_parses() {
    let args = Cli::try_parse_from(["uptrakit", "settings", "oidc", "activate", OIDC_UUID])
        .expect("should parse");
    match args.command {
        Some(Commands::Settings {
            command:
                SettingsCommands::Oidc {
                    command: OidcCommands::Activate { id },
                },
        }) => {
            assert_eq!(id, uuid(OIDC_UUID));
        }
        _ => panic!("expected Settings Oidc Activate"),
    }
}

#[test]
fn settings_oidc_deactivate_parses() {
    let args = Cli::try_parse_from(["uptrakit", "settings", "oidc", "deactivate", OIDC_UUID])
        .expect("should parse");
    match args.command {
        Some(Commands::Settings {
            command:
                SettingsCommands::Oidc {
                    command: OidcCommands::Deactivate { id },
                },
        }) => {
            assert_eq!(id, uuid(OIDC_UUID));
        }
        _ => panic!("expected Settings Oidc Deactivate"),
    }
}

#[test]
fn settings_alerts_parses() {
    let args = Cli::try_parse_from(["uptrakit", "settings", "alerts"]).expect("should parse");
    assert!(matches!(
        args.command,
        Some(Commands::Settings {
            command: SettingsCommands::Alerts
        })
    ));
}

#[test]
fn settings_nats_show_parses() {
    let args = Cli::try_parse_from(["uptrakit", "settings", "nats", "show"]).expect("should parse");
    assert!(matches!(
        args.command,
        Some(Commands::Settings {
            command: SettingsCommands::Nats {
                command: NatsCommands::Show
            }
        })
    ));
}

#[test]
fn settings_nats_set_parses() {
    let args = Cli::try_parse_from([
        "uptrakit",
        "settings",
        "nats",
        "set",
        "--url",
        "nats://host:4222",
    ])
    .expect("should parse");
    match args.command {
        Some(Commands::Settings {
            command:
                SettingsCommands::Nats {
                    command: NatsCommands::Set { url },
                },
        }) => {
            assert_eq!(url, "nats://host:4222");
        }
        _ => panic!("expected Settings Nats Set"),
    }
}

#[test]
fn settings_nats_clear_parses() {
    let args =
        Cli::try_parse_from(["uptrakit", "settings", "nats", "clear"]).expect("should parse");
    assert!(matches!(
        args.command,
        Some(Commands::Settings {
            command: SettingsCommands::Nats {
                command: NatsCommands::Clear
            }
        })
    ));
}

#[test]
fn settings_reset_data_parses() {
    let args = Cli::try_parse_from(["uptrakit", "settings", "reset-data", "--confirm"])
        .expect("should parse");
    assert!(matches!(
        args.command,
        Some(Commands::Settings {
            command: SettingsCommands::ResetData { confirm: true }
        })
    ));
}

#[test]
fn settings_reset_data_without_confirm_parses() {
    let args = Cli::try_parse_from(["uptrakit", "settings", "reset-data"]).expect("should parse");
    assert!(matches!(
        args.command,
        Some(Commands::Settings {
            command: SettingsCommands::ResetData { confirm: false }
        })
    ));
}

#[test]
fn hosts_update_parses() {
    let args = Cli::try_parse_from([
        "uptrakit",
        "hosts",
        "update",
        HOST_UUID,
        "--friendly-name",
        "My Server",
    ])
    .expect("should parse");
    match args.command {
        Some(Commands::Hosts {
            command: HostsCommands::Update { id, friendly_name },
        }) => {
            assert_eq!(id, uuid(HOST_UUID));
            assert_eq!(friendly_name.as_deref(), Some("My Server"));
        }
        _ => panic!("expected Hosts Update"),
    }
}

#[test]
fn hosts_deactivate_parses() {
    let args =
        Cli::try_parse_from(["uptrakit", "hosts", "deactivate", HOST_UUID]).expect("should parse");
    match args.command {
        Some(Commands::Hosts {
            command: HostsCommands::Deactivate { id },
        }) => {
            assert_eq!(id, uuid(HOST_UUID));
        }
        _ => panic!("expected Hosts Deactivate"),
    }
}

#[test]
fn hosts_discover_parses() {
    let args =
        Cli::try_parse_from(["uptrakit", "hosts", "discover", HOST_UUID]).expect("should parse");
    match args.command {
        Some(Commands::Hosts {
            command: HostsCommands::Discover { id },
        }) => {
            assert_eq!(id, uuid(HOST_UUID));
        }
        _ => panic!("expected Hosts Discover"),
    }
}

#[test]
fn software_items_create_parses() {
    let args = Cli::try_parse_from(["uptrakit", "software-items", "create", "--name", "My App"])
        .expect("should parse");
    match args.command {
        Some(Commands::SoftwareItems {
            command: SoftwareItemsCommands::Create { name, featured, .. },
        }) => {
            assert_eq!(name, "My App");
            assert!(featured.is_none());
        }
        _ => panic!("expected SoftwareItems Create"),
    }
}

#[test]
fn software_items_update_parses() {
    let args = Cli::try_parse_from([
        "uptrakit",
        "software-items",
        "update",
        ITEM_UUID,
        "--name",
        "Updated App",
        "--featured",
        "false",
    ])
    .expect("should parse");
    match args.command {
        Some(Commands::SoftwareItems {
            command:
                SoftwareItemsCommands::Update {
                    id, name, featured, ..
                },
        }) => {
            assert_eq!(id, uuid(ITEM_UUID));
            assert_eq!(name.as_deref(), Some("Updated App"));
            assert_eq!(featured, Some(false));
        }
        _ => panic!("expected SoftwareItems Update"),
    }
}

#[test]
fn software_items_delete_parses() {
    let args = Cli::try_parse_from(["uptrakit", "software-items", "delete", ITEM_UUID])
        .expect("should parse");
    match args.command {
        Some(Commands::SoftwareItems {
            command: SoftwareItemsCommands::Delete { id },
        }) => {
            assert_eq!(id, uuid(ITEM_UUID));
        }
        _ => panic!("expected SoftwareItems Delete"),
    }
}

#[test]
fn software_items_approve_parses() {
    let args = Cli::try_parse_from(["uptrakit", "software-items", "approve", ITEM_UUID])
        .expect("should parse");
    match args.command {
        Some(Commands::SoftwareItems {
            command: SoftwareItemsCommands::Approve { id },
        }) => {
            assert_eq!(id, uuid(ITEM_UUID));
        }
        _ => panic!("expected SoftwareItems Approve"),
    }
}

#[test]
fn software_items_assign_parses() {
    let args = Cli::try_parse_from([
        "uptrakit",
        "software-items",
        "assign",
        ITEM_UUID,
        "--host",
        HOST_UUID,
        "--plugin-config",
        PC_UUID,
        "--package",
        "org/app",
    ])
    .expect("should parse");
    match args.command {
        Some(Commands::SoftwareItems {
            command:
                SoftwareItemsCommands::Assign {
                    id,
                    host,
                    plugin_config,
                    package,
                },
        }) => {
            assert_eq!(id, uuid(ITEM_UUID));
            assert_eq!(host, uuid(HOST_UUID));
            assert_eq!(plugin_config, Some(uuid(PC_UUID)));
            assert_eq!(package.as_deref(), Some("org/app"));
        }
        _ => panic!("expected SoftwareItems Assign"),
    }
}

#[test]
fn software_items_unassign_parses() {
    let args = Cli::try_parse_from([
        "uptrakit",
        "software-items",
        "unassign",
        ITEM_UUID,
        "--host",
        HOST_UUID,
    ])
    .expect("should parse");
    match args.command {
        Some(Commands::SoftwareItems {
            command: SoftwareItemsCommands::Unassign { id, host, ignore },
        }) => {
            assert_eq!(id, uuid(ITEM_UUID));
            assert_eq!(host, uuid(HOST_UUID));
            assert!(!ignore);
        }
        _ => panic!("expected SoftwareItems Unassign"),
    }
}

#[test]
fn software_items_unassign_with_ignore_parses() {
    let args = Cli::try_parse_from([
        "uptrakit",
        "software-items",
        "unassign",
        ITEM_UUID,
        "--host",
        HOST_UUID,
        "--ignore",
    ])
    .expect("should parse");
    match args.command {
        Some(Commands::SoftwareItems {
            command: SoftwareItemsCommands::Unassign { ignore, .. },
        }) => {
            assert!(ignore);
        }
        _ => panic!("expected SoftwareItems Unassign"),
    }
}

#[test]
fn plugin_configs_list_parses() {
    let args = Cli::try_parse_from(["uptrakit", "plugin-configs", "list"]).expect("should parse");
    assert!(matches!(
        args.command,
        Some(Commands::PluginConfigs {
            command: PluginConfigsCommands::List { .. }
        })
    ));
}

#[test]
fn plugin_configs_show_parses() {
    let args =
        Cli::try_parse_from(["uptrakit", "plugin-configs", "show", PC_UUID]).expect("should parse");
    match args.command {
        Some(Commands::PluginConfigs {
            command: PluginConfigsCommands::Show { id },
        }) => {
            assert_eq!(id, uuid(PC_UUID));
        }
        _ => panic!("expected PluginConfigs Show"),
    }
}

#[test]
fn plugin_configs_create_parses() {
    let args = Cli::try_parse_from([
        "uptrakit",
        "plugin-configs",
        "create",
        "--name",
        "My GitHub",
        "--plugin-type",
        "releases_github",
        "--config",
        r#"{"tag_strip_prefix":"v"}"#,
    ])
    .expect("should parse");
    match args.command {
        Some(Commands::PluginConfigs {
            command:
                PluginConfigsCommands::Create {
                    name, plugin_type, ..
                },
        }) => {
            assert_eq!(name, "My GitHub");
            assert_eq!(plugin_type, "releases_github");
        }
        _ => panic!("expected PluginConfigs Create"),
    }
}

#[test]
fn plugin_configs_delete_parses() {
    let args = Cli::try_parse_from(["uptrakit", "plugin-configs", "delete", PC_UUID])
        .expect("should parse");
    match args.command {
        Some(Commands::PluginConfigs {
            command: PluginConfigsCommands::Delete { id },
        }) => {
            assert_eq!(id, uuid(PC_UUID));
        }
        _ => panic!("expected PluginConfigs Delete"),
    }
}

#[test]
fn plugin_configs_discover_parses() {
    let args = Cli::try_parse_from(["uptrakit", "plugin-configs", "discover", PC_UUID])
        .expect("should parse");
    match args.command {
        Some(Commands::PluginConfigs {
            command: PluginConfigsCommands::Discover { id },
        }) => {
            assert_eq!(id, uuid(PC_UUID));
        }
        _ => panic!("expected PluginConfigs Discover"),
    }
}

#[test]
fn parse_plugin_type_settings_list() {
    let args =
        Cli::try_parse_from(["uptrakit", "plugin-type-settings", "list"]).expect("should parse");
    assert!(matches!(
        args.command,
        Some(Commands::PluginTypeSettings {
            command: PluginTypeSettingsCommands::List
        })
    ));
}

#[test]
fn parse_plugin_type_settings_show() {
    let args = Cli::try_parse_from([
        "uptrakit",
        "plugin-type-settings",
        "show",
        "releases_github",
    ])
    .expect("should parse");
    match args.command {
        Some(Commands::PluginTypeSettings {
            command: PluginTypeSettingsCommands::Show { plugin_type },
        }) => {
            assert_eq!(plugin_type, "releases_github");
        }
        _ => panic!("expected PluginTypeSettings Show"),
    }
}

#[test]
fn parse_plugin_type_settings_set() {
    let args = Cli::try_parse_from([
        "uptrakit",
        "plugin-type-settings",
        "set",
        "releases_github",
        "--config",
        r#"{"poll_interval_secs":300}"#,
    ])
    .expect("should parse");
    match args.command {
        Some(Commands::PluginTypeSettings {
            command:
                PluginTypeSettingsCommands::Set {
                    plugin_type,
                    config,
                },
        }) => {
            assert_eq!(plugin_type, "releases_github");
            assert_eq!(config, r#"{"poll_interval_secs":300}"#);
        }
        _ => panic!("expected PluginTypeSettings Set"),
    }
}

#[test]
fn parse_plugin_type_settings_reset() {
    let args = Cli::try_parse_from([
        "uptrakit",
        "plugin-type-settings",
        "reset",
        "releases_github",
    ])
    .expect("should parse");
    match args.command {
        Some(Commands::PluginTypeSettings {
            command: PluginTypeSettingsCommands::Reset { plugin_type },
        }) => {
            assert_eq!(plugin_type, "releases_github");
        }
        _ => panic!("expected PluginTypeSettings Reset"),
    }
}

#[test]
fn enrollment_tokens_list_parses() {
    let args =
        Cli::try_parse_from(["uptrakit", "enrollment-tokens", "list"]).expect("should parse");
    assert!(matches!(
        args.command,
        Some(Commands::EnrollmentTokens {
            command: EnrollmentTokensCommands::List {
                page: None,
                per_page: None
            }
        })
    ));
}

#[test]
fn enrollment_tokens_list_with_pagination() {
    let args = Cli::try_parse_from([
        "uptrakit",
        "enrollment-tokens",
        "list",
        "--page",
        "2",
        "--per-page",
        "50",
    ])
    .expect("should parse");
    match args.command {
        Some(Commands::EnrollmentTokens {
            command: EnrollmentTokensCommands::List { page, per_page },
        }) => {
            assert_eq!(page, Some(2));
            assert_eq!(per_page, Some(50));
        }
        _ => panic!("expected EnrollmentTokens List"),
    }
}

#[test]
fn enrollment_tokens_create_parses() {
    let args = Cli::try_parse_from([
        "uptrakit",
        "enrollment-tokens",
        "create",
        "--name",
        "CI Token",
        "--capabilities",
        "software_discovery,update_tracking",
        "--max-uses",
        "10",
        "--expires-in",
        "86400",
    ])
    .expect("should parse");
    match args.command {
        Some(Commands::EnrollmentTokens {
            command:
                EnrollmentTokensCommands::Create {
                    name,
                    capabilities,
                    max_uses,
                    expires_in,
                },
        }) => {
            assert_eq!(name, "CI Token");
            assert_eq!(
                capabilities.as_deref(),
                Some("software_discovery,update_tracking")
            );
            assert_eq!(max_uses, Some(10));
            assert_eq!(expires_in, Some(86400));
        }
        _ => panic!("expected EnrollmentTokens Create"),
    }
}

#[test]
fn enrollment_tokens_create_minimal() {
    let args = Cli::try_parse_from([
        "uptrakit",
        "enrollment-tokens",
        "create",
        "--name",
        "Wildcard",
    ])
    .expect("should parse");
    match args.command {
        Some(Commands::EnrollmentTokens {
            command:
                EnrollmentTokensCommands::Create {
                    name,
                    capabilities,
                    max_uses,
                    expires_in,
                },
        }) => {
            assert_eq!(name, "Wildcard");
            assert!(capabilities.is_none());
            assert!(max_uses.is_none());
            assert!(expires_in.is_none());
        }
        _ => panic!("expected EnrollmentTokens Create"),
    }
}

#[test]
fn enrollment_tokens_show_parses() {
    let args = Cli::try_parse_from(["uptrakit", "enrollment-tokens", "show", ET_UUID])
        .expect("should parse");
    match args.command {
        Some(Commands::EnrollmentTokens {
            command: EnrollmentTokensCommands::Show { id },
        }) => {
            assert_eq!(id, uuid(ET_UUID));
        }
        _ => panic!("expected EnrollmentTokens Show"),
    }
}

#[test]
fn enrollment_tokens_revoke_parses() {
    let args = Cli::try_parse_from(["uptrakit", "enrollment-tokens", "revoke", ET_UUID])
        .expect("should parse");
    match args.command {
        Some(Commands::EnrollmentTokens {
            command: EnrollmentTokensCommands::Revoke { id },
        }) => {
            assert_eq!(id, uuid(ET_UUID));
        }
        _ => panic!("expected EnrollmentTokens Revoke"),
    }
}

#[test]
fn system_enrollment_tokens_list_parses() {
    let args = Cli::try_parse_from(["uptrakit", "system-enrollment-tokens", "list"])
        .expect("should parse");
    assert!(matches!(
        args.command,
        Some(Commands::SystemEnrollmentTokens {
            command: SystemEnrollmentTokensCommands::List {
                page: None,
                per_page: None
            }
        })
    ));
}

#[test]
fn system_enrollment_tokens_create_parses() {
    let args = Cli::try_parse_from([
        "uptrakit",
        "system-enrollment-tokens",
        "create",
        "--name",
        "MQTT Bridge Token",
        "--max-uses",
        "5",
        "--expires-in",
        "86400",
    ])
    .expect("should parse");
    match args.command {
        Some(Commands::SystemEnrollmentTokens {
            command:
                SystemEnrollmentTokensCommands::Create {
                    name,
                    max_uses,
                    expires_in,
                },
        }) => {
            assert_eq!(name, "MQTT Bridge Token");
            assert_eq!(max_uses, Some(5));
            assert_eq!(expires_in, Some(86400));
        }
        _ => panic!("expected SystemEnrollmentTokens Create"),
    }
}

#[test]
fn system_enrollment_tokens_create_minimal() {
    let args = Cli::try_parse_from([
        "uptrakit",
        "system-enrollment-tokens",
        "create",
        "--name",
        "Unlimited",
    ])
    .expect("should parse");
    match args.command {
        Some(Commands::SystemEnrollmentTokens {
            command:
                SystemEnrollmentTokensCommands::Create {
                    name,
                    max_uses,
                    expires_in,
                },
        }) => {
            assert_eq!(name, "Unlimited");
            assert!(max_uses.is_none());
            assert!(expires_in.is_none());
        }
        _ => panic!("expected SystemEnrollmentTokens Create"),
    }
}

#[test]
fn system_enrollment_tokens_show_parses() {
    let args = Cli::try_parse_from(["uptrakit", "system-enrollment-tokens", "show", SYS_ET_UUID])
        .expect("should parse");
    match args.command {
        Some(Commands::SystemEnrollmentTokens {
            command: SystemEnrollmentTokensCommands::Show { id },
        }) => {
            assert_eq!(id, uuid(SYS_ET_UUID));
        }
        _ => panic!("expected SystemEnrollmentTokens Show"),
    }
}

#[test]
fn system_enrollment_tokens_revoke_parses() {
    let args = Cli::try_parse_from([
        "uptrakit",
        "system-enrollment-tokens",
        "revoke",
        SYS_ET_UUID,
    ])
    .expect("should parse");
    match args.command {
        Some(Commands::SystemEnrollmentTokens {
            command: SystemEnrollmentTokensCommands::Revoke { id },
        }) => {
            assert_eq!(id, uuid(SYS_ET_UUID));
        }
        _ => panic!("expected SystemEnrollmentTokens Revoke"),
    }
}

#[test]
fn autodiscovery_ignores_list_parses() {
    let args = Cli::try_parse_from(["uptrakit", "autodiscovery", "ignores", "list"])
        .expect("should parse");
    assert!(matches!(
        args.command,
        Some(Commands::Autodiscovery {
            command: AutodiscoveryCommands::Ignores {
                command: IgnoresCommands::List { .. }
            }
        })
    ));
}

#[test]
fn autodiscovery_ignores_create_parses() {
    let args = Cli::try_parse_from([
        "uptrakit",
        "autodiscovery",
        "ignores",
        "create",
        "--name",
        "FreshRSS",
    ])
    .expect("should parse");
    match args.command {
        Some(Commands::Autodiscovery {
            command:
                AutodiscoveryCommands::Ignores {
                    command: IgnoresCommands::Create { name },
                },
        }) => {
            assert_eq!(name, "FreshRSS");
        }
        _ => panic!("expected Autodiscovery Ignores Create"),
    }
}

#[test]
fn autodiscovery_ignores_delete_parses() {
    let args = Cli::try_parse_from([
        "uptrakit",
        "autodiscovery",
        "ignores",
        "delete",
        IGNORE_UUID,
    ])
    .expect("should parse");
    match args.command {
        Some(Commands::Autodiscovery {
            command:
                AutodiscoveryCommands::Ignores {
                    command: IgnoresCommands::Delete { id },
                },
        }) => {
            assert_eq!(id, uuid(IGNORE_UUID));
        }
        _ => panic!("expected Autodiscovery Ignores Delete"),
    }
}

#[test]
fn settings_registration_update_rejects_invalid_mode() {
    let result = Cli::try_parse_from([
        "uptrakit",
        "settings",
        "registration",
        "update",
        "--mode",
        "invalid",
    ]);
    assert!(result.is_err());
}

#[test]
fn rejects_invalid_uuid_for_id_arguments() {
    let result = Cli::try_parse_from(["uptrakit", "hosts", "show", "not-a-uuid"]);
    assert!(result.is_err());
}

#[test]
fn global_timeout_parses() {
    let args = Cli::try_parse_from(["uptrakit", "--timeout", "60", "hosts", "list"])
        .expect("should parse");
    assert_eq!(args.timeout, Some(60));
}

#[test]
fn global_timeout_defaults_to_none() {
    let args = Cli::try_parse_from(["uptrakit", "hosts", "list"]).expect("should parse");
    assert!(args.timeout.is_none());
}

// -- Users commands --

#[test]
fn users_list_parses() {
    let args = Cli::try_parse_from(["uptrakit", "users", "list"]).expect("should parse");
    assert!(matches!(
        args.command,
        Some(Commands::Users {
            command: UsersCommands::List
        })
    ));
}

#[test]
fn users_show_parses() {
    let args = Cli::try_parse_from(["uptrakit", "users", "show", SVC_UUID]).expect("should parse");
    match args.command {
        Some(Commands::Users {
            command: UsersCommands::Show { id },
        }) => assert_eq!(id, uuid(SVC_UUID)),
        _ => panic!("expected Users Show"),
    }
}

#[test]
fn users_set_roles_parses() {
    let args = Cli::try_parse_from([
        "uptrakit",
        "users",
        "set-roles",
        SVC_UUID,
        HOST_UUID,
        ITEM_UUID,
    ])
    .expect("should parse");
    match args.command {
        Some(Commands::Users {
            command: UsersCommands::SetRoles { id, role_ids },
        }) => {
            assert_eq!(id, uuid(SVC_UUID));
            assert_eq!(role_ids.len(), 2);
            assert_eq!(role_ids[0], uuid(HOST_UUID));
            assert_eq!(role_ids[1], uuid(ITEM_UUID));
        }
        _ => panic!("expected Users SetRoles"),
    }
}

#[test]
fn users_activate_parses() {
    let args =
        Cli::try_parse_from(["uptrakit", "users", "activate", SVC_UUID]).expect("should parse");
    assert!(matches!(
        args.command,
        Some(Commands::Users {
            command: UsersCommands::Activate { .. }
        })
    ));
}

#[test]
fn users_deactivate_parses() {
    let args =
        Cli::try_parse_from(["uptrakit", "users", "deactivate", SVC_UUID]).expect("should parse");
    assert!(matches!(
        args.command,
        Some(Commands::Users {
            command: UsersCommands::Deactivate { .. }
        })
    ));
}

#[test]
fn users_apply_preset_parses() {
    let args = Cli::try_parse_from(["uptrakit", "users", "apply-preset", SVC_UUID, "admin"])
        .expect("should parse");
    match args.command {
        Some(Commands::Users {
            command: UsersCommands::ApplyPreset { id, preset },
        }) => {
            assert_eq!(id, uuid(SVC_UUID));
            assert_eq!(preset, "admin");
        }
        _ => panic!("expected Users ApplyPreset"),
    }
}

// -- Roles commands --

#[test]
fn roles_list_parses() {
    let args = Cli::try_parse_from(["uptrakit", "roles", "list"]).expect("should parse");
    assert!(matches!(
        args.command,
        Some(Commands::Roles {
            command: RolesCommands::List
        })
    ));
}

#[test]
fn roles_show_parses() {
    let args = Cli::try_parse_from(["uptrakit", "roles", "show", SVC_UUID]).expect("should parse");
    match args.command {
        Some(Commands::Roles {
            command: RolesCommands::Show { id },
        }) => assert_eq!(id, uuid(SVC_UUID)),
        _ => panic!("expected Roles Show"),
    }
}

// -- Access Presets commands --

#[test]
fn access_presets_list_parses() {
    let args = Cli::try_parse_from(["uptrakit", "access-presets", "list"]).expect("should parse");
    assert!(matches!(
        args.command,
        Some(Commands::AccessPresets {
            command: AccessPresetsCommands::List
        })
    ));
}
