//! Adapts the machine's lane-folder files to the lane-file contract.

use kkt_lane_files::Disk;
use kkt_lane_files_os::OsDisk;
use std::io;
use std::path::{Path, PathBuf};

pub(crate) struct LaneDisk;

impl Disk for LaneDisk {
    fn file_names(&self, folder: &Path) -> io::Result<Vec<String>> {
        OsDisk.file_names(folder)
    }

    fn read(&self, path: &Path) -> io::Result<String> {
        OsDisk.read(path)
    }

    fn create_dir_all(&self, path: &Path) -> io::Result<()> {
        OsDisk.create_dir_all(path)
    }

    fn write(&self, path: &Path, contents: &str) -> io::Result<()> {
        OsDisk.write(path, contents)
    }

    fn absolute(&self, path: &Path) -> io::Result<PathBuf> {
        OsDisk.absolute(path)
    }
}
