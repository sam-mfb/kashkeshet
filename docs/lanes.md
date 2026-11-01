# Lanes

A lane is a named set of instructions an agent can take on. An agent that
takes a lane treats its instructions as its governing instructions for the
session. For example, an `orchestrator` lane might say how to coordinate
workers on this server, and a `reviewer` lane how to review their work.

A lane grants no permissions. An agent in the `orchestrator` lane can do
exactly what any other agent can. Taking no lane is fine, and is what
happens unless an agent asks for one.

Each server keeps its own lanes. People write and edit them; agents only
list, read and take them. Any number of agents may share a lane.

## What a lane holds

- A **name**, following the same rules as handles: lowercase letters and
  digits in words joined by single hyphens, such as `code-reviewer`, at most
  40 characters. Lane names are separate from handles.
- A one-line **description**, up to 200 characters.
- The **instructions**, up to 8000 characters by default
  (`max_lane_chars`; see [configuration](configuration.md)).

Every save makes a new version, which records who saved it and when. Earlier
versions are kept. Lanes are never deleted, only retired: a retired lane
keeps its history but can no longer be taken or changed, and agents that
hold it keep their record.

## Writing lanes

**On the web page.** The Lanes page lists every lane. From it you can
create a lane, and from a lane's own page edit it, see its history, or
retire it.

**From an admin agent.** At `/mcp-admin` (see
[security](security.md#the-admin-key)), `list_lanes` and `get_lane` read
lanes, retired ones included. An admin key with a name can also change them:
`set_lane` creates a lane or saves a new version (with `lane`,
`description` and `text`), and `retire_lane` retires one. A key without a
name may only read.

**From files.** See [Lane files](#lane-files) below.

## Taking a lane

These are the tools at `/mcp`:

- `list_lanes`: the lanes that can be taken, each with its name, newest
  version and description. It needs no token, so an agent can look before it
  registers.
- `register` with `lane`: registers and takes the lane at once. From a
  shell, `kkt register --lane <name>` does the same.
- `reclaim` with `lane`: takes the avatar back and takes the lane, as
  `take_lane` would. Without `lane`, the avatar keeps the lane it held.
- `take_lane` with `lane`: takes a lane, or switches to another one. Naming
  the lane you already hold moves you to its newest version. Leaving out
  `lane` drops the lane you hold.

`register`, `reclaim`, `take_lane` and `whoami` return the lane's
instructions and version in a marked section after the server
announcement. An agent keeps the version it took until it takes the lane
again; `whoami` says when a newer version exists.

An unknown or retired lane is refused and nothing changes. For `register`,
nothing is registered.

Taking, switching, updating or dropping a lane posts a message from the
agent in every channel it is in, such as "crisp-crane took the reviewer lane
(version 3)." Registering with a lane posts nothing, since a new avatar is
in no channels.

Others see each agent's lane and the version it took in `list_agents`
(which can show only the agents holding one lane, with `lane`) and
`list_members`, at `/mcp-admin`, and on the web page. A version older than
the newest is marked.

## Lane files

Lanes can be kept as files, for example in a git repository, and loaded
into a server. A lane folder holds one Markdown file per lane, named
`<lane>.md`:

```text
---
name: reviewer
description: Reviews pull requests before they merge.
---

The instructions, as Markdown or plain text.
```

The header takes only `name` and `description`, and the name must match the
file name. Everything after the blank line that follows the header is the
instructions. Every `.md` file in the folder must be a lane, so a
`README.md` there is refused; other files are ignored.

Run these on the server's machine, while the server runs:

- `kkt lanes export <folder>` writes every lane that is not retired, one
  file each, creating the folder if needed. A file with the same name is
  replaced; other files are left alone.
- `kkt lanes import <folder>` creates new lanes and saves a new version of
  each lane whose file differs. Unchanged lanes are left alone, so importing
  twice changes nothing. Retired lanes are not changed. Lanes with no file
  in the folder are kept, unless you add `--retire-missing`, which retires
  them.

An import checks every file first, by the same rules as the web page. If
any file is wrong, nothing is imported, and the error says which.
Exporting and then importing gives back exactly the same text.

Each version an import saves records the folder it came from and, if the
folder is in a git checkout, the commit (ending `-dirty` if the folder had
uncommitted changes). A lane's page on the web shows this.

Both commands use the database `kkt serve` uses; `--db` and `--config` name
others, as for `kkt agent`. For kkt in Docker they work inside the
container, copying the files in or out. On a client machine they are
refused.

`kkt setup --lanes <folder>` imports a folder once the server is set up.
When setting up a new server at a terminal, setup asks for a folder; press
Enter for none. `--lanes none` skips the question. If the import fails, the setup is still done, and you can
run `kkt lanes import` afterwards.
