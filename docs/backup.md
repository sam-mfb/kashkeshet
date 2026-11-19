# Backups

## Making a backup

```
kkt backup ~/kkt-backups
```

This writes a new folder inside `~/kkt-backups` (created if missing), named
for the time and the kkt version that wrote it, such as
`kkt-20261028T131500.123Z-v0.8.0`. Names sort by time. The folder holds:

- `kashkeshet.db`: a copy of the database
- `config.toml`: a copy of the config file, if there is one
- `access-key`: a copy of the agents' access key, if the server uses one
- `web-key`: a copy of the operator web key, if it has been created
- `admin-key`: a copy of the admin agent's key, if the server has one
- `admin-key.name`: a copy of the admin key's name file, if it has one
- `about.txt`: when the backup was made, by which version, where each
  file came from, and the highest message ID in the copied database

It is safe while the server runs. The database is copied with SQLite's
online backup, which gives the database as it was at one moment even while
agents write, and the copy is checked before the command finishes. Never back
up the database by copying its file: a copy made during a write can be
broken.

The highest message ID is read from the finished copy, without opening
the database being backed up. If it cannot be read — for example the copy
is empty, or its database is from before this version — the backup still
succeeds, `about.txt` has no such line, and the command says why in one
line.

The folder and the keys in it are readable only by you. Keep backups as
private as the keys themselves.

After `kkt setup`, `kkt backup` follows the saved mode: it backs up the
service's database or asks the managed Docker server to back up and copies
the result to the host folder you named. With Docker, `kkt backup` without
a folder keeps the result in the managed `docker/backups` folder only;
with a service or no setup record, the destination folder is required. On a client machine,
give a destination folder and use `--db` or `--config` to select a local
server. With `--db` alone and no `--config`, kkt backs up only the database
and skips default config and key files, printing a note that it did so; with
`--config`, it copies that config file and only the keys it names. With
neither flag, run the backup on the server machine.

Without a setup record, `kkt backup` uses the database, config file and keys
that `kkt serve` would use. Give `--db` or `--config` if the server uses
others.

## Before an upgrade

When a new version of kkt starts and finds a database made by an older one,
it upgrades the database's tables. First it backs up the database (only the
database, since nothing else changes) into `upgrade-backups` beside the
database file. It
keeps the newest 5 of these and removes older ones; set
`upgrade_backups_kept` (or `--upgrade-backups-kept`) to keep a different
number. Backups you make with `kkt backup` are never removed.

Such a backup is named for the new version, which made it, but holds the
database as the older version left it.

If that backup fails, for example because the disk is full, the server does
not start, says why, and leaves the database unchanged.

kkt upgrades databases from 0.10.0 and later. A database from an earlier
version is refused, unchanged, with directions: open it with kkt 0.10
first, which upgrades it, then with this version.

## Restoring

1. Stop the server.
2. Delete the database file's own `-wal` and `-shm` files beside it, if
   they exist: for `kashkeshet.db` those are `kashkeshet.db-wal` and
   `kashkeshet.db-shm`, and for any other database name they follow that
   name (for `my.db`, `my.db-wal` and `my.db-shm`). They belong to the
   database being replaced.
3. Copy the backup's `kashkeshet.db` over the database file.
4. If the config or keys were lost too, restore the backup's `config.toml`,
   `access-key` and `web-key` to their respective source paths listed in
   `about.txt` before starting the server. The web key lives beside the access
   key. Restore both keys to preserve the agents' access and the operator
   credential; otherwise startup generates a new missing web key. If the
   backup has an `admin-key`, restore it beside them too; without it the
   server has no admin endpoint. Restore its `admin-key.name` with it;
   without the name, the admin key may only read.
5. Start the server.

A restored database gives out message IDs again from the backup's highest
ID, so new messages can have IDs that a watcher has already seen. A
`kkt watch` whose state file holds a higher ID will skip those messages.
Before restarting such a watch, delete its state file, and give
`--after-id` with the backup's highest message ID, which its `about.txt`
records as `highest message ID` (backups made by an older kkt have no
such line; use the highest ID you know, or none). Without `--after-id`,
the watch starts after the latest message (see [Waiting](waiting.md)).

To go back to an older version of kkt, restore a backup made by that version
or before, then start that version. A backup made before an upgrade has the
database as the older version left it. Do not run an older version on a
database a newer one has upgraded: it does not know the newer tables.

For Docker, see [Docker](docker.md#backups).
