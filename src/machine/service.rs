//! Adapts the machine's files and commands to the service logic's system.

use kkt_service::{Command, Finished, System};
use kkt_service_os::{self as native, OsSystem};
use std::io;
use std::path::Path;

pub(crate) struct ServiceSystem;

impl System for ServiceSystem {
    fn user_id(&self) -> io::Result<u32> {
        OsSystem.user_id()
    }
    fn exists(&self, path: &Path) -> io::Result<bool> {
        OsSystem.exists(path)
    }
    fn read_file(&self, path: &Path) -> io::Result<String> {
        OsSystem.read_file(path)
    }
    fn create_dir_all(&self, path: &Path) -> io::Result<()> {
        OsSystem.create_dir_all(path)
    }
    fn write_file(&self, path: &Path, contents: &str) -> io::Result<()> {
        OsSystem.write_file(path, contents)
    }
    fn remove_file(&self, path: &Path) -> io::Result<()> {
        OsSystem.remove_file(path)
    }
    fn run(&self, command: &Command) -> io::Result<Finished> {
        OsSystem.run(command.program, &command.args).map(finished)
    }
    fn run_attached(&self, command: &Command) -> io::Result<Finished> {
        OsSystem
            .run_attached(command.program, &command.args)
            .map(finished)
    }
}

fn finished(output: native::Finished) -> Finished {
    Finished {
        code: output.code,
        stdout: output.stdout,
        stderr: output.stderr,
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use kkt_service::{Manager, ServiceError, Spec};
    use std::fmt;
    use std::path::PathBuf;

    /// Real fixture files, with no path to a service-manager command.
    struct NoCommands;

    impl System for NoCommands {
        fn user_id(&self) -> io::Result<u32> {
            panic!("permission test must not run id")
        }
        fn exists(&self, path: &Path) -> io::Result<bool> {
            ServiceSystem.exists(path)
        }
        fn read_file(&self, path: &Path) -> io::Result<String> {
            ServiceSystem.read_file(path)
        }
        fn create_dir_all(&self, path: &Path) -> io::Result<()> {
            ServiceSystem.create_dir_all(path)
        }
        fn write_file(&self, path: &Path, contents: &str) -> io::Result<()> {
            ServiceSystem.write_file(path, contents)
        }
        fn remove_file(&self, path: &Path) -> io::Result<()> {
            ServiceSystem.remove_file(path)
        }
        fn run(&self, _command: &Command) -> io::Result<Finished> {
            panic!("permission test must not run a command")
        }
        fn run_attached(&self, _command: &Command) -> io::Result<Finished> {
            panic!("permission test must not run an attached command")
        }
    }

    fn linux_spec() -> Spec {
        Spec {
            program: PathBuf::from("/home/sam/.cargo/bin/kkt"),
            args: [
                "serve",
                "--ignore-setup",
                "--config",
                "/home/sam/.config/kkt/config.toml",
                "--db",
                "/home/sam/.local/share/kashkeshet/kashkeshet.db",
            ]
            .map(String::from)
            .to_vec(),
            dirs: vec![PathBuf::from("/home/sam/.local/share/kashkeshet")],
        }
    }

    fn assert_denied(result: Result<impl fmt::Debug, ServiceError>, path: &str) {
        match result {
            Err(ServiceError::File {
                action: "check",
                path: p,
                source,
            }) if p == Path::new(path) && source.kind() == io::ErrorKind::PermissionDenied => {}
            other => panic!("expected a denied check of {path}, got {other:?}"),
        }
    }

    /// On the real system: a service file in a folder that can't be searched is
    /// an error naming it, for reading and for removal, while a missing one
    /// still reads as not installed. Skipped where the folder can still be
    /// searched, as when running as root.
    #[cfg(unix)]
    #[test]
    fn the_real_system_tells_a_denied_service_file_from_a_missing_one() {
        use std::os::unix::fs::PermissionsExt;
        let dir = crate::scratch::Dir::new("kashkeshet-service-denied");
        let manager = Manager::systemd(dir.path());
        // Where installs keep the unit: kkt-service's systemd user folder and
        // unit name, written out.
        let unit_dir = dir.path().join("systemd/user");
        let unit = unit_dir.join("kashkeshet.service");

        assert_eq!(manager.installed_args(&NoCommands).expect("read"), None);
        assert_eq!(
            manager.uninstall(&NoCommands).expect("nothing to remove"),
            ["The service was not installed."]
        );

        std::fs::create_dir_all(&unit_dir).expect("unit dir");
        std::fs::write(&unit, manager.render(&linux_spec()).expect("unit")).expect("write unit");
        assert_eq!(
            manager.installed_args(&NoCommands).expect("read"),
            Some(linux_spec().args)
        );
        std::fs::set_permissions(&unit_dir, std::fs::Permissions::from_mode(0o000))
            .expect("deny the unit folder");
        let searchable = std::fs::metadata(&unit).is_ok();
        let read = manager.installed_args(&NoCommands);
        // The public uninstall checks the file before running any command.
        // Skip first when permissions do not deny access (e.g. root), so this
        // test never invokes a real service manager.
        if searchable {
            std::fs::set_permissions(&unit_dir, std::fs::Permissions::from_mode(0o700))
                .expect("allow the unit folder");
            eprintln!("skipped: the folder can still be searched, as when running as root");
            return;
        }
        let removed = manager.uninstall(&NoCommands);
        std::fs::set_permissions(&unit_dir, std::fs::Permissions::from_mode(0o700))
            .expect("allow the unit folder");
        let unit = unit.to_str().expect("utf-8");
        assert_denied(read, unit);
        assert_denied(removed, unit);
        assert!(Path::new(unit).exists(), "the unit is still there");
    }
}
