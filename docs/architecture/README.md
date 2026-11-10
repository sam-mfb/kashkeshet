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
| wires | amber, open arrow | the crate builds the other crate's implementation of a contract, or a type that holds one, to hand it on (a program building `SqlitePlace` for `Db::new`) | items that implement a contract, build one (a public method returns another crate's holder, as `SqlitePlace::open` gives a `Database`), or hold one, which the crate builds: it calls one of the type's constructors (a public function giving back the type, such as `KeyMode::new`), or names it if it has none (`SystemClock`). Calling another function of such a type (`KeyMode::admin_operator`) is using it. Plain settings structs and the traits the wired types implement go with the wiring |

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
