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

What flows along an arrow is read from names: the paths a crate's code
names in the other crate, matched to that crate's public items. An item
used only through a method call, a trait or a derive does not show, and an
arrow whose crate is named nowhere says so.

Run the script after a change to crates, dependencies or public items, and
commit `map-data.js` with it. CI runs it with `--check`, which fails if the
committed file differs from what it would write in anything but the commit
it names.
