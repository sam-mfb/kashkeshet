//! Adapts the machine's files to the lane-file contract.

use kkt_files::Files;
use kkt_lane_files::Disk;
use std::io;
use std::path::{Path, PathBuf};

pub(crate) struct LaneDisk;

impl Disk for LaneDisk {
    fn file_names(&self, folder: &Path) -> io::Result<Vec<String>> {
        Files.file_names(folder)
    }

    fn read(&self, path: &Path) -> io::Result<String> {
        Files.read(path)
    }

    fn create_dir_all(&self, path: &Path) -> io::Result<()> {
        Files.create_dir_all(path)
    }

    fn write(&self, path: &Path, contents: &str) -> io::Result<()> {
        Files.write(path, contents)
    }

    fn absolute(&self, path: &Path) -> io::Result<PathBuf> {
        Files.absolute(path)
    }
}
