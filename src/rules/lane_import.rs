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
use crate::store::{LaneRetiredRecord, LaneSourceRecord, NewLaneVersionRecord};
use std::collections::BTreeSet;

/// Lanes to import, and where they came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LaneImport {
    pub lanes: Vec<LaneSave>,
    pub source: LaneSource,
    /// Retire every current lane not in `lanes`.
    pub retire_missing: bool,
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
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImportedLane {
    pub name: String,
    pub outcome: LaneOutcome,
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
    /// imported are retired. Returns what happened to each lane, imported
    /// ones in the order given, then retired ones in name order.
    ///
    /// Every lane is checked first; a lane that breaks the lane rules, or
    /// one named twice, is refused, naming it, and nothing changes. A
    /// database no server has run on yet is refused when there is something
    /// to change, as there is no server run to stamp it with.
    pub async fn import_lanes(&self, import: LaneImport) -> Result<Vec<ImportedLane>> {
        let mut names = BTreeSet::new();
        let mut lanes = Vec::with_capacity(import.lanes.len());
        for save in import.lanes {
            let name = save.name.clone();
            if !names.insert(name.clone()) {
                return Err(RuleError::invalid(format!(
                    "the lane {name} is given twice"
                )));
            }
            let checked = checked_save(save, import.max_lane_chars)
                .map_err(|e| RuleError::invalid(format!("lane {name}: {}", e.message)))?;
            lanes.push(checked);
        }
        let source = LaneSourceRecord {
            folder: import.source.folder,
            commit: import.source.commit,
        };
        let retire_missing = import.retire_missing;
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
