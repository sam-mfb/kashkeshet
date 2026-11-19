use super::*;
use kkt_bare_service::ServiceError;
use std::path::PathBuf;

fn env(home: Option<&str>, xdg_config_home: Option<&str>) -> Env {
    Env {
        home: home.map(PathBuf::from),
        named: Default::default(),
        xdg_config_home: xdg_config_home.map(PathBuf::from),
        xdg_data_home: None,
    }
}

/// launchd needs only the home folder, systemd only the config base, which
/// the home folder gives when `$XDG_CONFIG_HOME` is not set.
#[test]
fn this_platforms_service_is_ready_with_only_its_own_folder() {
    let ready = |home, xdg| for_this_os(&env(home, xdg)).ready();
    if cfg!(target_os = "macos") {
        assert!(ready(Some("/Users/sam"), None).is_ok());
        assert!(matches!(
            ready(None, Some("/config")),
            Err(ServiceError::NoHome)
        ));
    } else if cfg!(target_os = "linux") {
        assert!(ready(None, Some("/home/sam/.config")).is_ok());
        assert!(ready(Some("/home/sam"), None).is_ok());
    } else {
        assert!(matches!(
            ready(Some("/home/sam"), Some("/config")),
            Err(ServiceError::Unsupported(_))
        ));
    }
}

#[test]
fn this_platforms_service_keeps_its_missing_folder_and_unsupported_messages() {
    let error = for_this_os(&env(None, None))
        .ready()
        .expect_err("no platform folder");
    if cfg!(any(target_os = "macos", target_os = "linux")) {
        assert!(matches!(error, ServiceError::NoHome));
        assert_eq!(
            error.to_string(),
            "no home directory to install the service in: set HOME"
        );
    } else {
        assert!(matches!(error, ServiceError::Unsupported(_)));
        assert_eq!(error.to_string(), UNSUPPORTED);
    }
}

/// On every platform, so the words are pinned where CI runs.
#[test]
fn an_unsupported_system_is_told_which_service_managers_kkt_works_with() {
    let error = kkt_bare_service::Unsupported::new(UNSUPPORTED.to_string())
        .ready()
        .expect_err("unsupported");
    assert_eq!(
        error.to_string(),
        "kkt service works with launchd on macOS and systemd on Linux, not on this system"
    );
}
