//! The run record's settings text, pinned byte for byte for each kind of
//! server: with and without a config file, each key file, an admin name and
//! a log file, and one with every setting given. A changed text changes what
//! every existing install records, so these texts change only on purpose.

use super::*;
use kkt_names::AdminName;
use std::time::Duration;

/// The settings a test server runs with, in the record's own terms.
struct Case {
    allow_any_bind: bool,
    limits: Limits,
    log_level: LogLevel,
    dependency_log_level: LogLevel,
    origins: &'static [&'static str],
    hosts: &'static [&'static str],
    config_file: Option<&'static str>,
    access_key_file: Option<&'static str>,
    web_key_file: Option<&'static str>,
    admin_key_file: Option<&'static str>,
    /// The name as given in the name file, without the `admin-` prefix.
    admin_name: Option<&'static str>,
    log_file: Option<&'static str>,
    instructions: &'static str,
}

/// A server with nothing given: every setting at its default.
const PLAIN: Case = Case {
    allow_any_bind: false,
    limits: Limits::DEFAULT,
    log_level: LogLevel::Info,
    dependency_log_level: LogLevel::Warn,
    origins: &[],
    hosts: &[],
    config_file: None,
    access_key_file: None,
    web_key_file: None,
    admin_key_file: None,
    admin_name: None,
    log_file: None,
    instructions: "",
};

const BUILD: Build = Build {
    version: "9.9.9",
    commit: "abc123",
};

fn run_of(case: &Case) -> ServerRun {
    let settings = RunSettings {
        allow_any_bind: case.allow_any_bind,
        limits: case.limits.clone(),
        log_level: case.log_level,
        dependency_log_level: case.dependency_log_level,
        allowed_origins: case.origins.iter().map(|o| o.to_string()).collect(),
        allowed_hosts: case.hosts.iter().map(|h| h.to_string()).collect(),
        config_file: case.config_file.map(str::to_string),
        access_key_file: case.access_key_file.map(str::to_string),
        web_key_file: case.web_key_file.map(str::to_string),
        admin_key_file: case.admin_key_file.map(str::to_string),
        // As `serve` gives it: the name's own text.
        admin_name: case.admin_name.map(|name| {
            AdminName::new(name, kkt_config::MAX_ADMIN_NAME_CHARS)
                .expect("admin name")
                .to_string()
        }),
        log_file: case.log_file.map(str::to_string),
        log_policy: LogPolicy::DEFAULT,
        instructions: case.instructions.into(),
    };
    ServerRun::new(BUILD, &settings)
}

/// The lines for [`Limits::DEFAULT`].
const DEFAULT_LIMITS: &str = "name_pack=animals\n\
     max_response_bytes=8192\n\
     default_limit=20\n\
     max_limit=100\n\
     max_body_bytes=8192\n\
     max_channel_name_chars=64\n\
     max_purpose_chars=500\n\
     max_description_chars=300\n\
     max_note_chars=10000\n\
     max_server_announcement_chars=4000\n\
     max_memory_entry_chars=20000\n\
     max_memory_chars=100000\n\
     max_lane_chars=8000\n\
     sends_per_minute=30\n\
     registrations_per_minute=20\n\
     max_agents=1000\n\
     default_wait_ms=30000\n\
     max_wait_ms=50000\n\
     waiting_grace_ms=3000\n\
     member_preview=3\n\
     purpose_preview_chars=120\n\
     web_session_lifetime_ms=43200000\n\
     web_page_messages=50\n\
     web_member_excerpt_chars=80\n\
     upgrade_backups_kept=5";

/// What every case below records before its own lines.
fn plain_text() -> String {
    format!("allow_any_bind=false\n{DEFAULT_LIMITS}\nlog_level=info\ndependency_log_level=warn")
}

#[test]
fn a_plain_server_records_the_defaults() {
    let run = run_of(&PLAIN);
    assert_eq!(run.settings, plain_text());
    assert_eq!(run.version, "9.9.9");
    assert_eq!(run.commit, "abc123");
    assert_eq!(
        run.instructions_sha256,
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
}

#[test]
fn a_server_with_a_config_file_records_it() {
    let run = run_of(&Case {
        config_file: Some("/home/sam/.config/kkt/config.toml"),
        ..PLAIN
    });
    assert_eq!(
        run.settings,
        format!(
            "{}\nconfig_file=/home/sam/.config/kkt/config.toml",
            plain_text()
        )
    );
}

#[test]
fn a_server_with_an_access_key_records_its_file() {
    let run = run_of(&Case {
        access_key_file: Some("/keys/access-key"),
        ..PLAIN
    });
    assert_eq!(
        run.settings,
        format!("{}\naccess_key_file=/keys/access-key", plain_text())
    );
}

#[test]
fn a_server_with_a_web_key_records_its_file() {
    let run = run_of(&Case {
        web_key_file: Some("/keys/web-key"),
        ..PLAIN
    });
    assert_eq!(
        run.settings,
        format!("{}\nweb_key_file=/keys/web-key", plain_text())
    );
}

#[test]
fn a_server_with_an_unnamed_admin_key_records_its_file() {
    let run = run_of(&Case {
        admin_key_file: Some("/keys/admin-key"),
        ..PLAIN
    });
    assert_eq!(
        run.settings,
        format!("{}\nadmin_key_file=/keys/admin-key", plain_text())
    );
}

#[test]
fn a_server_with_a_named_admin_key_records_its_file_and_name() {
    let run = run_of(&Case {
        admin_key_file: Some("/keys/admin-key"),
        admin_name: Some("sentinel"),
        ..PLAIN
    });
    assert_eq!(
        run.settings,
        format!(
            "{}\nadmin_key_file=/keys/admin-key\nadmin_name=admin-sentinel",
            plain_text()
        )
    );
}

#[test]
fn a_server_with_a_log_file_records_it_and_its_policy() {
    let run = run_of(&Case {
        log_file: Some("/logs/kkt.log"),
        ..PLAIN
    });
    assert_eq!(
        run.settings,
        format!(
            "{}\nlog_file=/logs/kkt.log\n\
             log_max_bytes=10485760\n\
             log_rotated_files=5\n\
             log_max_age_ms=2592000000",
            plain_text()
        )
    );
}

#[test]
fn a_server_with_every_setting_given_records_them_all() {
    let run = run_of(&Case {
        allow_any_bind: true,
        limits: Limits {
            max_limit: 99,
            default_wait: Duration::from_secs(12),
            ..Limits::DEFAULT
        },
        log_level: LogLevel::Debug,
        dependency_log_level: LogLevel::Error,
        origins: &["http://b.example", "http://a.example"],
        hosts: &["b.example", "a.example"],
        config_file: Some("/home/sam/.config/kkt/config.toml"),
        access_key_file: Some("/keys/access-key"),
        web_key_file: Some("/keys/web-key"),
        admin_key_file: Some("/keys/admin-key"),
        admin_name: Some("sentinel"),
        log_file: Some("/logs/kkt.log"),
        instructions: "hello",
    });
    assert_eq!(
        run.settings,
        "allow_any_bind=true\n\
         name_pack=animals\n\
         max_response_bytes=8192\n\
         default_limit=20\n\
         max_limit=99\n\
         max_body_bytes=8192\n\
         max_channel_name_chars=64\n\
         max_purpose_chars=500\n\
         max_description_chars=300\n\
         max_note_chars=10000\n\
         max_server_announcement_chars=4000\n\
         max_memory_entry_chars=20000\n\
         max_memory_chars=100000\n\
         max_lane_chars=8000\n\
         sends_per_minute=30\n\
         registrations_per_minute=20\n\
         max_agents=1000\n\
         default_wait_ms=12000\n\
         max_wait_ms=50000\n\
         waiting_grace_ms=3000\n\
         member_preview=3\n\
         purpose_preview_chars=120\n\
         web_session_lifetime_ms=43200000\n\
         web_page_messages=50\n\
         web_member_excerpt_chars=80\n\
         upgrade_backups_kept=5\n\
         log_level=debug\n\
         dependency_log_level=error\n\
         allowed_origin=http://a.example\n\
         allowed_origin=http://b.example\n\
         allowed_host=a.example\n\
         allowed_host=b.example\n\
         config_file=/home/sam/.config/kkt/config.toml\n\
         access_key_file=/keys/access-key\n\
         web_key_file=/keys/web-key\n\
         admin_key_file=/keys/admin-key\n\
         admin_name=admin-sentinel\n\
         log_file=/logs/kkt.log\n\
         log_max_bytes=10485760\n\
         log_rotated_files=5\n\
         log_max_age_ms=2592000000"
    );
    assert_eq!(
        run.instructions_sha256,
        "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
    );
}
