//! The core's tables, as kkt-db migrations: the set `core`.
//!
//! Version 1 is the baseline: the tables kkt 0.10.0 had (its schema 21),
//! declared in kkt-db's terms, less the two handoff-note columns of
//! `agents` (migration 0019 copied them into memory entries, and nothing
//! has read them since), and with an increasing key on `memberships`,
//! which keeps the order members joined in. Version 2 is swarms (schema
//! 22). Later versions only add, as every migration does.

use kkt_db::{Column, Constraint, Set, Step, Table};

/// The name of the core's set of migrations.
pub const CORE: &str = "core";

use super::MAIN_SWARM;

/// The core's tables, at every version.
pub fn schema() -> Set {
    Set {
        name: CORE,
        migrations: vec![baseline(), swarms()],
    }
}

fn run(name: &'static str) -> Column {
    Column::int(name).references("server_runs", "id")
}

fn baseline() -> Vec<Step> {
    vec![
        Step::AddTable(Table::new(
            "server_runs",
            vec![
                Column::key("id"),
                Column::text("started_at"),
                Column::text("version"),
                // A hash, a hash with `-dirty`, or `unknown`.
                Column::text("git_commit"),
                // `name=value` lines.
                Column::text("settings"),
                // Lowercase hex.
                Column::text("instructions_sha256"),
            ],
            vec!["id"],
        )),
        Step::AddTable(
            Table::new(
                "lanes",
                vec![
                    Column::text("name"),
                    Column::text("created_at"),
                    run("run_id"),
                    // When it was retired, by whom (an admin key's name, or
                    // null for the person on the web page) and in which run:
                    // all null while it is current.
                    Column::text("retired_at").nullable(),
                    Column::text("retired_by_admin").nullable(),
                    run("retired_run_id").nullable(),
                ],
                vec!["name"],
            )
            .constraint(Constraint::NoDelete),
        ),
        Step::AddTable(
            Table::new(
                "lane_versions",
                vec![
                    Column::text("lane").references("lanes", "name"),
                    Column::int("version"),
                    Column::text("description"),
                    Column::text("text"),
                    Column::text("saved_at"),
                    run("run_id"),
                    // The admin key that saved it, or null for the web page.
                    Column::text("saved_by_admin").nullable(),
                    Column::text("source_folder").nullable(),
                    Column::text("source_commit").nullable(),
                ],
                vec!["lane", "version"],
            )
            .constraint(Constraint::Positive("version"))
            .constraint(Constraint::Immutable),
        ),
        Step::AddTable(
            Table::new(
                "agents",
                vec![
                    // The handle, such as `amber-falcon`.
                    Column::text("name"),
                    Column::bytes("token_hash").unique(),
                    Column::text("created_at"),
                    Column::text("last_seen").nullable(),
                    run("run_id").nullable(),
                    Column::text("revoked_at").nullable(),
                    Column::bytes("recovery_hash").nullable(),
                    Column::text("description").nullable(),
                    Column::bool("hidden").default(false),
                    Column::text("client_name").nullable(),
                    Column::text("client_version").nullable(),
                    Column::text("client_recorded_at").nullable(),
                    run("client_run_id").nullable(),
                    Column::text("lane").nullable().references("lanes", "name"),
                    Column::int("lane_version").nullable(),
                    Column::text("lane_taken_at").nullable(),
                    run("lane_run_id").nullable(),
                ],
                vec!["name"],
            )
            .index("agents_by_lane", vec!["lane"]),
        ),
        Step::AddTable(Table::new(
            "channels",
            vec![
                Column::text("id"),
                Column::text("name"),
                Column::text("purpose").nullable(),
                Column::bool("is_open"),
                // An avatar, or `operator`, who is none.
                Column::text("created_by")
                    .references("agents", "name")
                    .except(OPERATOR),
                Column::text("created_at"),
                Column::text("note").default(""),
                Column::text("note_edited_by")
                    .nullable()
                    .references("agents", "name"),
                Column::text("note_edited_at").nullable(),
                Column::bool("hidden").default(false),
                Column::text("owner")
                    .nullable()
                    .references("agents", "name"),
                run("run_id").nullable(),
                run("note_run_id").nullable(),
            ],
            vec!["id"],
        )),
        Step::AddTable(
            Table::new(
                "memberships",
                vec![
                    // The order members joined in, for members who joined at
                    // the same time.
                    Column::key("seq"),
                    Column::text("channel_id").references("channels", "id"),
                    Column::text("agent").references("agents", "name"),
                    Column::text("joined_at"),
                ],
                vec!["channel_id", "agent"],
            )
            .index("memberships_by_agent", vec!["agent"]),
        ),
        Step::AddTable(
            Table::new(
                "messages",
                vec![
                    Column::key("id"),
                    Column::text("channel_id").references("channels", "id"),
                    // An avatar, or `operator`, who is none.
                    Column::text("agent")
                        .references("agents", "name")
                        .except(OPERATOR),
                    Column::text("body"),
                    Column::text("created_at"),
                    run("run_id").nullable(),
                ],
                vec!["id"],
            )
            .index("messages_by_channel", vec!["channel_id", "id"])
            .index("messages_by_time", vec!["created_at"])
            .index("messages_by_agent", vec!["agent", "id"]),
        ),
        Step::AddSearch {
            table: "messages",
            column: "body",
        },
        Step::AddTable(
            // The server announcement, under the table name it had before.
            Table::new(
                "operator_instructions",
                vec![
                    Column::key("version"),
                    Column::text("text"),
                    Column::text("saved_at"),
                    run("run_id"),
                    Column::text("saved_by_admin").nullable(),
                ],
                vec!["version"],
            )
            .constraint(Constraint::Immutable),
        ),
        Step::AddTable(Table::new(
            "memory_entries",
            vec![
                Column::text("agent").references("agents", "name"),
                Column::text("name"),
                Column::text("text"),
                Column::text("saved_at"),
                run("run_id").nullable(),
            ],
            vec!["agent", "name"],
        )),
    ]
}

/// Swarms: every avatar and channel belongs to one, and a server in key
/// mode has exactly one, `sw_main`, made here. Its creation time and run
/// are the sentinels `1970-01-01T00:00:00Z` and 0: a migration reads no
/// clock and runs before the run is recorded, and no server run has ID 0.
fn swarms() -> Vec<Step> {
    vec![
        Step::AddTable(
            Table::new(
                "swarms",
                vec![
                    Column::text("id"),
                    Column::text("created_at"),
                    Column::int("run_id"),
                ],
                vec!["id"],
            )
            .constraint(Constraint::Immutable),
        ),
        Step::Fill {
            name: "the main swarm",
            run: |c| {
                c.insert(
                    &kkt_db::Insert::new("swarms")
                        .value("id", MAIN_SWARM)
                        .value("created_at", "1970-01-01T00:00:00Z")
                        .value("run_id", 0),
                )
                .map(drop)
            },
        },
        Step::AddColumn {
            table: "agents",
            column: Column::text("swarm")
                .default(MAIN_SWARM)
                .references("swarms", "id"),
        },
        Step::AddColumn {
            table: "channels",
            column: Column::text("swarm")
                .default(MAIN_SWARM)
                .references("swarms", "id"),
        },
        Step::AddConstraint {
            table: "agents",
            constraint: Constraint::Fixed("swarm"),
        },
        Step::AddConstraint {
            table: "channels",
            constraint: Constraint::Fixed("swarm"),
        },
    ]
}

/// The author recorded for the operator's messages and channels: no
/// avatar.
pub(crate) const OPERATOR: &str = "operator";
