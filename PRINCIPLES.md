# Coding principles

These principles apply to all code in kashkeshet. The first section states
general principles; the second states how kashkeshet's code is organized
into parts and crates. CLAUDE.md turns them into specific rules for this
project; where the two seem to differ, CLAUDE.md's rule is the more specific
one.

## General principles

**Declarative Architecture.** The architectural pieces of a system ("infrastructure") and the
configuration of those components ("environment") MUST be defined declaratively and immutably.
Changes to either infrastructure or environment MUST be done by creating a new, versioned
declaration and not by ad hoc configuration of a live system.

**Provenance.** The outputs of a system SHOULD be traceable to the specific version of
infrastructure, environment, and code that created them.

**Modularity.** Code MUST be organized into self-contained modular components each with
responsibility for a particular domain. A component is self-contained if a developer can work in
that module without needing to understand or worry about the functionality of other components.
Domains reflect cross-cutting concerns (e.g., logging, authorization, etc.) or specific features.
Domains do not reflect file types or code kind. For example, a game application might be organized
into components like "player", "levels", "physics", "logging", and "leaderboard"; it should not be
organized into components like "types", "scripts", "configs", "assembly".

**Contracts.** The components of the system MUST clearly define how they behave and are to be used.
This is their contract. New behaviors or usages SHOULD be added to the system and not replace old
features or behaviors. In other words, contracts SHOULD NOT be broken. In addition to being
documented, any exceptions to this principle MUST be done with sufficient notice to users of the
contract.

**Dependency Injection.** A component SHOULD define the contractual requirements of its
dependencies, and implementations of those dependencies SHOULD be provided to the component upon
initialization. In other words, dependencies SHOULD be injected rather than imported.

**Loose Coupling of Dependencies.** Whether or not they are injected, dependencies SHOULD be loosely
coupled from their consuming component. In practice, this typically means that third-party
dependencies SHOULD be wrapped by code reflecting a contract defined by our software, rather than
simply using the contract provided by the dependency's creator. For example, if we want to use a
third-party PDF rendering library, we should wrap it in our own PDF rendering component that
implements our own contract.

**Business Logic Purity.** The business logic of software SHOULD be kept separate from its
configuration and side-effects. Configurations SHOULD be injected at initialization. Side-effects
SHOULD be injected as dependencies or handled on return. Put another way, configuration and
side-effects SHOULD be evident from a component's contract, a function's declaration, or similar
definitions. Note that for these purposes: error throwing is a side effect; time and randomness are
configurations.

**Errors.** An error MUST be handled or propagated, never discarded. Logging an error is a type of
handling. Error propagation SHOULD NOT be used as a substitute for control flow. Unhandled
exceptions, i.e., errors that propagate to the edge of an application, MUST be unexpected.

**Defaults.** Code SHOULD NOT rely on implicit defaults. Code SHOULD either eschew all use of
defaults or explicitly define default behavior.

**Testability.** Test coverage MUST be sufficient to ensure the system as a whole and its components
are operating as expected and do not regress. In general, the level of appropriate test coverage is
left to the discretion of the developer in accordance with this principle. Notwithstanding, tests
MUST cover the following cases: (a) code that purports to fix an identified bug, (b) code that
purports to add fault tolerance or robustness to the system, (c) code that purports to improve the
performance of the system.

## Organizing code

These rules decide how code is split into parts and how the parts depend on
each other.

1. **Add a dependency only for a real reason.** A part that depends on
   another for something minor, such as one small helper, is a sign
   something is wrong.
2. **Depend on what you need, not on a larger contract that contains it.**
   A part states its needs in its own terms; the code that builds it meets
   them, from a larger contract or through a small adapter. A part that
   depends on a whole contract but uses only some of it is harder to
   understand and to fake in tests. A contract small or common enough may be
   shared even if a part does not use all of it. Example: `web/` declares
   `Source` and `Gate`, and `web/real.rs` adapts `rules` and `admission` to
   them.
3. **Keep composition separate from business logic.** Code that wires parts
   together decides nothing. Example: `app.rs` assembles the server from
   its parts; the decisions it serves are made in `rules/`.
4. **Composition must be readable.** A reader must be able to follow how
   something is put together from the code that composes it. If they
   cannot, reorganize it. Example: `app.rs` builds the server from
   `app::Parts` in one place.
5. **Each part is easy to reason about on its own.** If understanding a part
   means reading another's internals, reorganize them.
6. **Where the compiler can enforce a boundary between parts, it should,**
   rather than convention and search. Today that means keeping items private
   (see "Public surface" in CLAUDE.md).

Rules 7 to 9 take effect when the code becomes a workspace of crates (#247);
until then, read "crate" as "part" and apply them to the parts under `src/`.

7. **One crate per clear concern, named for what it does,** not for the
   library or tool behind it. A crate does not bundle unrelated things.
   Example: the crate that runs kkt in containers is `kkt-container`, not
   `kkt-docker`.
8. **A concern's crate holds its contract, its real implementation and its
   shared test stand-in.** The real implementation is behind an `os`
   feature, and the stand-in (such as `FixedClock`) behind a `testing`
   feature. Only programs (the `kkt` binary and examples) and integration
   tests turn on `os`; other crates turn on `testing` only in their
   dev-dependencies. CI builds each crate on its own, which enforces this.
   A fake used only inside one crate stays in that crate's unit tests.
9. **Most tasks stay in one crate.** A task should happen entirely, or
   almost entirely, in one crate. A consumer may need a small change when a
   crate's public surface changes. A task that needs edits to two or more
   crates at once is a sign the crate lines are wrong.
