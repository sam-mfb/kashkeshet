//! Where backups go: beside the database for the backups made before an
//! upgrade, and in kkt's config folder for a service's backups.

use std::path::{Path, PathBuf};

const UPGRADE_BACKUP_FOLDER: &str = "upgrade-backups";
const SERVICE_BACKUP_FOLDER: &str = "backups";

/// The folder for the backups made before an upgrade, beside `database`. A
/// database path in no folder, such as `/`, is refused.
pub fn upgrade_backup_folder(database: &Path) -> Result<PathBuf, String> {
    let folder = database
        .parent()
        .ok_or_else(|| format!("{} is not a database file in a folder", database.display()))?;
    Ok(folder.join(UPGRADE_BACKUP_FOLDER))
}

/// The folder for a service's backups, in kkt's config folder
/// (`config_dir`, `None` without a home).
pub fn service_backup_folder(config_dir: Option<PathBuf>) -> Result<PathBuf, String> {
    Ok(config_dir
        .ok_or("set HOME to find the backup folder")?
        .join(SERVICE_BACKUP_FOLDER))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_upgrade_backups_go_beside_the_database_and_a_path_in_no_folder_is_refused() {
        assert_eq!(
            upgrade_backup_folder(Path::new("/")),
            Err("/ is not a database file in a folder".to_string())
        );
        assert_eq!(
            upgrade_backup_folder(Path::new("/d/kkt.db")),
            Ok(PathBuf::from("/d/upgrade-backups"))
        );
    }

    #[test]
    fn the_service_backups_go_in_the_config_folder() {
        assert_eq!(
            service_backup_folder(Some(PathBuf::from("/h/.config/kkt"))),
            Ok(PathBuf::from("/h/.config/kkt/backups"))
        );
        assert_eq!(
            service_backup_folder(None),
            Err("set HOME to find the backup folder".to_string())
        );
    }
}
