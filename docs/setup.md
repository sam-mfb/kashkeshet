# Setup

After installing the `kkt` program, run:

```sh
kkt setup
```

Choose a server in Docker (recommended), a server as a service, or a
client-only machine. At a terminal, Enter chooses Docker and answers no to
sharing over Tailscale. Setup offers the clients it detects,
checks the server connection with `kkt doctor`, then applies the chosen clients' settings
through `kkt connect`.
Restart connected clients so they load their settings.

## Questions and defaults

At a terminal, every question shows its default; pressing Enter accepts it.
Yes/no defaults appear as `[Y/n]` (yes) or `[y/N]` (no).

| Question | Shown default | Enter chooses |
|---|---|---|
| What is this machine for? | `[1]` | Docker |
| Share the server over Tailscale? | `[y/N]` | No; sharing needs an explicit yes |
| Will agents in Docker containers use this server? | `[Y/n]` | Yes; allow `host.docker.internal` |
| Connect each detected client to kkt? | `[Y/n]` | Yes |
| Continue that setup? | `[Y/n]` | Yes; continue the saved unfinished setup |
| Try again? | `[Y/n]` | Yes; retry the failed step |
| The server's MCP address | `[http://127.0.0.1:7420/mcp]` | The local server address |
| The server's access key | `[none]` | No key |

The client address and key still have to pass their connection checks.
Without a terminal, defaults never replace required flags, even with `--yes`.

## When a step fails

At a terminal, setup shows what failed and the fix, and asks `Try again?
[Y/n]`. Fix the problem, or wait (a new Tailscale machine can take a minute
to get its HTTPS certificate), then press Enter to run that step again.
Steps already done are not repeated: a client that failed to connect is
tried again without touching the server. Without a terminal, or with
`--yes`, a failed step ends setup with its error and nothing is asked.

Before installing a server, setup notes the install, with your answers and
the clients you chose, in `~/.config/kkt/setup-unfinished.toml`. For Docker
over Tailscale that includes the `--ts-hostname` and the path given with
`--ts-authkey-file`, but never the auth key itself. Setup notes
it again once the install has finished. If setup then ends before the
setup record is written, the server, or the part of it already started,
stays as it is. Setup says so, and `kkt status` and `kkt doctor` report it
too. Run `kkt setup` again: it finds the note, says what it found, and
continues without asking the questions it already has answers for, and
connects the clients chosen before (`--clients` replaces them). If the
install had finished, it continues from the connection check without
installing again; if not, it runs the install again first. At a terminal
it first asks `Continue that setup? [Y/n]`; answer `n` to start over.
Without a terminal it continues unless the flags given (`--mode`,
`--tailscale`, `--allow-host`) differ from the note, in which case it starts
over.

## A server set up by an older kkt, with no record

Before 0.5.0, setup wrote nothing until its last step, so a setup that
stopped after starting the server (as 0.4.1's did on the Tailscale 502 fixed
in 0.4.2) left the server running with no setup record and no note. With
neither, `kkt status`, `kkt doctor`, `kkt upgrade` and `kkt setup` look for a
server kkt itself installed:

- Docker: the managed `docker` folder at its usual path, with its `.env` and
  Compose files, and `~/.config/kkt/docker-access-key`. The address comes
  from `.env`: the Tailscale machine name and tailnet, or the local port.
  A `.env` with settings setup does not write, such as another
  `COMPOSE_FILE`, is not taken for setup's.
- Service: an installed kkt launchd or systemd service, with the address and
  key from the config file, and over Tailscale if `tailscale serve` proxies
  to it. Its service file must run `kkt serve` with the default config
  file's settings, the ones `kkt setup` installs and `kkt upgrade` installs
  again. A service installed with other settings, such as
  `kkt service install --config other.toml`, is named but not recorded, so
  an upgrade never replaces its settings.

`kkt status` and `kkt doctor` say what was found, and `kkt upgrade` says to
run `kkt setup` first. At a terminal, setup asks `Found a kkt server in
Docker from <folder>, ... not recorded. Record it and continue? [Y/n]`. Yes
writes the record from what was found, then runs the connection check and
connects clients, without installing anything or showing a Tailscale login.
The record is written before the check, so if the check fails (for a server
from an older kkt, `kkt upgrade` may fix it), `kkt upgrade` can still manage
the server. Without a terminal, setup adopts the server only with
`--adopt`. If something needed can't be read, such as the address of a
Docker server whose Tailscale login never finished, setup says what is
missing and does not record it.

`--adopt` never installs anything. If a setup noted here had not finished
installing its server, `kkt setup --adopt` refuses; run `kkt setup` without
it to continue the install.

Nothing records which version an adopted service runs, so its record says
`written_by = "unknown"`, and the next `kkt upgrade` installs the service
again.

## Server on this machine

**Docker** needs the Docker daemon and Compose. Setup keeps its Compose
files and settings in a managed `docker` folder under kkt's data directory:
`~/Library/Application Support/kashkeshet/docker` on macOS, or
`~/.local/share/kashkeshet/docker` on Linux (`$XDG_DATA_HOME` if set).
The files are embedded in the installed binary for its version. Setup
creates the backups folder, handles its permissions, starts the containers
and saves both keys in `~/.config/kkt/docker-access-key` (agents) and
`~/.config/kkt/docker-web-key` (operator), each with mode 0600. No Compose download, version entry or manual key copy is needed.
[Docker](docker.md) describes the separate manual setup.

**Service** uses launchd on macOS or a systemd user service on Linux,
with the usual config file, database and key paths. Setup creates both
`~/.config/kkt/access-key` and `~/.config/kkt/web-key` with mode 0600. It starts now and at
login; see [service](service.md) for logs and start/stop commands. A Linux
user service normally stops at logout unless lingering is enabled.

Setup gives client configurations only the access key. Read the separate
operator key with `kkt key show --web` on the server host to log in to the
web page. `kkt status` names its file and `kkt doctor` checks it locally,
without sending it to MCP. Client-only setups do not keep the web key.

For a server, setup also asks whether to make an admin key, so an agent
holding it can supervise the server at `/mcp-admin` (see
[security](security.md#the-admin-key)). The answer Enter accepts is no.
`--admin-key` and `--no-admin-key` answer it; with no terminal and neither
flag, no admin key is made, so scripted setups run as before. A yes then
asks the admin agent's name, such as `sentinel`, which becomes
`admin-sentinel` (see [security](security.md#the-admin-name));
`--admin-name` answers it, and without a terminal, or when continuing a
setup, the name is `supervisor`. The name is given only to a key setup
makes, not to one the server already has. For a
service, the key is `~/.config/kkt/admin-key`; for Docker, setup makes it
in the container and saves a host copy as `~/.config/kkt/docker-admin-key`,
mode 0600. Either way setup then restarts the server so it serves
`/mcp-admin`, even when the key was already there, so running setup again
finishes a run whose restart failed. Saying no keeps an admin key the
server already has. `kkt key show --admin` exits with code 4 when there is
no admin key.
`kkt status` names the admin key's file when there is one, and `kkt doctor`
checks it. For a service, status also shows its admin name; for Docker,
the name stays in the container and is not saved with the host copy.
Continuing an unfinished setup does not ask again; pass
`--admin-key` to add one then.

Without Tailscale, the web page is at <http://127.0.0.1:7420/> and clients
use `http://127.0.0.1:7420/mcp`.

## Tailscale

For a Docker server, setup starts a Tailscale container and shows a login
link. At a terminal it says "Press Enter to open it in your browser (or open
it yourself)": Enter opens the link with `open` (macOS) or `xdg-open`
(Linux), and setup never opens a browser unless you ask. If opening fails,
setup says so and the link stays on the screen. With no terminal, as in a
script, it only prints the link. Sign in with the link; setup keeps watching
for the login while it waits for Enter, so signing in some other way works
too. Setup waits up to ten minutes; if interrupted or
timed out, run setup again to resume with the saved container state. It
reads the node's actual DNS name, including any suffix Tailscale adds when
a name is already taken, and configures kkt to accept it.

### When the machine needs to log in again

A Tailscale machine's key expires after 180 days unless expiry is turned off
for it in the Tailscale admin console (see [Docker](docker.md#with-tailscale)).
When the key expires, or the machine is logged out, it drops off the tailnet,
and its HTTPS address stops answering, while kkt keeps running on this
machine. `kkt status` then prints `Tailscale login: needed`, and `kkt doctor`
reports a problem on the line "this machine's Tailscale login". A Tailscale
container that does not answer at all is reported as unknown, not as logged
out.

To log in again, run `kkt setup` again with the same answers. Setup notices
the machine is logged out, restarts the Tailscale container to get a fresh
login link, shows it as above (Enter opens it), and waits for you to log in.
It is the same machine: the same name, the same address, and nothing is
lost. There is no separate command for this.

For a service, install Tailscale on the host and log in there first. Setup
uses that login and `tailscale serve`; it does not log you out or replace
the host's Tailscale settings.

HTTPS certificates must be enabled on the tailnet. If setup reports they
are off, open the Tailscale admin console's DNS page, enable HTTPS
certificates, and rerun setup. Other machines use the HTTPS MCP URL setup
prints and a copy of the server's key. Use Tailscale's access controls to
limit which devices and users can reach it; the key is still required.

## Client-only machine

Have the server's MCP URL (ending in `/mcp`) and a copy of its access key.
Setup can ask for the key or use an existing private file. It records the
remote connection without starting a local server.

Setup checks the address as soon as it is given: the server must answer
there and accept the host name. It then checks that the server accepts the
key. These checks change nothing. If one fails, setup says what was wrong
and asks again; when the answer came from `--url` or `--key-file`, it stops
with an error naming the flag.

```sh
kkt setup --mode client --url https://kkt.tail1234.ts.net/mcp \
  --key-file /private/path/server-key --clients codex,claude-code
```

Keep the key file private (mode 0600). The server owner can read the key
with `kkt key show` on a service host, or the manual container command in
[Docker](docker.md#the-access-key).

## Flags and unattended setup

At a terminal, "Connect <client> to kkt?" shows `[Y/n]`: press Enter to
connect, or type `n` to skip it. The Tailscale question shows `[y/N]`:
press Enter to keep the server local, or type `y` to share it.

Each setup question has a flag. Without a terminal, an unanswered question
is an error naming the flag to supply.

```sh
kkt setup --mode service --no-tailscale --clients codex
kkt setup --mode docker --tailscale --clients none
```

| Flag | Answer |
|---|---|
| `--mode service`, `docker` or `client` | What this machine is for |
| `--tailscale` / `--no-tailscale` | Whether to share a server over Tailscale |
| `--url URL` | A client-only machine's server MCP URL |
| `--key-file PATH` | A client-only machine's existing server key file |
| `--clients claude-code,codex,opencode,agy,muse-code` | Which clients to connect; `none` skips them |
| `--yes` | Connect every detected client without asking |
| `--adopt` | Record a server found with no setup record, without asking (see above) |
| `--ts-hostname NAME` | Docker Tailscale node name to request (default `kkt`) |
| `--ts-authkey-file PATH` | Docker Tailscale auth key file instead of an interactive login link |
| `--allow-host NAME` | Another host name the server answers to, such as `host.docker.internal`; repeat for more, or `none` |

`--yes` does not choose the server mode, Tailscale setting, or a remote
URL/key for you. A Docker Tailscale login may still need a person to open
the link. For unattended Docker login, pass `--ts-authkey-file` with a
private file holding an auth key; keep that file out of shared logs and
repositories.

## Agents in containers

kkt answers only to the host names it knows: `localhost`, `127.0.0.1` and
`::1`, plus its tailnet name with Tailscale. Agents running in Docker
containers on the same machine reach it as `host.docker.internal`. Setup
asks whether such agents will use the server and, if so, allows that name.
At a terminal, this question shows `[Y/n]`; Enter answers yes. Without a
terminal, setup still requires `--allow-host`.
To answer with flags, or to allow other names:

```sh
kkt setup --allow-host host.docker.internal
```

For a service, setup adds the names to `allow_hosts` in the config file. For
Docker, it writes them to `KKT_ALLOW_HOSTS` in the managed `.env`, which
`kkt upgrade` keeps. Run setup again with the full list to change them.
Clients in those containers then use `http://host.docker.internal:7420/mcp`
with a copy of the access key.

## Managing the saved setup

```sh
kkt status
kkt backup ~/kkt-backups
kkt upgrade
kkt uninstall
```

`status` reports the selected setup. `backup` acts on that server, exporting
Docker backups into the host folder you name. In Docker mode the folder
argument is optional: `kkt backup` alone leaves the backup in the managed
`docker/backups` folder. Service mode still requires a destination.

Install a new version of the program before `upgrade`: a Docker setup
backs up, updates its managed
files to that version's image and starts it, then saves the host copies of
the access and web keys as setup does. If saving them fails, it says how to
get the keys and the upgrade still finishes. A service backs up and
reinstalls the service using the installed program. See
[backups](backup.md) for restoring an older database when rolling back.
The installed version is the upgrade target. If it is already selected,
`upgrade` reports that and makes no changes. It refuses a lower version or
an image tag it cannot compare unless you pass `--allow-downgrade`.
That flag only overrides the version check: to roll back across database
schema changes, restore a compatible backup while the server is stopped
(see [restoring](backup.md#restoring)). Client-only mode reports that backup
and upgrade belong on the server.

`uninstall` removes the managed service or containers and the setup record,
keeping data, keys and client settings. It leaves the `kkt` executable
installed. See the [README](../README.md#uninstall) for removing client
entries and saved data too.

The setup record is `~/.config/kkt/setup.toml` (`$XDG_CONFIG_HOME/kkt/`
if set). It records the mode, connection URL, key path, and the Docker
folder when applicable. The record is written only after setup succeeds;
failure leaves the previous record in place (see
[When a step fails](#when-a-step-fails)).

`kkt connect` and `kkt doctor` default to this record's URL and key path;
`--url` and `--key-file` override them. See [clients](clients.md).
`kkt serve` refuses on Docker and client-only machines, pointing to the
recorded server. With a service setup it refuses while the service is
running, and allows a terminal server when the service is stopped.
`kkt serve --ignore-setup` explicitly overrides the guard. With no setup
record, `serve` works as before; `backup --db ... --config ...` also keeps
its manual behavior.
