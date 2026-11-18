//! Installations in memory ([`Memory`]), for tests. No commands run and no
//! files are read. What runs in the server print, the files under its
//! config home and sharing keys are supplied explicitly by the test.

use crate::{
    Arg, Carry, ConfigReader, Container, ContainerError, Existing, InServer, Inputs, KeyCopies,
    KeyCopy, Login, LoginLink, Provisioned, State,
};
use futures::future::LocalBoxFuture;
use kkt_clock::Clock;
use kkt_container::Folder;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

struct Install {
    inputs: Inputs,
    running: bool,
}

/// A run in the server that [`Memory`] was asked for, as it was asked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Run {
    pub folder: Folder,
    pub args: Vec<Arg>,
    pub folder_prefix: String,
    pub carry: Carry,
}

/// Installations in memory, with a supplied sharing name and the output of
/// every run in the server. Backups use the fixed name `backup`; key
/// comparisons return `Same`. A file under the config home is empty unless
/// the test supplies it.
pub struct Memory {
    node: Option<(String, String)>,
    installs: RefCell<BTreeMap<PathBuf, Install>>,
    keys: RefCell<BTreeMap<PathBuf, String>>,
    config_files: RefCell<BTreeMap<PathBuf, String>>,
    calls: RefCell<Vec<&'static str>>,
    runs: RefCell<Vec<Run>>,
    read: RefCell<Vec<PathBuf>>,
    run_output: String,
}

impl Memory {
    pub fn new(node: Option<(String, String)>, run_output: String) -> Self {
        Self {
            node,
            installs: RefCell::new(BTreeMap::new()),
            keys: RefCell::new(BTreeMap::new()),
            config_files: RefCell::new(BTreeMap::new()),
            calls: RefCell::new(Vec::new()),
            runs: RefCell::new(Vec::new()),
            read: RefCell::new(Vec::new()),
            run_output,
        }
    }
    pub fn set_sharing_key(&self, path: PathBuf, key: String) {
        self.keys.borrow_mut().insert(path, key);
    }
    /// Gives every installation the file at `relative` under its config
    /// home, with `text`.
    pub fn set_config_home_file(&self, relative: PathBuf, text: String) {
        self.config_files.borrow_mut().insert(relative, text);
    }
    pub fn calls(&self) -> Vec<&'static str> {
        self.calls.borrow().clone()
    }
    /// The runs in the server asked for, in order.
    pub fn runs(&self) -> Vec<Run> {
        self.runs.borrow().clone()
    }
    /// The files under the config home asked for, in order.
    pub fn config_home_reads(&self) -> Vec<PathBuf> {
        self.read.borrow().clone()
    }
    fn note(&self, operation: &'static str) {
        self.calls.borrow_mut().push(operation);
    }
    fn with<T>(
        &self,
        folder: &Folder,
        change: impl FnOnce(&mut Install) -> T,
    ) -> Result<T, ContainerError> {
        let mut installs = self.installs.borrow_mut();
        let install = installs
            .get_mut(folder.path())
            .ok_or_else(|| ContainerError::new("container is not installed"))?;
        Ok(change(install))
    }
}

impl Container for Memory {
    fn provision<'a>(
        &'a self,
        _: &'a dyn Clock,
        _: &'a dyn LoginLink,
        folder: &'a Folder,
        inputs: &'a Inputs,
        _: &'a KeyCopies,
    ) -> LocalBoxFuture<'a, Result<Provisioned, ContainerError>> {
        self.note("provision");
        Box::pin(async move {
            let node =
                if inputs.sharing.is_some() {
                    Some(self.node.clone().ok_or_else(|| {
                        ContainerError::new("the test must supply a sharing name")
                    })?)
                } else {
                    None
                };
            self.installs.borrow_mut().insert(
                folder.0.clone(),
                Install {
                    inputs: inputs.clone(),
                    running: true,
                },
            );
            Ok(Provisioned { node })
        })
    }
    fn add_admin_key<'a>(
        &'a self,
        _: &'a dyn Clock,
        folder: &'a Folder,
        keys: &'a KeyCopies,
        _: &'a str,
    ) -> LocalBoxFuture<'a, Result<PathBuf, ContainerError>> {
        self.note("add_admin_key");
        Box::pin(async move { self.with(folder, |_| keys.admin.clone()) })
    }
    fn status(
        &self,
        folder: &Folder,
        _: &dyn ConfigReader,
        default_name_pack: &str,
        _: &Path,
    ) -> Result<State, ContainerError> {
        self.note("status");
        Ok(match self.installs.borrow().get(folder.path()) {
            Some(i) => State {
                files_present: true,
                running: i.running,
                version: Some(i.inputs.version.clone()),
                name_pack: Ok(default_name_pack.into()),
                sharing: i.inputs.sharing.is_some(),
                login: i.inputs.sharing.as_ref().map(|_| Login::LoggedIn),
            },
            None => State {
                files_present: false,
                running: false,
                version: None,
                name_pack: Err("container is not installed".into()),
                sharing: false,
                login: None,
            },
        })
    }
    fn existing(&self, folder: &Folder) -> Result<Option<Existing>, ContainerError> {
        self.note("existing");
        Ok(self.installs.borrow().get(folder.path()).map(|i| Existing {
            sharing: i.inputs.sharing.is_some(),
            node: if i.inputs.sharing.is_some() {
                self.node.clone()
            } else {
                None
            },
            port: i.inputs.port,
        }))
    }
    fn compare_web_key(&self, folder: &Folder, _: &Path) -> Result<KeyCopy, ContainerError> {
        self.note("compare_web_key");
        self.with(folder, |_| KeyCopy::Same)
    }
    fn up(&self, folder: &Folder) -> Result<(), ContainerError> {
        self.note("up");
        self.with(folder, |i| i.running = true)
    }
    fn down(&self, folder: &Folder, _: bool) -> Result<(), ContainerError> {
        self.note("down");
        self.with(folder, |i| i.running = false)?;
        Ok(())
    }
    fn backup(&self, folder: &Folder, dest: Option<&Path>) -> Result<PathBuf, ContainerError> {
        self.note("backup");
        self.with(folder, |_| {
            dest.map(Path::to_path_buf)
                .unwrap_or_else(|| folder.path().join("backups"))
                .join("backup")
        })
    }
    fn upgrade<'a>(
        &'a self,
        _: &'a dyn Clock,
        folder: &'a Folder,
        new_version: &'a str,
        _: Option<&'a KeyCopies>,
    ) -> LocalBoxFuture<'a, Result<(), ContainerError>> {
        self.note("upgrade");
        Box::pin(async move {
            self.with(folder, |i| {
                i.inputs.version = new_version.into();
                i.running = true;
            })
        })
    }
    fn run_in_server(
        &self,
        folder: &Folder,
        command: &InServer<'_>,
    ) -> Result<String, ContainerError> {
        self.note("run_in_server");
        self.with(folder, |_| ())?;
        self.runs.borrow_mut().push(Run {
            folder: folder.clone(),
            args: command.args.to_vec(),
            folder_prefix: command.folder_prefix.into(),
            carry: command.carry.clone(),
        });
        Ok(self.run_output.clone())
    }
    fn config_home_file(&self, folder: &Folder, relative: &Path) -> Result<String, ContainerError> {
        self.note("config_home_file");
        self.with(folder, |_| ())?;
        self.read.borrow_mut().push(relative.to_path_buf());
        Ok(self
            .config_files
            .borrow()
            .get(relative)
            .cloned()
            .unwrap_or_default())
    }
    fn has_settings(&self, folder: &Folder) -> bool {
        self.note("has_settings");
        self.installs.borrow().contains_key(folder.path())
    }
    fn settings_file(&self, folder: &Folder) -> bool {
        self.note("settings_file");
        self.installs.borrow().contains_key(folder.path())
    }
    fn version(&self, folder: &Folder) -> Result<String, ContainerError> {
        self.note("version");
        self.with(folder, |i| i.inputs.version.clone())
    }
    fn sharing_key(&self, path: &Path) -> Result<String, ContainerError> {
        self.note("sharing_key");
        self.keys
            .borrow()
            .get(path)
            .map(|k| k.trim().into())
            .ok_or_else(|| ContainerError::new("sharing key is not supplied"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ContainerConfig, Sharing};
    use futures::future::BoxFuture;
    use kkt_clock::Time;

    struct Unused;
    impl Clock for Unused {
        fn now(&self) -> Time {
            panic!("the memory container reads no clock")
        }
        fn sleep_until(&self, _: Time) -> BoxFuture<'_, ()> {
            panic!("the memory container never sleeps")
        }
    }
    impl LoginLink for Unused {
        fn show(&self, _: &str) {
            panic!("the memory container never logs in")
        }
        fn open_requested(&self) -> bool {
            panic!("the memory container never opens a browser")
        }
    }
    impl ConfigReader for Unused {
        fn settings(&self, _: &str, _: &Path, _: Option<&Path>) -> Result<ContainerConfig, String> {
            panic!("the memory container parses no config file")
        }
    }

    #[test]
    fn installations_keep_their_settings_when_stopped_and_do_not_affect_each_other() {
        futures::executor::block_on(async {
            let memory = Memory::new(
                Some(("shared".into(), "network".into())),
                "lanes transferred".into(),
            );
            let one = Folder("/one".into());
            let two = Folder("/two".into());
            let key = &KeyCopies {
                access: "/keys/access".into(),
                web: "/keys/web".into(),
                admin: "/keys/admin".into(),
            };
            let inputs = Inputs {
                version: "0.1.0".into(),
                project: "kkt".into(),
                port: 7420,
                allow_hosts: vec![],
                sharing: None,
            };
            assert!(!memory.has_settings(&one));
            assert!(memory.existing(&one).unwrap().is_none());
            assert!(memory.up(&one).is_err());
            memory
                .provision(&Unused, &Unused, &one, &inputs, key)
                .await
                .unwrap();
            memory
                .provision(&Unused, &Unused, &two, &inputs, key)
                .await
                .unwrap();
            memory.down(&one, true).unwrap();
            let stopped = memory
                .status(&one, &Unused, "plants", Path::new("c"))
                .unwrap();
            assert!(stopped.files_present && !stopped.running);
            assert!(memory.settings_file(&one));
            assert!(
                memory
                    .status(&two, &Unused, "plants", Path::new("c"))
                    .unwrap()
                    .running
            );
            memory.upgrade(&Unused, &one, "0.2.0", None).await.unwrap();
            assert_eq!(memory.version(&one).unwrap(), "0.2.0");
            assert_eq!(memory.version(&two).unwrap(), "0.1.0");
            assert_eq!(
                memory
                    .add_admin_key(&Unused, &one, key, "admin-a")
                    .await
                    .unwrap(),
                PathBuf::from("/keys/admin")
            );
            let export = InServer {
                args: &[Arg::Text("export".into()), Arg::Folder],
                folder_prefix: "/tmp/out",
                carry: Carry::Out("/export".into()),
            };
            assert_eq!(
                memory.run_in_server(&one, &export).unwrap(),
                "lanes transferred"
            );
            assert_eq!(
                memory.runs(),
                vec![Run {
                    folder: one.clone(),
                    args: vec![Arg::Text("export".into()), Arg::Folder],
                    folder_prefix: "/tmp/out".into(),
                    carry: Carry::Out("/export".into()),
                }]
            );
            assert!(
                memory
                    .run_in_server(&Folder("/none".into()), &export)
                    .is_err()
            );
            assert_eq!(memory.runs().len(), 1);
            assert_eq!(memory.config_home_file(&one, Path::new("a/b")).unwrap(), "");
            memory.set_config_home_file("a/b".into(), "said".into());
            assert_eq!(
                memory.config_home_file(&two, Path::new("a/b")).unwrap(),
                "said"
            );
            assert_eq!(
                memory.config_home_reads(),
                vec![PathBuf::from("a/b"), PathBuf::from("a/b")]
            );
            memory.set_sharing_key("/keys/sharing".into(), "  secret\n".into());
            assert_eq!(
                memory.sharing_key(Path::new("/keys/sharing")).unwrap(),
                "secret"
            );
            let shared = Folder("/shared".into());
            let inputs = Inputs {
                sharing: Some(Sharing {
                    hostname: "shared".into(),
                    auth_key: None,
                }),
                ..inputs
            };
            let ready = memory
                .provision(&Unused, &Unused, &shared, &inputs, key)
                .await
                .unwrap();
            assert_eq!(ready.node, Some(("shared".into(), "network".into())));
            assert_eq!(memory.existing(&shared).unwrap().unwrap().node, ready.node);
            assert_eq!(
                memory
                    .status(&shared, &Unused, "plants", Path::new("c"))
                    .unwrap()
                    .login,
                Some(Login::LoggedIn)
            );
            assert!(memory.calls().contains(&"upgrade"));
        });
    }
}
