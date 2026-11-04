//! Backups of an install: writing one, and the one `kkt serve` makes
//! before it upgrades the database.

use kkt_backup as backup;
use kkt_clock::Clock;
use kkt_config as config;
use kkt_provenance::Build;
use kkt_store as store;
use std::path::{Path, PathBuf};

/// Writes a backup of `contents` into `folder`, copying the database with
/// SQLite's online backup.
pub fn write_backup(
    folder: &Path,
    clock: &dyn Clock,
    contents: &backup::Contents,
) -> Result<PathBuf, String> {
    let at = clock.now();
    backup::write(
        folder,
        at,
        &kkt_clock::to_text(at),
        Build::CURRENT.version,
        contents,
        |from, to| store::copy_database(from, to).map_err(|e| e.to_string()),
    )
}

/// Before the server upgrades the tables in `database`, backs it up beside
/// it and prunes those backups to the newest `keep`. Only the database: an
/// upgrade changes nothing else, and a copy of the key in the data folder
/// would be one more place it could leak from. Any failure to back up stops
/// the server before the database is changed.
pub fn backup_before_upgrade(
    database: &Path,
    keep: usize,
    clock: &dyn Clock,
) -> Result<(), String> {
    let Some(upgrade) = store::pending_upgrade(database)
        .map_err(|e| format!("could not read {}: {e}", database.display()))?
    else {
        return Ok(());
    };
    let folder = database
        .parent()
        .unwrap_or(Path::new("."))
        .join(config::UPGRADE_BACKUP_FOLDER);
    let contents = backup::Contents {
        database,
        config: None,
        access_key: None,
        web_key: None,
        admin_key: None,
        admin_name: None,
    };
    let written = write_backup(&folder, clock, &contents).map_err(|e| {
        format!(
            "the database needs upgrading (schema {} to {}), and kkt backs it up first, \
             but the backup failed, so the server did not start and the database is \
             unchanged: {e}",
            upgrade.from, upgrade.to
        )
    })?;
    tracing::info!(
        "backed up the database to {} before upgrading it from schema {} to {}",
        written.display(),
        upgrade.from,
        upgrade.to
    );
    // The backup is made, so the upgrade is safe; an old backup left behind
    // only takes space.
    match backup::prune(&folder, keep) {
        Ok(removed) => {
            for path in removed {
                tracing::info!("removed the old backup {}", path.display());
            }
        }
        Err(e) => tracing::warn!("could not remove old backups: {e}"),
    }
    Ok(())
}
