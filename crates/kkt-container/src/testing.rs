//! In-memory installations for tests. No commands run and no files are read.
//! Lane output and sharing keys are supplied explicitly by the test.

use super::*;
use std::cell::RefCell;
use std::collections::BTreeMap;

struct Install {
    inputs: Inputs,
    running: bool,
}

/// Installations in memory, with a supplied sharing name and lane output.
/// Backups use the fixed name `backup`; key comparisons return `Same`.
pub struct Memory {
    node: Option<(String, String)>,
    installs: RefCell<BTreeMap<PathBuf, Install>>,
    keys: RefCell<BTreeMap<PathBuf, String>>,
    calls: RefCell<Vec<&'static str>>,
    lane_output: String,
}

impl Memory {
    pub fn new(node: Option<(String, String)>, lane_output: String) -> Self {
        Self {
            node,
            installs: RefCell::new(BTreeMap::new()),
            keys: RefCell::new(BTreeMap::new()),
            calls: RefCell::new(Vec::new()),
            lane_output,
        }
    }
    pub fn set_sharing_key(&self, path: PathBuf, key: String) {
        self.keys.borrow_mut().insert(path, key);
    }
    pub fn calls(&self) -> Vec<&'static str> {
        self.calls.borrow().clone()
    }
    fn note(&self, operation: &'static str) {
        self.calls.borrow_mut().push(operation);
    }
    fn with<T>(
        &self,
        folder: &Folder,
        change: impl FnOnce(&mut Install) -> T,
    ) -> Result<T, String> {
        let mut installs = self.installs.borrow_mut();
        let install = installs
            .get_mut(folder.path())
            .ok_or("container is not installed")?;
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
        _: &'a Path,
    ) -> LocalBoxFuture<'a, Result<Provisioned, String>> {
        self.note("provision");
        Box::pin(async move {
            let node = if inputs.sharing.is_some() {
                Some(
                    self.node
                        .clone()
                        .ok_or("the test must supply a sharing name")?,
                )
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
        key_file: &'a Path,
        _: &'a AdminName,
    ) -> LocalBoxFuture<'a, Result<PathBuf, String>> {
        self.note("add_admin_key");
        Box::pin(async move { self.with(folder, |_| key_file.with_file_name(ADMIN_KEY_FILE_NAME)) })
    }
    fn status(
        &self,
        folder: &Folder,
        _: &dyn ConfigReader,
        default_name_pack: &str,
    ) -> Result<State, String> {
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
    fn existing(&self, folder: &Folder) -> Result<Option<Existing>, String> {
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
    fn compare_web_key(&self, folder: &Folder, _: &Path) -> Result<KeyCopy, String> {
        self.note("compare_web_key");
        self.with(folder, |_| KeyCopy::Same)
    }
    fn up(&self, folder: &Folder) -> Result<(), String> {
        self.note("up");
        self.with(folder, |i| i.running = true)
    }
    fn down(&self, folder: &Folder, _: bool) -> Result<(), String> {
        self.note("down");
        self.with(folder, |i| i.running = false)?;
        Ok(())
    }
    fn backup(&self, folder: &Folder, dest: Option<&Path>) -> Result<PathBuf, String> {
        self.note("backup");
        self.with(folder, |_| {
            dest.map(Path::to_path_buf)
                .unwrap_or_else(|| folder.backups())
                .join("backup")
        })
    }
    fn upgrade<'a>(
        &'a self,
        _: &'a dyn Clock,
        folder: &'a Folder,
        new_version: &'a str,
        _: Option<&'a Path>,
    ) -> LocalBoxFuture<'a, Result<(), String>> {
        self.note("upgrade");
        Box::pin(async move {
            self.with(folder, |i| {
                i.inputs.version = new_version.into();
                i.running = true;
            })
        })
    }
    fn import_lanes(&self, folder: &Folder, _: &LanesImport) -> Result<String, String> {
        self.note("import_lanes");
        self.with(folder, |_| self.lane_output.clone())
    }
    fn export_lanes(&self, folder: &Folder, _: &Path) -> Result<String, String> {
        self.note("export_lanes");
        self.with(folder, |_| self.lane_output.clone())
    }
    fn lane_limit(
        &self,
        folder: &Folder,
        _: &dyn ConfigReader,
        default: usize,
    ) -> Result<usize, String> {
        self.note("lane_limit");
        self.with(folder, |_| default)
    }
    fn has_settings(&self, folder: &Folder) -> bool {
        self.note("has_settings");
        self.installs.borrow().contains_key(folder.path())
    }
    fn settings_file(&self, folder: &Folder) -> bool {
        self.note("settings_file");
        self.installs.borrow().contains_key(folder.path())
    }
    fn version(&self, folder: &Folder) -> Result<String, String> {
        self.note("version");
        self.with(folder, |i| i.inputs.version.clone())
    }
    fn sharing_key(&self, path: &Path) -> Result<String, String> {
        self.note("sharing_key");
        self.keys
            .borrow()
            .get(path)
            .map(|k| k.trim().into())
            .ok_or("sharing key is not supplied".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
            let key = Path::new("/keys/access");
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
            let stopped = memory.status(&one, &Unused, "plants").unwrap();
            assert!(stopped.files_present && !stopped.running);
            assert!(memory.settings_file(&one));
            assert!(memory.status(&two, &Unused, "plants").unwrap().running);
            memory.upgrade(&Unused, &one, "0.2.0", None).await.unwrap();
            assert_eq!(memory.version(&one).unwrap(), "0.2.0");
            assert_eq!(memory.version(&two).unwrap(), "0.1.0");
            assert_eq!(memory.lane_limit(&one, &Unused, 2048).unwrap(), 2048);
            assert_eq!(
                memory.export_lanes(&one, Path::new("/export")).unwrap(),
                "lanes transferred"
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
                memory.status(&shared, &Unused, "plants").unwrap().login,
                Some(Login::LoggedIn)
            );
            assert!(memory.calls().contains(&"upgrade"));
        });
    }
}
