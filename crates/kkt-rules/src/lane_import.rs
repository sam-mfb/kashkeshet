//! Importing lanes from files, and reading them for export: what `kkt lanes
//! import` and `kkt lanes export` do to the database, on the server machine,
//! while a server may be running on it. Reading and writing the files is the
//! `lane_files` part's.
//!
//! An import is the human operator's save of each lane, by the same rules as
//! a save on the web page, all in one write: every lane is checked first, and
//! one bad lane changes nothing. Each version it saves records its source.
//! A command is not a server run, so the versions carry the newest server
//! run's ID: the server the database belongs to.

use super::lanes::checked_save;
use super::*;
use kkt_store::{LaneRetiredRecord, LaneSourceRecord, NewLaneVersionRecord};
use std::collections::BTreeSet;

/// Lanes to import, and where they came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LaneImport {
    pub lanes: Vec<LaneSave>,
    pub source: LaneSource,
    /// Retire every current lane not in `lanes`, except those in
    /// `skipped_files`.
    pub retire_missing: bool,
    /// Lanes with a file in the folder that was skipped as not a lane file
    /// (it had no header). `retire_missing` never retires them, so a lane
    /// whose file loses its header is kept, not retired.
    pub skipped_files: Vec<String>,
    /// The server's `max_lane_chars`.
    pub max_lane_chars: usize,
}

/// What an import did to one lane.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LaneOutcome {
    /// A new lane, at version 1.
    Created,
    /// A new version, this one.
    Updated(i64),
    /// Its current version already says this; nothing changed.
    Unchanged(i64),
    /// The lane is retired, so it was not changed.
    RetiredSkipped,
    /// Not in the import, and retired by `retire_missing`.
    Retired,
    /// Not in the import, but not retired by `retire_missing`: its file was
    /// skipped.
    FileSkipped,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImportedLane {
    pub name: String,
    pub outcome: LaneOutcome,
}

/// Checks lanes as [`Operator::import_lanes`] does before it changes
/// anything, without a database: each lane against the lane rules with
/// `max_lane_chars`, and no lane named twice. `kkt setup` uses it to find a
/// bad lane before it installs anything. An error names the lane.
pub fn check_lane_import(lanes: &[LaneSave], max_lane_chars: usize) -> Result<()> {
    checked_lanes(lanes.to_vec(), max_lane_chars).map(drop)
}

/// Whether an import from `folder` that found `lanes` lane files and
/// `skipped_files` skipped `.md` files may retire the lanes it leaves out.
/// A folder with no lane files but skipped `.md` files is most likely the
/// wrong folder, such as a repository's root, so retiring every lane from it
/// is refused, naming the folder. A folder with no `.md` files at all is not.
/// Whoever reads the folder calls it before anything is changed;
/// [`Operator::import_lanes`] calls it too.
pub fn check_retire_missing(folder: &str, lanes: usize, skipped_files: usize) -> Result<()> {
    if lanes == 0 && skipped_files > 0 {
        return Err(RuleError::invalid(format!(
            "{folder} has no lane files, only skipped .md files, so --retire-missing would \
             retire every lane; name the folder that holds the lanes"
        )));
    }
    Ok(())
}

/// The lanes, each checked by the lane rules, or an error naming the first
/// bad one or one given twice.
fn checked_lanes(lanes: Vec<LaneSave>, max_lane_chars: usize) -> Result<Vec<LaneSave>> {
    let mut names = BTreeSet::new();
    let mut checked = Vec::with_capacity(lanes.len());
    for save in lanes {
        let name = save.name.clone();
        if !names.insert(name.clone()) {
            return Err(RuleError::invalid(format!(
                "the lane {name} is given twice"
            )));
        }
        checked.push(
            checked_save(save, max_lane_chars)
                .map_err(|e| RuleError::invalid(format!("lane {name}: {}", e.message)))?,
        );
    }
    Ok(checked)
}

impl Operator {
    /// Every lane that is not retired, in name order: what `kkt lanes export`
    /// writes.
    pub async fn current_lanes(&self) -> Result<Vec<Lane>> {
        self.db
            .read(|s| {
                Ok(s.lanes()?
                    .into_iter()
                    .filter(|l| l.retired.is_none())
                    .map(Lane::from)
                    .collect())
            })
            .await
    }

    /// Imports `import` as the human operator, in one write: new lanes are
    /// created, changed ones get a new version, and unchanged ones are left
    /// alone, so importing the same lanes again changes nothing. A retired
    /// lane is left as it is. With `retire_missing`, current lanes not
    /// imported are retired, except those named in `skipped_files`, which
    /// are reported as kept. Returns what happened to each lane, imported
    /// ones in the order given, then the rest in name order.
    ///
    /// Every lane is checked first; a lane that breaks the lane rules, or
    /// one named twice, is refused, naming it, and nothing changes. With
    /// `retire_missing`, no lanes and some `skipped_files` are refused by
    /// [`check_retire_missing`], naming the source folder. A database no
    /// server has run on yet is refused when there is something to change,
    /// as there is no server run to stamp it with.
    pub async fn import_lanes(&self, import: LaneImport) -> Result<Vec<ImportedLane>> {
        if import.retire_missing {
            check_retire_missing(
                &import.source.folder,
                import.lanes.len(),
                import.skipped_files.len(),
            )?;
        }
        let lanes = checked_lanes(import.lanes, import.max_lane_chars)?;
        let names: BTreeSet<String> = lanes.iter().map(|l| l.name.clone()).collect();
        let source = LaneSourceRecord {
            folder: import.source.folder,
            commit: import.source.commit,
        };
        let retire_missing = import.retire_missing;
        let skipped_files: BTreeSet<String> = import.skipped_files.into_iter().collect();
        let at = clock::to_text(self.clock.now());
        self.db
            .write(move |s| {
                // Found only once something is to change.
                let mut run_id = None;
                let mut run = |s: &dyn Writes| -> Result<i64> {
                    if let Some(id) = run_id {
                        return Ok(id);
                    }
                    let id = s.server_runs()?.first().map(|r| r.id).ok_or_else(|| {
                        RuleError::invalid(
                            "no server has run on this database yet: start `kkt serve` on it \
                             once, then import",
                        )
                    })?;
                    run_id = Some(id);
                    Ok(id)
                };
                let mut done = Vec::new();
                for save in lanes {
                    let outcome = match s.lane(&save.name)? {
                        Some(lane) if lane.retired.is_some() => LaneOutcome::RetiredSkipped,
                        Some(lane)
                            if lane.current.description == save.description
                                && lane.current.text == save.text =>
                        {
                            LaneOutcome::Unchanged(lane.current.version)
                        }
                        found => {
                            let run_id = run(s)?;
                            if found.is_none() {
                                s.insert_lane(&save.name, &at, run_id)?;
                            }
                            let saved = s.append_lane_version(&NewLaneVersionRecord {
                                lane: save.name.clone(),
                                description: save.description,
                                text: save.text,
                                saved_at: at.clone(),
                                run_id,
                                saved_by_admin: None,
                                source: Some(source.clone()),
                            })?;
                            match found {
                                None => LaneOutcome::Created,
                                Some(_) => LaneOutcome::Updated(saved.version),
                            }
                        }
                    };
                    done.push(ImportedLane {
                        name: save.name,
                        outcome,
                    });
                }
                if retire_missing {
                    for lane in s.lanes()? {
                        if lane.retired.is_some() || names.contains(&lane.name) {
                            continue;
                        }
                        if skipped_files.contains(&lane.name) {
                            done.push(ImportedLane {
                                name: lane.name,
                                outcome: LaneOutcome::FileSkipped,
                            });
                            continue;
                        }
                        let run_id = run(s)?;
                        s.retire_lane(
                            &lane.name,
                            &LaneRetiredRecord {
                                at: at.clone(),
                                run_id,
                                by_admin: None,
                            },
                        )?;
                        done.push(ImportedLane {
                            name: lane.name,
                            outcome: LaneOutcome::Retired,
                        });
                    }
                }
                Ok(done)
            })
            .await
    }
}
