//! Adapts the machine's files to the config file's `Disk`.

use kkt_config_file::files::Disk;
use kkt_files::Files;
use std::io;
use std::path::Path;

/// The machine's files, as the config file's reads and writes take them.
pub(crate) struct ConfigDisk;

impl Disk for ConfigDisk {
    fn read(&self, path: &Path) -> io::Result<String> {
        Files.read(path)
    }
    fn exists(&self, path: &Path) -> bool {
        Files.exists(path)
    }
    fn create_dir_all(&self, path: &Path) -> io::Result<()> {
        Files.create_dir_all(path)
    }
    fn write(&self, path: &Path, text: &str) -> io::Result<()> {
        Files.write(path, text)
    }
}
