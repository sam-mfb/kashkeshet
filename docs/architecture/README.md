# The crate map

`index.html` is an interactive map of the workspace's crates: one box per
package, in layers, with an arrow from each crate to each crate it depends
on. Open it in a browser straight from the checkout; it loads nothing from
the network. Hover a crate to light up its dependencies and the crates that
use it, and click it for its concern, what it must not do, the public items
it provides, and which of them flow along each arrow. Search finds crates
and items; the switches show or hide dev-only crates, dev and build
dependencies, and the items only the `real` or `testing` features make.

It shows surfaces and flows only: public items, their documentation's
first sentence, public fields, methods and variants, and what a feature adds
to them. It never shows private items, private module paths or function
bodies. A crate's "must not" appears only where CLAUDE.md states one.

| File | What it is |
|---|---|
| `index.html`, `map.css`, `map.js` | The page |
| `concerns.json` | Written by hand: the layers, and each crate's layer, concern and what it must not do, from CLAUDE.md and the crates' docs |
| `assembly.json` | Written by hand: how each program (kkt, kkt-keymode, the fake product) assembles the server, step by step; every item a step names is checked against the code |
| `generate.py` | Writes `map-data.js` from the code; its docstring says where each part comes from |
| `map-data.js` | Generated: the crates, arrows and public items, with `concerns.json` |

To refresh it after the code changes, run

```
python3 docs/architecture/generate.py
```

from anywhere in the checkout. It needs the pinned toolchain, and builds
rustdoc's JSON (`RUSTC_BOOTSTRAP=1`, since that output is not yet stable)
into `target/architecture-map`. It stops if a package has no entry in
`concerns.json`, or an entry names a package that is gone: add or remove the
entry and run it again.

## Roles, contracts and slots

Each normal arrow has one or more roles, and is drawn as its strongest:

| Role | Drawn as | Means | Derived from |
|---|---|---|---|
| implements a contract of | green, hollow triangle | the crate implements a trait the other crate defines (kkt-agent-client's `Remote` implements kkt-watch's `Server`) | the trait impls in rustdoc's JSON, including impls by private types, which the page names only as "a type of its own" |
| uses | grey, filled arrow | the crate calls or reads the other crate's items | every named item that is not one of the others |
| wires | amber, open arrow | the crate builds the other crate's implementation of a contract, or a type that holds one, to hand it on (a program building `SqlitePlace` for `Db::new`) | named items that implement a contract, build one (a public method returns another crate's holder, as `SqlitePlace::open` gives a `Database`), or hold one and whose `new` the crate calls; plain settings structs and the traits the wired types implement go with them |

A crate's panel lists the contracts it defines (its public traits, with who
implements each), the contracts of other crates it implements, and the
slots it takes: public parameters and fields typed as a workspace trait
(`dyn`, `impl` or a generic bound), or as a type that holds one (`Db::new`'s
`database: Database`, which holds a `Backend`). A program's panel opens how
it is assembled: the hand-written flow from `assembly.json`, and, derived
from the code, which implementation it picks fills which slot. Every
section says whether it is generated or hand-written.

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
write in anything but the commit it names. CI does not run it.
