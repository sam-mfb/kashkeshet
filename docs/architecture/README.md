# The crate map

`index.html` is an interactive map of the workspace's crates: one box per
package, in layers, with an arrow for each dependency between crates. Open it in a browser straight from the checkout; it loads nothing from
the network. Click a crate to show it in the panel: its concern, what it
must not do, the public items it provides, and which of them flow along each
arrow. Clicking highlights nothing. Tick the checkbox on a box's top corner
to light up that crate's arrows, neighbours and delivery lines; several
ticked crates light up together, and the panel lists them with a button to
clear them. With nothing ticked, every arrow is faded. Ticks and the panel's
crate are kept by name, so changing a filter never moves them to another
crate. Hovering only outlines a box, and shows a highlighted arrow's or a
delivery line's tooltip; faded arrows take no pointer. Every box keeps its
place in a map whatever the filters hide: a hidden crate leaves an empty
slot. The eye on a
program box's bottom corner hides that program like a hidden layer; Show
all and Reset view bring it back, and the dev-only box hides and shows the
dev programs with it. Search finds crates
and items. Above the map, the Crates menu shows or hides crates by kind
(a hidden kind's crates keep their places) and dev-only crates; the Arrows
menu shows or hides arrows by role and by what uses them (code, tests or
build scripts). A crate's panel lists every public item; one that exists
only with the `real` or `testing` feature is marked "real only" or "tests
only". Clicking a layer's name on the map hides that
layer: its crates stay in place, faded and still clickable, and no arrow
or delivery line goes to or from them. Each menu has All and None buttons
at its top; "Show all" and "Hide all" above the map turn every layer on or
off; and "Reset view" in the header puts every filter, menu and layer back
as the page starts, clears the search, the ticks and the panel's crate,
and keeps the map shown.

It shows surfaces and flows only: public items, their documentation's
first sentence, public fields, methods and variants, and what a feature adds
to them. It never shows private items, private module paths or function
bodies. A crate's "must not" appears only where CLAUDE.md states one.

## Reading the page after the refactor

The page has two maps, switched by the toggle in its header.

- **Current** is the code. `generate.py` builds it from the code into
  `map-data.js`; regenerate it after a change to crates, dependencies or
  public items.
- **Proposed** is the refactor's plan, as it was last amended while the
  refactor ran, kept as a record. `propose.py` rebuilds it from
  `proposal-base-data.js` (the map the proposal was made from, at 27f1d11),
  `proposed.json` and `config-split.json`. It changes only when Sam
  decides a further change, such as phase 9, in which kkt-settings goes.

`decisions.md` says where the two still differ, and why, in "After the
refactor".

| File | What it is |
|---|---|
| `index.html`, `map.css`, `map.js` | The page |
| `concerns.json` | Written by hand: the layers (where in the system a crate sits), the kinds (what sort of crate it is), and each crate's layer, kind, concern and what it must not do, from CLAUDE.md and the crates' docs |
| `kinds.py` | The kinds, and the checks a map's kinds must pass; `generate.py` and `propose.py` both use it |
| `page-test.js` | A headless check of the page: ticks and the panel's crate kept by name through every filter and a switch of map, Reset view clearing them, no box moving when a filter changes, and only full-strength arrows answering the pointer. Needs Playwright with Chromium; from the repository's root, once: `npm install --prefix target/tmp/page-test playwright` and `target/tmp/page-test/node_modules/.bin/playwright install chromium`; then `NODE_PATH=target/tmp/page-test/node_modules node docs/architecture/page-test.js`. CI does not run it |
| `assembly.json` | Written by hand: how each program (kkt, kkt-keymode, the fake product) assembles the server, step by step; every item a step names is checked against the code |
| `generate.py` | Writes `map-data.js` from the code; its docstring says where each part comes from; with `--out FILE` it writes the map to that file instead and leaves `map-data.js` as it is |
| `progress.py` | The refactor's scorecard: builds the map from the code into `target/tmp` and compares it with the proposed map, by crates, items, arrows and rules. `--list` names every missing or extra entry. While the refactor ran, `map-data.js` stayed at the map the proposal was made from, and the proposal was updated as deliberate differences merged; see `refactor-plan.md` |
| `check-programs.py` | CI's check that the programs do no machine work (step 8.2): builds the map from the code into `target/tmp` and fails on machine work in a composition crate's own code, file by file and library by library, unless the plan's "Temporary breaks" table lists it in a row such as "`kkt-keymode` does machine work in `src/main.rs`". A listed file that no longer does any also fails, so the step that moves the work out removes the row. `test_check_programs.py` tests it |
| `map-data.js` | Generated: the crates, arrows and public items, with `concerns.json` |

To refresh it after the code changes, run

```
python3 docs/architecture/generate.py
```

from anywhere in the checkout. It needs the pinned toolchain, and builds
rustdoc's JSON (`RUSTC_BOOTSTRAP=1`, since that output is not yet stable)
into `target/architecture-map`. It stops if a package has no entry in
`concerns.json`, an entry names a package that is gone, or an entry's layer
or kind is not one it knows: fix the entry and run it again.

## Kinds

Each box is outlined in its kind's colour, written by hand per crate:

- **contract**: traits and plain values, with pure behaviour on its own value types (Time arithmetic, query checks, search matching), and no effect or wiring;
- **contract and effect**: a contract in the open and its one implementation behind `real`, which only programs turn on (phase 13); a crate that depends on it without `real` depends on the contract alone;
- **logic**: behaviour, with every effect injected (protocol translation counts, so kkt-mcp and kkt-web are logic);
- **effect**: touches the operating system, disk, network, real time or real randomness directly; a named tool wrapper also owns its tool knowledge, tested through a private runner, with process execution behind `real`;
- **composition**: wires others and decides nothing (kkt-server and the programs);
- **test support**: for tests only;
- **mixed**: a contract or logic together with a real effect, in the current map only. Named tool wrappers are the explicit exception and are marked effect. In the proposed map a mixed crate is a violation.

`kinds.py` checks what it can: a contract that does more than compute on its
own values (behaviour through an injected dependency, wiring, or an effect), a logic crate that turns on `real`,
an effect crate with no `real` feature and no I/O dependency, a composition
crate other than a program with a `real` feature of its own, and a composition
crate, programs included, that does machine work in its own code. For a crate
marked contract and effect, it checks that its contract, outside `real`, only
computes on its own values (a slot behind `real` is the implementation's), that
it has a `real` feature, that its I/O dependencies are optional ones only `real`
turns on, that a type outside `testing` implements its own contract, and that
only programs turn its `real` on. A crate that fails these is mixed. In the proposed
map the last two, like a mixed crate, are violations. It reports
a mismatch on the page and does not change the kind.

Machine work is what `generate.py` finds in a crate's own code, outside its
tests: a third-party crate that reaches the machine (libc, tracing-subscriber,
rusqlite, reqwest, rand); a path under `std` for files, the environment,
processes, sockets, the terminal or the clock (`std::fs`, `std::env::var`,
`std::process::Command`, `std::io::stdin`, `std::path::absolute` and the
like); a print macro; or a `Path` method that asks the file system about a
path (`exists`, `is_file`, `canonicalize` and the like, called with no
arguments, so an effect crate's `host.exists(path)` is not counted).
Reading the command line (`std::env::args`) is not machine work: parsing its
arguments is the program's. Value types such as `SocketAddr` and `ExitCode`
are not either. The scan reads names, so it misses calls such as
`path.exists()` on a `Path`. The map records the work by file. In the
proposal, `program_work` in `proposed.json` says, for every program file that
does machine work today, which effect crates take the work and which logic
crates take its decisions. `propose.py` stops if a file is left out, if a
crate named is of the wrong kind, or if the program has no arrow to the effect
crate.

A merged crate, of kind "contract and effect", is drawn as a split box:
its contract on the left, and on the right a tinted strip labelled `real`,
its one implementation. A small hollow green triangle on the divider, on
the second line, is the implementation implementing its own contract. A
name too long for the contract part is squeezed to end before the divider. An arrow meets the part
it depends on: one that turns `real` on (the program's, or the fake
product's tests') meets the implementation; any other meets the contract.
An arrow with several roles is placed stroke by stroke: an "implements"
stroke always meets the contract, which is what it implements.
The merged crate's own optional dependencies, which only `real` turns on,
leave from the implementation. The page reads all of this from the data
it already has: the crate's kind, each arrow's features and whether it is
optional.

## Roles, contracts and slots

Each arrow its code uses (Cargo's dependency) has one or more roles. It is drawn as one stroke per role, side by side in each role's colour, with one head in the shape of its first role in this order; the header's Arrows menu hides roles, and an arrow with none of its roles shown is not drawn.

Which way an arrow points (Sam, msg 3155): a "uses" or "wires" arrow runs from the crate that is used or built to the crate that uses or builds it, the way a part goes into what includes it; an "implements" arrow runs from the implementer to the contract. An arrow that both implements and uses keeps the implementing direction, since using the contract's records is part of implementing it. Only the drawing goes this way: the data, the panel's lists ("Includes" for what a crate depends on, "Included in" for what depends on it) and the scripts keep Cargo's sense, in which a crate depends on another.

| Role | Drawn as | Means, for an arrow from A to B | Derived from |
|---|---|---|---|
| implements a contract of | green, hollow triangle | A implements a trait B defines (kkt-service-launchd's `Launchd` implements kkt-service's `Service`) | the trait impls in rustdoc's JSON, including impls by private types, which the page names only as "a type of its own" |
| used by | grey, filled arrow | B calls or reads A's items | every named item that is not one of the others |
| built by | amber, open arrow | B builds A's implementation of a contract, or a type that holds one, to hand it on (a program building `SqlitePlace` for `Db::new`) | items that implement a contract, build one (a public method returns another crate's holder, as `SqlitePlace::open` gives a `Database`), or hold one, which the crate builds: it calls one of the type's constructors (a public function giving back the type, such as `KeyMode::new`), or names it if it has none (`SystemClock`). Calling another function of such a type (`KeyMode::admin_operator`) is using it. Plain settings structs and the traits the wired types implement go with the wiring |

Where wired things go comes from the assembly data (assembly.json, and the proposal's flows): what a program, or kkt-server where it does the wiring itself (its "injectors" entry), builds or passes on, from which crate, and which slot of which crate it injects it into, with the program's adapter where there is one. A wires arrow's tooltip and its panel entry say so in words. Ticking a crate that injects, supplies or receives things draws thin dotted amber lines from the injecting program, or kkt-server, to each receiving crate, one per pair, labelled with the crates that supply what is injected; hovering a line or its label lists each injection, with the slot it fills and the contract it meets. They are not dependencies, and they follow the filters.

A crate's panel lists the contracts it defines (its public traits, with who
implements each), the contracts of other crates it implements, and the
slots it takes: public parameters and fields typed as a workspace trait
(`dyn`, `impl` or a generic bound), or as a type that holds one (`Db::new`'s
`database: Database`, which holds a `Backend`). A program's panel opens how
it is assembled, from `assembly.json`, written by hand and checked against
the code: the flow, step by step, and what the program passes to which
slot, its own implementations included. Below that, folded away, the page
also lists the pairs it infers from the
generated data by matching names: the program builds the implementation
(or has its own), names the slot's owner, and the slot takes that contract
or holds what the implementation builds. A pair says what fits, not that
the code passes that value there. Every section says whether it is
generated, inferred or hand-written.

What flows along an arrow is read from names: the paths a crate's code
names in the other crate, matched to that crate's public items. An item
used only through a method call, a trait or a derive does not show, and an
arrow whose crate is named nowhere says so.

Run the script after a change to crates, dependencies or public items, and
commit `map-data.js` with it. To see whether the committed map is current
without changing it, run

```
python3 docs/architecture/generate.py --check
```

It writes nothing, and fails if `map-data.js` differs from what it would
write in anything but the commit it names.

CI runs `--check` only on pull requests into `main`, so a release needs a
fresh map: regenerate it on `next` and merge that, and the release's pull
request picks it up. Pull requests into `next` and pushes don't run it, so
a change to public items there is never held up by the map.

## The proposed map

The page's "Proposed" toggle shows the workspace reorganized by one
dependency-injection rule and Sam's decisions on it. It was made to plan the
refactor, and is now its record. It is a chart, not a change to the code. Every change on it carries a status:
decided by Sam, or recommended pending Sam. It includes item 7, kkt-config
broken up by domain, which Sam accepted; its per-item table opens from the
proposed overview.

| File | What it is |
|---|---|
| `decisions.md` | Written by hand: the decisions behind the proposed map, with their reasons, the accepted assumptions and the rejected options. The page states only what is or is proposed and does not link here |
| `proposed.json` | Written by hand: the rule and Sam's strict reading of it (no implementation implements another crate's contract, though a crate may implement its own traits; the program, or a composition, adapts), and how they apply here: new crates, renamed crates, moves, items made private or added, methods that move, features that move, new slots, traits consumers declare, implementations in other crates that stop implementing a trait, code that moves with a moved type, adapters (with their size), adapters shared from composition crates, implementations that stay because their crate is itself the adapter, functions that become a trait's provided methods, every adapter measured method by method (forwarding or translating) before and after the reshape of msg 2009, slots that take one value meeting several contracts, the types a slot holds, arrow edits, the programs' flows as they change, a decision for every trait, the conflicts and how they are resolved, and the assumptions. Every entry has a status, kept for the checks and decisions.md; the page shows descriptions only, with references to decisions and discussions taken out |
| `config-split.json` | Written by hand (drafted from each item's users, then decided item by item): item 7, accepted by Sam, where each of kkt-config's public items goes, and whether it stays a constant, becomes injected, or is test-visible |
| `proposal-base-data.js` | A copy of `map-data.js` as it was at 27f1d11, the map the proposal was made from. It does not change |
| `propose.py` | Applies both to `proposal-base-data.js` and writes `proposed-data.js`; its docstring says what it derives |
| `proposed-data.js` | Generated: the proposed map (with item 7), the table of traits with their consumers, the item 7 table, and what changed |

Run `python3 docs/architecture/propose.py` (with `--check` to compare
without writing). It reads `proposal-base-data.js`, not `map-data.js`, so
regenerating the current map does not change the proposal. It stops if a trait in the map has no
decision, a kkt-config item is missing from `config-split.json`, an item made
private is still used elsewhere, or anything named is not in the map, including
every item and slot the programs' proposed flows name. The
consumer counts are derived: a consumer is a crate other than the trait's
definer and implementers that takes it in a public slot or names it in normal
code.
