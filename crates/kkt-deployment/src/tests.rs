use super::*;
use std::path::PathBuf;

fn recorded(deployment: Deployment) -> Recorded {
    Recorded {
        deployment,
        url: "https://kkt.tail1234.ts.net/mcp".into(),
        key_file: Some("/home/sam/.config/kkt/docker-access-key".into()),
        clients_url: None,
        version: ServerVersion::Known("0.4.0".into()),
    }
}

fn docker() -> Deployment {
    Deployment::Docker {
        folder: "/home/sam/.local/share/kashkeshet/docker".into(),
        tailscale: true,
        hostname: Some("kkt-1".into()),
        tailnet: Some("tail1234.ts.net".into()),
    }
}

fn data_env(home: Option<&str>, data: Option<&str>) -> Env {
    Env {
        home: home.map(PathBuf::from),
        xdg_config_home: None,
        xdg_data_home: data.map(PathBuf::from),
        named: Default::default(),
    }
}

#[test]
fn the_database_goes_in_the_platform_data_directory() {
    let e = data_env(Some("/Users/sam"), None);
    assert_eq!(
        default_db(&e, Platform::MacOs),
        Some(PathBuf::from(
            "/Users/sam/Library/Application Support/kashkeshet/kashkeshet.db"
        ))
    );
    let e = data_env(Some("/home/sam"), None);
    assert_eq!(
        default_db(&e, Platform::Xdg),
        Some(PathBuf::from(
            "/home/sam/.local/share/kashkeshet/kashkeshet.db"
        ))
    );
    let e = data_env(Some("/home/sam"), Some("/xdg/data"));
    assert_eq!(
        default_db(&e, Platform::Xdg),
        Some(PathBuf::from("/xdg/data/kashkeshet/kashkeshet.db"))
    );
}

#[test]
fn the_data_directory_needs_a_home_unless_xdg_names_one() {
    // macOS keeps data under the home folder, whatever XDG_DATA_HOME says.
    let e = data_env(Some("/Users/sam"), Some("/xdg/data"));
    assert_eq!(
        data_dir(&e, Platform::MacOs),
        Some(PathBuf::from(
            "/Users/sam/Library/Application Support/kashkeshet"
        ))
    );
    assert_eq!(data_dir(&data_env(None, None), Platform::MacOs), None);
    assert_eq!(data_dir(&data_env(None, None), Platform::Xdg), None);
    assert_eq!(default_db(&data_env(None, None), Platform::Xdg), None);
    // An absolute XDG_DATA_HOME is enough without a home folder; a relative
    // one is not.
    assert_eq!(
        data_dir(&data_env(None, Some("/xdg/data")), Platform::Xdg),
        Some(PathBuf::from("/xdg/data/kashkeshet"))
    );
    assert_eq!(
        data_dir(&data_env(None, Some("relative/data")), Platform::Xdg),
        None
    );
}

#[test]
fn a_docker_install_is_reached_on_its_port_or_at_its_tailscale_name() {
    assert_eq!(docker_url(7420, None, "/mcp"), "http://127.0.0.1:7420/mcp");
    assert_eq!(
        docker_url(
            7420,
            Some(&("kkt-1".into(), "tail1234.ts.net".into())),
            "/mcp"
        ),
        "https://kkt-1.tail1234.ts.net/mcp"
    );
}

#[test]
fn a_relative_xdg_data_home_is_ignored() {
    let e = data_env(Some("/home/sam"), Some("relative/data"));
    assert_eq!(
        data_dir(&e, Platform::Xdg),
        Some(PathBuf::from("/home/sam/.local/share/kashkeshet"))
    );
}

/// An existing Docker install's managed folder stays where setup put it:
/// `docker` in the data directory.
#[test]
fn the_managed_docker_folder_keeps_its_place() {
    assert_eq!(
        docker_folder(&data_env(Some("/Users/sam"), None), Platform::MacOs),
        Ok(PathBuf::from(
            "/Users/sam/Library/Application Support/kashkeshet/docker"
        ))
    );
    assert_eq!(
        docker_folder(&data_env(Some("/home/sam"), None), Platform::Xdg),
        Ok(PathBuf::from("/home/sam/.local/share/kashkeshet/docker"))
    );
    assert_eq!(
        docker_folder(
            &data_env(Some("/home/sam"), Some("/xdg/data")),
            Platform::Xdg
        ),
        Ok(PathBuf::from("/xdg/data/kashkeshet/docker"))
    );
    assert_eq!(
        docker_folder(&data_env(None, None), Platform::Xdg),
        Err("set HOME to keep kkt's Docker files".to_string())
    );
}

#[test]
fn without_a_home_there_is_no_database_unless_xdg_says() {
    assert_eq!(default_db(&data_env(None, None), Platform::MacOs), None);
    assert_eq!(default_db(&data_env(None, None), Platform::Xdg), None);
    assert_eq!(
        default_db(&data_env(None, Some("/xdg/data")), Platform::MacOs),
        None
    );
    assert!(default_db(&data_env(None, Some("/xdg/data")), Platform::Xdg).is_some());
}

#[test]
fn serve_runs_with_no_record_or_when_told_to_ignore_it() {
    assert_eq!(guard(None, None, false), Ok(()));
    for mode in [
        Deployment::Service { tailscale: false },
        docker(),
        Deployment::Client,
    ] {
        assert_eq!(guard(Some(&recorded(mode)), Some(true), true), Ok(()));
    }
}

#[test]
fn with_the_service_set_up_serve_waits_for_it_to_stop() {
    let r = recorded(Deployment::Service { tailscale: false });
    let error = guard(Some(&r), Some(true), false).expect_err("running");
    assert!(error.contains("kkt status"), "{error}");
    assert!(error.contains("--ignore-setup"), "{error}");
    assert_eq!(guard(Some(&r), Some(false), false), Ok(()));
    // Not known: serve tries, and a port in use says so itself.
    assert_eq!(guard(Some(&r), None, false), Ok(()));
}

#[test]
fn docker_and_client_machines_say_where_the_server_is() {
    let error = guard(Some(&recorded(docker())), Some(false), false).expect_err("docker");
    assert!(
        error.contains("Docker, from /home/sam/.local/share/kashkeshet/docker"),
        "{error}"
    );
    let error = guard(Some(&recorded(Deployment::Client)), None, false).expect_err("client");
    assert!(
        error.contains("client of the kkt server at https://kkt.tail1234.ts.net/mcp"),
        "{error}"
    );
    assert!(error.contains("--ignore-setup"), "{error}");
}

/// The serve guard's refusals, word for word.
#[test]
fn the_serve_guard_refusals_are_pinned() {
    let service = recorded(Deployment::Service { tailscale: false });
    assert_eq!(
        guard(Some(&service), Some(true), false),
        Err(
            "kkt already runs on this machine as a service (see `kkt status`); stop it with \
             `kkt service stop`, or pass --ignore-setup to start another server"
                .to_string()
        )
    );
    assert_eq!(
        guard(Some(&recorded(docker())), None, false),
        Err("this machine runs kkt in Docker, from \
             /home/sam/.local/share/kashkeshet/docker (see `kkt status`); pass \
             --ignore-setup to start another server"
            .to_string())
    );
    assert_eq!(
        guard(Some(&recorded(Deployment::Client)), None, false),
        Err("this machine is set up as a client of the kkt server at \
             https://kkt.tail1234.ts.net/mcp; run `kkt setup` to change that, or pass \
             --ignore-setup to start a server anyway"
            .to_string())
    );
}

/// The upgrade check's refusals, word for word.
#[test]
fn the_upgrade_refusals_are_pinned() {
    assert_eq!(
        upgrade(&ServerVersion::Known("0.4.0".into()), "invalid", true),
        Err("this kkt has an invalid version \"invalid\"".to_string())
    );
    assert_eq!(
        upgrade(&ServerVersion::Known("local-test".into()), "0.4.0", false),
        Err(
            "cannot compare server version \"local-test\" with 0.4.0; pass \
             --allow-downgrade to replace it explicitly"
                .to_string()
        )
    );
    assert_eq!(
        upgrade(&ServerVersion::Known("0.5.0".into()), "0.4.0", false),
        Err(
            "refusing to downgrade the server from 0.5.0 to 0.4.0; pass --allow-downgrade \
             to override"
                .to_string()
        )
    );
}
