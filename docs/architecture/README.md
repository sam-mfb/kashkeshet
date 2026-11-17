# The crate map

`index.html` is an interactive map of the workspace's crates: one box per
package, in layers, with an arrow for each dependency between crates. Open it in a browser straight from the checkout; it loads nothing from
the network. Click a crate to show it in the panel: its concern, the
public items it provides, and which of them flow along each arrow. Each of
the panel's lists is a section with a count, which the viewer opens and
closes: short ones (five entries or fewer) start open, long ones closed,
and an empty one is not shown. The page keeps each viewer's choice per
section in the browser's storage, and works the same without it. The panel
describes a crate by what it does. Clicking highlights nothing. Tick the checkbox on a box's top corner
to light up that crate's arrows, neighbours and delivery lines; several
ticked crates light up together, and the panel lists them with a button to
clear them. With nothing ticked, every arrow is faded. Ticks and the panel's
crate are kept by name, so changing a filter never moves them to another
crate. Hovering only outlines a box, and shows a highlighted arrow's or a
delivery line's tooltip; faded arrows take no pointer. Every box keeps its
place on the map whatever the filters hide: a hidden crate leaves an empty
slot. The eye on a
program box's bottom corner hides that program like a hidden layer; Show
all and Reset view bring it back, and the dev-only box hides and shows the
dev programs with it. Search finds crates
and items. Above the map, the Crates menu shows or hides crates by kind
(a hidden kind's crates keep their places) and dev-only crates; the Arrows
menu shows or hides arrows by role and by what uses them (code, tests or
build scripts). A crate's panel lists every public item; one that exists
only with the `testing` feature is marked "tests only". Clicking a layer's name on the map hides that
layer: its crates stay in place, faded and still clickable, and no arrow
or delivery line goes to or from them. Each menu has All and None buttons
at its top; "Show all" and "Hide all" above the map turn every layer on or
off; and "Reset view" in the header puts every filter, menu and layer back
as the page starts, clears the search, the ticks and the panel's crate,
and keeps the map shown.

Some crates are drawn as a group: a head crate on top, its partners
stacked directly beneath it in the same layer, and a dotted outline
around them (white in the dark theme, dark grey in the light one; the
key shows it). A crate goes under another when it implements a contract
of exactly one crate in its own layer, as kkt-db-sqlite does kkt-db's,
and kkt-service-launchd and kkt-service-systemd do kkt-service's.
Programs and test support are never grouped, and a partner is never a
head. `groups.py` holds the rule, and nothing else decides it: no group
is written by hand. A group keeps its outline while any of its crates is
drawn, so a partner whose head a filter hides still shows where it
belongs. An arrow meets a box's top or bottom unless another crate of its
group lies that way; then it meets the box's side, so it never seems to
end on a partner. A band wraps onto lines of even length, each as tall as
its tallest group.

It shows surfaces and flows only: public items, their documentation's
first sentence, public fields, methods and variants, and what a feature adds
to them. It never shows private items, private module paths or function
bodies.

## What the page shows

The page shows the code as it is, and only that: it is a tool for
looking at the code, not for planning it. `generate.py` builds it from the
code into `map-data.js`; regenerate it after a change to crates,
dependencies or public items. Apart from each crate's concern and kind
(`concerns.json`) and the programs' assembly (`assembly.json`),
both written by hand and checked against the code, everything on it is
computed from the code.

The overview, shown when no crate is picked, reports two checks: the kind
checks (below), and the rule checks, where the code breaks the rules that
rest on the kinds (`kinds.rule_breaks`): only
programs depend on an effect crate, in code or in tests; only
programs depend on a composition crate in code; and a crate implements another crate's
trait only if that crate is a contract, unless it is a program, a
composition or test support; and no crate is mixed.

| File | What it is |
|---|---|
| `index.html`, `map.css`, `map.js` | The page |
| `concerns.json` | Written by hand: the layers (where in the system a crate sits), the kinds (what sort of crate it is), and each crate's layer, kind and concern, from CLAUDE.md and the crates' docs |
| `groups.py` | Which crates are drawn together, under which head; `generate.py` uses it |
| `kinds.py` | The kinds, the checks a map's kinds must pass, and the rule checks; `generate.py` and `check-programs.py` use it |
| `page-test.js` | A headless check of the page: a ticked crate's injection routes drawn whole, back to the suppliers and on to the final receiver, with no hop labelled only "passed on"; no composition's arrow drawn as an implementation, and an adapting arrow's tooltip naming the injector's own work or the crate its adapter is over; every arrow head on the crate that depends (an implementation arrow's on the implementer), no two heads overlapping, a program's delivery lines leaving it at points of their own, each group's partners beneath their head and outlined, every label inside its box, every arrow ending on its own crates' boxes, no band wider than five boxes on any line (a band wraps at five), no switch of map and no proposal loaded, the overview's kind and rule checks, ticks and the panel's crate kept by name through every filter, Reset view clearing them, no box moving when a filter changes, and only full-strength arrows answering the pointer. Needs Playwright with Chromium; from the repository's root, once: `npm install --prefix target/tmp/page-test playwright` and `target/tmp/page-test/node_modules/.bin/playwright install chromium`; then `NODE_PATH=target/tmp/page-test/node_modules node docs/architecture/page-test.js`. CI does not run it |
| `assembly.json` | Written by hand: how each program (kkt, kkt-keymode, the fake product) assembles the server, step by step, and what each program and injector injects into which slots; every item a step names is checked against the code, and each thing an injector passes on names, in `from`, the slots of its own that a program filled with it, which generate.py checks too |
| `generate.py` | Writes `map-data.js` from the code; its docstring says where each part comes from; with `--out FILE` it writes the map to that file instead and leaves `map-data.js` as it is |
| `check-programs.py` | CI's check that the programs do no machine work, and that the kind checks and rule checks find nothing: builds the map from the code into `target/tmp` and fails on machine work in a composition crate's own code, file by file and library by library, unless the plan's "Temporary breaks" table lists it in a row such as "`kkt-keymode` does machine work in `src/main.rs`". A listed file that does no machine work fails too, so a row goes when its work does. `test_check_programs.py` tests it |
| `map-data.js` | Generated: the crates, arrows and public items, the kind checks and the rule checks, with `concerns.json` and `assembly.json` |
| `test_check_programs.py`, `test_groups.py`, `test_kinds.py` | Tests of the scripts; CI runs them |
| `refactor-plan.md`, `decisions.md`, `refactor-merge.md`, `contract-audit.md` | The refactor's history: its plan, the decisions behind it, the merge's description and the contract audit. `check-programs.py` reads the plan's "Temporary breaks" table |

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
- **logic**: behaviour, with every effect injected (protocol translation counts, so kkt-mcp and kkt-web are logic);
- **effect**: touches the operating system, disk, network, real time or real randomness directly; a named tool wrapper also owns its tool knowledge, tested through a private runner;
- **composition**: wires others and decides nothing (kkt-server and the programs);
- **test support**: for tests only;
- **mixed**: a contract or logic together with a real effect. Named tool wrappers are the explicit exception and are marked effect. A mixed crate breaks a rule (the rule checks).

`kinds.py` checks what it can: a contract that does more than compute on its
own values (behaviour through an injected dependency, or wiring); a contract or
logic crate that touches the machine (machine work in its own code, or an I/O
library); an effect crate that touches it nowhere the map can see; and a
composition crate, programs included, that does machine work in its own code.
Every crate is one kind. `kinds.py` reports a mismatch
on the page and does not change the kind. `check-programs.py` fails CI on the
composition check (machine work in its own code).

Test code is everything under a condition that can hold only for tests or
the crate's own `testing` feature, which only tests turn on:
`test`, `feature = "testing"`, an `all(...)` with such a part (`all(test,
unix)`), or an `any(...)` whose every part is such. It is a module, a function, a `use` or a field. What a crate
names only there belongs to its test arrow ("Used by tests"), never to a
role of its code arrow: kkt-web builds kkt-log's
`CapturedLog` only for its tests, so its code arrow to kkt-log only uses. Likewise a dependency only the crate's own `testing` feature turns on
(optional, and enabled by no other feature, read from Cargo's metadata) is
a test dependency: it is drawn as a test arrow only, with no code arrow, as kkt-db's kkt-search, which only its stand-ins use. The
kind and rule checks treat it as they treat a dev-dependency.
Machine work is what `generate.py` finds in a crate's own code, outside its
tests and outside the modules only tests or the `testing` feature bring (a
stand-in's work is test support's): a third-party crate that reaches the
machine (libc, tracing-subscriber, rusqlite, reqwest, rand); a path under
`std` for files, the environment, processes, sockets, the terminal or the
clock (`std::fs`, `std::env::var`, `std::process::Command`, `std::io::stdin`,
`std::path::absolute` and the like); the same under `tokio` (`tokio::fs`,
`tokio::net`, `tokio::process`, `tokio::signal`, its terminal streams, and
`tokio::time::sleep` and the like); a print macro; or a `Path` method that asks the file system about a
path (`exists`, `is_file`, `canonicalize` and the like, called with no
arguments, so an effect crate's `host.exists(path)` is not counted).
Reading the command line (`std::env::args`) is not machine work: parsing its
arguments is the program's. Value types such as `SocketAddr` and `ExitCode`
are not either. The scan reads names, so it misses calls such as
`path.exists()` on a `Path`. The map records the work by file.

## Roles, contracts and slots

Each arrow its code uses (Cargo's dependency) has one or more roles. It is drawn as one stroke per role, side by side in each role's colour, with one head in the shape of its first role in this order; the header's Arrows menu hides roles, and an arrow with none of its roles shown is not drawn.

Which way an arrow points: every arrow runs from the crate that is depended on to the crate that depends on it, with its head on the dependant. A used or built crate goes into what uses or builds it, the way a part goes into what includes it, and a contract goes into its implementer, which needs the contract to know what to build. Inside a group, the arrows point down from the head crate to its partners. Only the drawing goes this way: the data, the panel's lists ("Includes" for what a crate depends on, "Included in" for what depends on it) and the scripts keep Cargo's sense, in which a crate depends on another.

| Role | Drawn as | Means, for an arrow from A to B | Derived from |
|---|---|---|---|
| implemented by | green, hollow triangle | B implements a trait A defines (kkt-service-launchd's `Launchd` implements kkt-service's `Service`) | the trait impls in rustdoc's JSON, including impls by private types, which the page names only as "a type of its own" |
| adapted by | yellow (its own colour, apart from the amber of "built by" and the injection lines), short dashes (not the injection lines' dots), open round head | B, a composition crate (a program, kkt-server or an assembly), has a type of its own that meets a trait A declares, and injects it: an adapter over another crate's work, or its own work (kashkeshet's adapters for kkt-watch's `Place` and `Report`). Not an implementation of a contract. Its tooltip says, for each type, whether it adapts another crate's work or is the injector's own, from the injection routes | the trait impls, as for "implemented by", where the implementer's kind in concerns.json is composition; computed from the kinds, not a list |
| used by | grey, filled arrow | B calls or reads A's items | every named item that is not one of the others |
| built by | amber, open arrow | B builds A's implementation of a contract, or a type that holds one, to hand it on (a program building `SqlitePlace` for `Db::new`) | items that implement a contract, build one (a public method returns another crate's holder, implementation or contract, as `SqlitePlace::open` gives an `Arc<dyn kkt_db::Backend>`), or hold one, which the crate builds: it calls one of the type's constructors (a public function giving back the type, such as `KeyMode::new`), or names it if it has none (`SystemClock`), or calls a free function of its crate that gives one back (`open_log_file` gives a `RotatingFile`). A type that implements a trait from outside the workspace that some public slot takes (`RotatingFile`, a `std::io::Write`) counts as an implementation. Calling another function of such a type (`KeyMode::admin_operator`) is using it. Plain settings structs and the traits the wired types implement go with the wiring |

Where wired things go comes from the assembly data (assembly.json): what an injector (a program, kkt-server, or one of the two assemblies, kkt-operate and kkt-agent-commands, under "injectors") puts into which slot of which crate. Each injection is one of four. It injects another crate's implementation ("impl"); or its own adapter over another crate's implementation, which names that crate as the supplier that does the work ("own" with "supplier"); or its own work, with no other crate's implementation (a supplier that is the injector itself, such as kkt's `ServeReads`); or it passes on what it was given ("passes_on"). A value of the receiving crate's own type is never an injection into it: `Db`, `Parts`, `Prompter`, `Inspector` and kkt-rules' own `DefaultWording` are not shown as injected, and what goes into them is (generate.py refuses such an entry). Nor is a value of a concrete type the receiver names directly, which the map counts as a slot only because the type holds a contract: kkt-mcp's `endpoint` takes kkt-result-format's `Formatter`, which kkt-server builds and hands over, so what is recorded is the clock that goes into `Formatter::new` (generate.py refuses the other). An injection into a slot typed only as a trait from outside the workspace is checked against rustdoc: the named type implements that trait (`RotatingFile` is a `std::io::Write`), or the named item is a public function and the slot takes a function trait (`kkt_listen::try_bind` into `OperateParts.try_bind`); generate.py refuses any other. A closure counts where it wraps another workspace crate's item for a slot, as an adapter does (the program's log-level check over kkt-log-tracing's `LogLevel::parse`, into kkt-config-file's `parse_file`); a closure of a crate's own logic does not. A factory that builds what the slot's contract needs, such as `SqlitePlace` for `Db::new`, is an injection. A slot such as kkt-rules' `Startup.wording` still shows in its crate's panel, so a product can see it may fill it. A wires arrow's tooltip and its panel entry say so in words. Each injected thing has a route: from the crate that supplies it (another crate's implementation, the crate an adapter is over, or none for the injector's own work), into the injector, then through each injector that passes it on, to where it ends. A hop that passes something on is joined to the hop that fed it on the slots its `from` names: `from` is written by hand in assembly.json and checked by generate.py (it must name a slot of the injector that some program fills), and the page never joins hops by their wording. Ticking a crate draws every route it is on, as the supplier, an injector or a receiver, whole, through crates that are not ticked: ticking kkt-watch draws kashkeshet to kkt-agent-commands to kkt-watch, and the crates that supply what kashkeshet injects. Solid is a dependency, dotted is an injection. A route's first hop, from the crate that supplies what is injected to the injector, is an ordinary dependency, since the injector names the supplier's type to build it: it is exactly one stroke of the solid arrow between them, lit amber like the rest of the route, so a route never changes colour halfway: the "built by" stroke where the arrow has one, labelled with what is built (`TracingLog`); otherwise, where an adapter wraps a plain type of the supplier, the "used by" stroke, labelled with what is wrapped and by what (`Terminal, wrapped by WatchOutput`). Never two strokes of one arrow; the arrow's other strokes keep their own role's colour. A hop whose stroke a filter hides is not drawn. From the injector on, one thin dotted amber line joins each pair of crates on those routes, labelled with what it carries and who supplies it (`WatchOutput (over kkt-terminal)`, `Mcp (kkt-agent-client-mcp)`, `ServeReads (its own work)`), so no hop is only "passed on". A hop that hands on what its crate was given says so once, after what it carries: "· passed on at start-up" where every slot it fills is a field or a constructor's parameter, which keeps the value, and "· passed on per call" otherwise. Any crate that hands on a contract value it was given is an injector here, logic crates included: kkt-rules hands the entropy the program gives it at start-up to kkt-names on each call that draws a handle or a channel ID. generate.py finds each place a crate may do so: a crate other than a program that takes a trait in a slot, and whose code names another crate's slot that takes the same trait directly. Each must be wired in assembly.json, under "injectors", or listed in its "not_passed_on" with why (the crate names the slot's type but never calls it, or the value is the receiving crate's own type); a listed place that is no longer one fails too. What generate.py cannot verify is the call itself: the names on an arrow show which items a crate's code names, not which values it passes, so each entry is confirmed by reading the code. A line that carries only the injector's own work has longer dashes and an italic label. A label sits on its own line, on the stretch nearest the middle that keeps a few pixels off every box, out of every group's outline and off other labels; where every stretch crosses a box, it sits mid-line on a backing of the page's colour. Hovering a line or its label lists each route through it, hop by hop, with the slots it fills. They are not dependencies, and they follow the filters. A line meets each box in a space between the arrows' ends there, so it stays visible beside the arrows, even between a program and the crate just below it; several lines on one face are spread along it like arrows. Each point on a face is one space's edge, however many arrows share it. A face with more lines than spaces shares points among them, on shared stems, as arrows do.

Where arrows meet a box: the ends of the arrows that meet one face of a box are spread along it, ordered by where their other ends lie, so they cross as little as they can. Ends sit 22 pixels apart where the face has room, and never closer than 9, so no two heads touch; they keep clear of the corners, where the tick box and a program's eye sit. A face with more arrows than it can hold 9 pixels apart (a program's bottom face, which most of its arrows reach) shares points: its arrows are split into runs, the arrows of a run end at one point on a shared stem, and only the first is drawn with a head. Arrows that end at the box and arrows that leave it are kept in separate runs, and arrows that end there by their role, so one head never stands for two roles. Each run gets a share of the face's points in proportion to its arrows. Heads are a fixed size, whatever an arrow's width.

A crate's panel lists the contracts it defines (its public traits, with who
implements each), the contracts of other crates it implements, and the
slots it takes: public parameters and fields typed as a workspace trait
(`dyn`, `impl` or a generic bound), or as a type that holds one (`Db::new`'s
`database: Database`, which holds a `Backend`), each with what a program's
wiring fills it with. A slot typed only as a trait from outside the
workspace (`init_logging`'s `log_file: Box<dyn std::io::Write>`, or an
`Fn`) is listed only where a program's wiring fills it with another crate's
work: elsewhere it takes a plain value or a closure. Marker, derive,
conversion and future traits never make a slot. A crate whose work a
program injects lists where it goes: what is carried and the slot it ends
in. A program's panel opens how
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
