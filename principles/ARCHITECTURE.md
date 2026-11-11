# Architectural Principles

The following principles describe how software should be developed regardless of language, technology stack, or level of human involvement.

These principles are a list of rules and preferences for how software is developed.

These principles are not an exhaustive list of best practices or responsible coding.

## Modules

1. Code MUST be organized into self-contained units ("modules").

1.1 Modules MUST be scoped so as to encompass (1) a unitary concern, (2) one that can be comprehended by a single developer without thinking about the code in other modules.

1.1.1 Indications of improper module scope include: (a) modules that consistently need updating in parallel (tight coupling), (b) a module that cannot be understood without reference to another (leaky abstractions).

1.2 Modules MUST be named to reflect their scope, not their implementation, unless multiple modules implement the same scope.

1.2.1 For example, a module that provides VPN functionality should be called "vpn", not "tailscale".

1.2.2 If, however, there are multiple VPN implementations, they may be called "vpn-tailscale" and "vpn-openvpn".

1.2.3 Modules MUST NOT be organized by kind of code: there is no module called "types", "utils", or "helpers".

1.2.3.1 The desire to organize modules by kind of code is an indication that a reorganization is needed. All code SHOULD belong to a domain.

1.3 Modules MUST NOT share decision-making responsibility.

1.3.1 For example, if a module owns the concern of authentication and returns an identity object, other modules MUST be able to rely on that and it MUST NOT be overridden by them.

1.4 Each module MUST state clearly in its documentation what it owns and what it exposes.

1.4.1 A project SHOULD maintain this information consistently across modules.

1.5 Module boundaries SHOULD be compiler enforced (e.g., by using separate packages, crates, or other units of compilation).

## Contracts

2. Modules MUST clearly define their public boundaries ("contracts").

2.1 A module SHOULD expose an item only when another module needs it.

2.2 Modules SHOULD declare their contracts independent of outside sources, unless they are expressly implementing an external contract such as a well-known protocol or a contract module (see 3.2).

2.3 Modules SHOULD use an outside source for a contract when they are required to implement and stay up to date with an external contract.

2.3.1 In this situation it is explicitly intended that the module break when the external contract changes.

2.3.2 Modules that implement external contracts SHOULD be named accordingly, e.g., `vpn` and `vpn-tailscale`.

2.4 Contracts SHOULD NOT break.

2.4.1 New behaviors SHOULD be added to the system rather than replace old behavior.

2.4.2 Exceptions to this rule MUST be done with notice to consumers of the contract.

2.5 Modules SHOULD provide stand-in or mock implementations of their contracts that can be used in testing without the need for significant dependencies.

2.6 Modules MUST have automated tests that test their contract.

## Dependencies

3. Modules MUST state explicitly what they require that is not their concern ("dependencies").

3.1 Modules SHOULD depend only on the precise functionality they need from their dependencies ("loose coupling").

3.1.1 A module declares each such dependency in its own terms, including only the functionality it uses.

3.1.2 If a dependency's contract does not match a module's declared dependency, the dependency SHOULD be provided via adapter logic. See also 4.5.

3.1.2.1 Third-party dependencies SHOULD be wrapped in a module that provides adapter logic to meet a contract provided by the project.

3.1.2.2 Third-party dependencies SHOULD NOT be directly relied on.

3.1.2.3 A module MUST NOT depend on its consumer in order to implement that consumer's dependency (circular dependency).

3.2 If multiple modules require the same dependency a contract module SHOULD be used.

3.2.1 A contract module is a module that declares a contract and is implemented by one or more other modules.

3.2.2 Contract modules are implemented by modules following principle 2.3.

3.2.3 When using a contract module, the contract module is the contract of its implementer and defines the dependency of its dependent module.

3.2.3.1 For example, `vpn` is a contract module, `vpn-tailscale` implements it, and `network` declares a dependency on the contract provided by `vpn`. A parent module injects `vpn-tailscale` into `network` to satisfy the contract of `vpn`.

3.2.4 Stand-ins SHOULD be implemented in the contract module or in a separate testing module, not the implementer.

3.2.5 If idiomatic in a given language, contract modules MAY provide methods that operate on the contract and values used by the contract.

3.2.5.1 Contract modules MUST NOT provide side effects, implementation-specific methods, or configured values.

3.3 Developers MUST apply judgment to determine whether a given situation requires the application of 3.1 (loose coupling/adapters) or 3.2 (contract module).

3.3.1 Loose coupling SHOULD be used for third-party dependencies.

3.3.2 Loose coupling SHOULD be used for one-to-one dependencies (module A, and only module A, depends on module B) where multiple implementations of the dependency are not likely.

3.3.3 Contract modules SHOULD be used if multiple implementations are likely.

3.3.4 Contract modules SHOULD be used if a module is likely to be depended on by more than two other modules.

3.4 Modules SHOULD receive implementations of their dependencies upon initialization ("dependency injection").

## Kinds

4. A module's kind SHOULD be one, and only one, of the following: (1) business logic, (2) side effects, (3) composition, (4) contract.

4.1 Business logic handles the manipulation of data.

4.1.1 Business logic SHOULD return new data ("immutability").

4.2 Side effects handle interaction with the outside world.

4.2.1 The following things are examples of side effects: throwing errors, reading time, obtaining randomness, logging.

4.2.1.1 Returning an error is not a side effect.

4.3 Composition unites business logic and side effect modules together.

4.3.1 Composition can be achieved by ordering or by dependency injection.

4.3.2 Composition code SHOULD be clear and readable by a human. Reading compositional code should make it clear which business logic is being implemented and what side effects it will have.

4.3.3 Composition code MUST NOT itself implement side effects.

4.3.4 Composition code SHOULD NOT itself implement complex business logic.

4.3.4.1 Simple business logic, such as adapter logic, is acceptable in composition code.

4.4 Contract modules are used to facilitate dependency injection as explained in 3.2.

4.5 Adapter logic facilitates loose coupling as explained in 3.1.2.

4.5.1 Simple adapter logic SHOULD be included in composition modules.

4.5.2 Complex adapter logic SHOULD be provided via a separate business logic module.

## Errors

5. An error MUST be handled or propagated, never discarded.

5.1 Logging an error is a form of handling.

5.2 Error propagation SHOULD NOT be used as a substitute for control flow.

5.3 An error that propagates to the edge of the application MUST be unexpected.

## Deployment

6. A project MUST include information necessary for a deployment.

6.1 Information necessary for a deployment includes the deployed pieces of a system ("infrastructure"), the configuration of those components ("environment"), and documentation sufficient to allow the deployment, operation, and debugging of the system ("runbooks").

6.2 Deployment information MUST be defined immutably and in writing.

6.3 Deployment information SHOULD be defined declaratively.

6.4 Changes to a deployed system MUST NOT be done by ad hoc configuration of a live system.

6.5 The outputs of a deployed system SHOULD be traceable to the specific version of infrastructure, environment, and code that created them.

## Data

7. Data written by a previous version MUST NOT cause subsequent versions of the software to break.

7.1 Subsequent versions SHOULD maintain compatibility with, or offer to upgrade, data written by prior versions.

7.2 Mechanisms for writing and reading data (e.g., database query languages) SHOULD be hidden from modules that do not own the data side effect.

## Testability

8. Test coverage MUST be sufficient to ensure the system as a whole and its components are operating as expected and do not regress.

8.1 In general, the level of appropriate test coverage is left to the discretion of the developer in accordance with this principle.

8.2 Tests MUST cover the following cases: (a) code that purports to fix an identified bug, (b) code that purports to add fault tolerance or robustness to the system, (c) code that purports to improve the performance of the system.

8.3 Tests SHOULD run in CI.

8.3.1 Tests that do not run in CI MUST have some other method of ensuring they will be run.

## Documentation

9. A project MUST include documentation.

9.1 Documentation MUST explain the purpose of the software.

9.2 Documentation MUST explain how to consume the software, whether by other software components or by end users.

9.3 Documentation SHOULD be readable by humans.

9.4 Documentation SHOULD explain choices in software that are not obvious to a reasonably skilled person reading the code.

9.5 Documentation MUST explain choices that deviate from these principles or that deviate from best practices of software development consistent with these principles.

9.6 A given piece of documentation SHOULD be self-contained.

9.6.1 It SHOULD NOT contain cross-references to other documentation of the project.

9.7 Documentation of a module SHOULD NOT require knowledge of other modules.
