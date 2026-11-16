"""Which crates the map draws together, as one group under a head crate.

The rule (Sam, msgs 3487 and 3490): a crate that implements a contract of
exactly one other crate in its own layer goes under that crate, as
kkt-db-sqlite goes under kkt-db. Programs and test support are never
grouped, and a crate that is a partner is never also a head. Nothing else
is grouped, and no list is written by hand. To widen or narrow the rule,
change `head_of` alone: generate.py and the page take its result as it is.
"""


def head_of(crate, layer_of, kind_of):
    """The crate `crate` goes under, or None: the one crate in its own
    layer whose contract it implements, if there is exactly one."""
    name = crate["name"]
    if crate["bins"] or kind_of.get(name) == "test support":
        return None
    implemented = {r["trait_crate"] for r in crate["implements"] if r["trait_crate"] != name}
    same_layer = sorted(t for t in implemented if layer_of.get(t) == layer_of.get(name))
    return same_layer[0] if len(same_layer) == 1 else None


def groups(crates, layer_of, kind_of):
    """{head: [partners, sorted]} for every group on the map. A crate that
    is itself a partner cannot be a head, so its own partners stay apart."""
    heads = {c["name"]: head_of(c, layer_of, kind_of) for c in crates}
    found = {}
    for name, head in sorted(heads.items()):
        if head is None or heads.get(head) is not None:
            continue
        found.setdefault(head, []).append(name)
    return found
