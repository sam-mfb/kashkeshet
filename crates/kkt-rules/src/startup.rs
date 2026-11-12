//! Starting the rules for a server run: the run is recorded in the
//! database, then the rules are built with its ID, which stamps the agents,
//! messages and other rows they make. The programs call [`start`]; recording
//! a run is not an operation of the rules it helps build.

use crate::store::{Db, NewServerRunRecord, StoreResult};
use crate::{Deps, OperatorWording, Rules, Settings};
use kkt_api_agent::SwarmId;
use kkt_clock::{self as clock, Clock};
use kkt_entropy::Entropy;
use kkt_events::Events;
use std::sync::Arc;

/// What produced a server run, as [`start`] records it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewServerRun {
    pub version: String,
    /// The git commit the program was built from.
    pub git_commit: String,
    /// The settings in effect, one `name=value` line each.
    pub settings: String,
    /// SHA-256 of the instructions agents are sent, in lowercase hex.
    pub instructions_sha256: String,
}

/// What the rules depend on at startup: [`Deps`] without the run, which
/// [`start`] records and fills in.
pub struct Startup {
    pub db: Db,
    /// The swarm the rules act in, until [`Rules::in_swarm`] says another.
    pub swarm: SwarmId,
    /// The server's one bus, which every swarm's notices go through.
    pub events: Events,
    pub clock: Arc<dyn Clock>,
    pub entropy: Arc<dyn Entropy>,
    /// What the rules enforce and fill in.
    pub settings: Settings,
    /// How notices name the operator, from where it asked.
    pub wording: Arc<dyn OperatorWording>,
}

/// Records `run` as started now, by `startup.clock`, then builds the rules
/// over `startup` with the run's ID. Fails, recording nothing, if the
/// database refuses the run.
pub async fn start(run: NewServerRun, startup: Startup) -> StoreResult<Rules> {
    let record = NewServerRunRecord {
        started_at: clock::to_text(startup.clock.now()),
        version: run.version,
        git_commit: run.git_commit,
        settings: run.settings,
        instructions_sha256: run.instructions_sha256,
    };
    let run_id = startup
        .db
        .write(move |w| w.insert_server_run(&record))
        .await?;
    Ok(Rules::new(Deps {
        db: startup.db,
        swarm: startup.swarm,
        events: startup.events,
        clock: startup.clock,
        entropy: startup.entropy,
        run_id,
        settings: startup.settings,
        wording: startup.wording,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use kkt_clock::testing::FixedClock;
    use kkt_entropy::testing::SeqEntropy;

    fn run() -> NewServerRun {
        NewServerRun {
            version: "0.11.0".into(),
            git_commit: "4f1c2a9e7b3d5f60a8c1e2d3b4a5968778695a4b-dirty".into(),
            settings: "default_limit=20\nmax_limit=100".into(),
            instructions_sha256: "ab".repeat(32),
        }
    }

    fn startup(db: Db, clock: Arc<FixedClock>) -> Startup {
        Startup {
            db,
            swarm: SwarmId::main(),
            events: Events::new(kkt_events::Settings::DEFAULT),
            clock,
            entropy: Arc::new(SeqEntropy::new(1)),
            settings: Settings::DEFAULT,
            wording: Arc::new(crate::DefaultWording),
        }
    }

    #[tokio::test]
    async fn start_records_the_run_as_given_at_the_clocks_now() {
        let at = kkt_clock::from_text("2026-11-12T08:30:00Z").expect("a time");
        let db = crate::testing::memory().expect("an in-memory store");
        let earlier = start(run(), startup(db.clone(), Arc::new(FixedClock::new(at))))
            .await
            .expect("the first run");
        let rules = start(run(), startup(db.clone(), Arc::new(FixedClock::new(at))))
            .await
            .expect("the second run");
        assert_eq!(rules.run_id(), earlier.run_id() + 1);
        let runs = db.read(|r| r.server_runs()).await.expect("the runs");
        let newest = runs
            .into_iter()
            .find(|r| r.id == rules.run_id())
            .expect("the run start recorded");
        let given = run();
        assert_eq!(
            (
                newest.started_at.as_str(),
                newest.version.as_str(),
                newest.git_commit.as_str(),
                newest.settings.as_str(),
                newest.instructions_sha256.as_str(),
            ),
            (
                "2026-11-12T08:30:00Z",
                given.version.as_str(),
                given.git_commit.as_str(),
                given.settings.as_str(),
                given.instructions_sha256.as_str(),
            )
        );
    }

    #[tokio::test]
    async fn the_rules_start_gives_stamp_their_rows_with_the_run() {
        let at = kkt_clock::from_text("2026-11-12T08:30:00Z").expect("a time");
        let db = crate::testing::memory().expect("an in-memory store");
        let rules = start(run(), startup(db.clone(), Arc::new(FixedClock::new(at))))
            .await
            .expect("a run");
        let handle = rules.register().await.expect("an avatar").handle;
        let agent = db
            .read(move |r| r.agent(&SwarmId::main(), &handle))
            .await
            .expect("a read")
            .expect("the avatar");
        assert_eq!(agent.run_id, Some(rules.run_id()));
    }
}
