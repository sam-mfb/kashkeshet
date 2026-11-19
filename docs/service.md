# Running in the background

`kkt setup --mode service` installs the service and connects your clients
(see [setup](setup.md)). The commands below also work when managing a
service by hand.

`kkt service install` runs `kkt serve` as a service of your login: a
launchd user agent on macOS, a systemd user unit on Linux. It starts the
server now and at each login, and starts it again only if it fails. A clean
stop stays stopped.

```
kkt service install            # write the service file and start it
kkt service install --dry-run  # show what would be written and run
kkt service status
kkt service logs --follow
kkt service stop               # until `kkt service start` or the next login
kkt service uninstall          # the database, config and logs stay
```

## What the service runs

Services start in a folder of the manager's choosing, so the service runs
this program with absolute paths: the config file if there is one, and the
database and access key file unless the config file names them. With no key
set anywhere, `install` uses `~/.config/kkt/access-key`, and makes it first
if it is missing. `install` also takes `--config`, `--db`,
`--access-key-file` and `--no-access-key`, as `serve` does.

Everything else comes from the config file each time the service starts.
After editing it, run `kkt service stop` and then `kkt service start`.
Install again after moving the program or changing `--db`.

`install` checks the settings first, so a bad one fails there rather than in
a loop of restarts. It also checks that the server can listen on its
address (`bind`), so an address another program has fails there too. This
check is skipped when the kkt service is already running, since it holds
its own address.

After starting the service, `install` waits 3 seconds and checks that the
server is still running. If it is not, `install` stops the service, so it
does not keep failing and starting again every 5 seconds, and fails,
pointing to `kkt service logs`. The service stays installed; once the cause
is fixed, `kkt service start` starts it.

`kkt status` asks the local service for its running version. If the service
cannot answer within two seconds, status reports that it could not read the
version and still shows the service manager’s status.

## Where things go

- **macOS:** `~/Library/LaunchAgents/com.sam-mfb.kashkeshet.plist`, with the
  log at the path set by `log_file` in the service config. Its default is
  `~/Library/Logs/kashkeshet/kashkeshet.log`. The server keeps that file
  under about 60 MB (see [Logs](configuration.md#logs)). launchd sends
  startup errors and crashes to `~/Library/Logs/kashkeshet/kashkeshet.crash.log`.
- **Linux:** `~/.config/systemd/user/kashkeshet.service` (or
  `$XDG_CONFIG_HOME/systemd/user/`), with the log in the systemd journal.
  The journal is bounded by the system's journald settings: by default at
  most 10% of the disk and 4 GB, with no age limit. A user unit stops when
  you log out unless you run `loginctl enable-linger`.
