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

1.2.3 Modules MUST NOT be organized by kind of code: there is no module called "types", "utils", "helpers", or "settings".

1.2.3.1 The desire to organize modules by kind of code is an indication that a reorganization is needed. All code SHOULD belong to a domain.

1.3 Modules MUST NOT share decision-making responsibility.

1.3.1 For example, if a module owns the concern of authentication and returns an identity object, other modules MUST be able to rely on that and it MUST NOT be overridden by them.

1.4 Each module MUST state clearly in its documentation what it owns and what it exposes.

1.4.1 A module's statement of its ownership and concern SHOULD be stated in terms of functionality not implementation.

1.4.1.1 For example, a vpn module should not be described as "it provides access to tailscale" but "it provides remote access functionality".

1.4.2 A project SHOULD maintain this information consistently across modules.

1.5 Module boundaries SHOULD be compiler enforced (e.g., by using separate packages, crates, or other units of compilation).

1.5.1 The boundaries of a module do not need to be compiler enforced if there is a more idiomatic way to separate these boundaries from the consumer in a given language.

1.5.1.1 The rules that will govern this separation MUST be stated in a language specific principles file in the project.

1.5.1.2 The rules that govern this separation SHOULD still be enforced by automated means where feasible and reasonable.

1.6 A human MUST approve the decision to create, delete, or materially change the concern of a module.

1.6.1 The fact that two modules share duplicate code is not in and of itself a reason to create a new module (over abstraction).

## Contracts

2. Modules MUST clearly define their public boundaries ("contracts").

2.1 A module SHOULD expose an item only when another module needs it.

2.2 Modules SHOULD declare their contracts independent of outside sources, unless they are expressly implementing an external contract such as a well-known protocol or a contract module (see 3.2).

2.3 Modules SHOULD use an outside source for a contract when they are required to implement and stay up to date with an external contract.

2.3.1 In this situation it is explicitly intended that the module break when the external contract changes.

2.3.2 Modules that implement external contracts SHOULD be named accordingly, e.g., `vpn` and `vpn-tailscale`, where `vpn-tailscale` indicates it is an implementation of the `vpn` contract.

2.4 Contracts SHOULD NOT break with new released versions.

2.4.1 New behaviors SHOULD be added to the system rather than replace old behavior.

2.4.2 Exceptions to this rule MUST be done with notice to consumers of the contract.

2.5 Modules SHOULD provide stand-in or mock implementations of their contracts that can be used in testing without the need for significant dependencies.

2.6 A module's contract MUST be tested by automated tests.

2.7 Contracts MUST be defined in terms of functionality not implementation.

2.7.1 VPN.login_to_tailscale() is not an acceptable name.

2.7.2 VPN.connect() is an acceptable name.

## Dependencies

3. Modules MUST state explicitly what they require ("dependencies").

3.1 Unless they are implementing a contract, modules SHOULD remain loosely coupled from their dependencies.

3.1.1 A goal of this principle is to avoid modules needing to change any time a dependency is changed.

3.1.2 A goal of this principle is to avoid requiring developers to dedicate mental bandwidth to a deep understanding of all dependencies.

3.2 Loose coupling SHOULD be facilitated by either a "shared contract" model or an "adapter" model.

3.2.1 In a shared contract model, both the target module and its dependency share a contract. The target module depends on the contract; the dependency implements the contract.

3.2.1.1 For example, `app` might depend on a `vpn` module's contract that describes remote access functionality and `vpn-tailscale` implements that contract. In this way `app` remains unaware of the details of `vpn-tailscale`'s implementation and is indifferent as to changes that do not affect the `vpn` contract.

3.2.2 In an adapter model, the target module and its dependency do not explicitly share types. The target module declares what contract it needs; the dependency declares what contract it provides; adapter code adapts the one to the other.

3.2.2.1 For example, `app` might declare it requires a `string_format()` function, and `strings` may provide a `Strings.format()` method. Adapter code will adapt the latter to satisfy the dependency of the former.

3.3 Dependency injection SHOULD be used to fulfill dependencies.

3.3.1 In the 'shared contract' model, the target module declares its dependency on the shared contract and a parent component injects the implementing module.

3.3.2 In the 'adapter' model, the target module declares its needed dependency and the parent provides or incorporates adapter code to adapt the dependency module to that.

3.4 Direct dependency without injection (tight coupling) MAY be used in the following situations:

3.4.1 Dependencies that involve only logic provided that coupling is limited to the module's provided contract.

3.4.2 Dependencies that are being injected by a parent module are directly depended on by the parent module.

3.4.3 Modules that are being composed together in a composition module (see 4).

3.5 A human MUST approve the decision of how to structure a module's dependency (loose-coupling vs. direct dependency, adapter vs. shared contract), considering the following principles.

3.5.1 The adapter model allows the target module to declare only the precise dependencies it requires. That can reduce cognitive load on the developer of the target module.

3.5.2 The shared contract model allows the dependency module to provide a unitary block of related functionality. That can reduce cognitive load on the developer of the dependency module.

3.5.3 What is considered idiomatic or best-practice in the language or framework being used.

3.5.4 The complexity of the adapter code required in the adapter model. The need to maintain a complex adapter may be a sign that the wrong model has been chosen.

3.5.5 The surface area exposed to the target module. A target module that depends on a large shared contract when it requires only a small amount of functionality may be an indication that the wrong model has been chosen.

3.5.6 Whether multiple modules rely on the dependency.

3.5.7 Whether multiple implementations of the dependency are intended or likely.

3.5.8 Whether the module holds shared state.

3.5.8.1 Shared state SHOULD be owned by one module.

3.5.8.2 Modules that only need access to a part of shared state SHOULD be provided a limited view of that shared state.

3.6 Third-party dependencies MUST NOT be directly relied on, but instead wrapped in code that exposes a contract controlled by the project.

3.6.1 A third-party dependency is one that is not under the control of the project developers.

3.6.2 A third-party dependency does not include code or libraries that are native to the language or framework being used.

3.6.2.1 This includes idiomatic language extensions. For example, `futures` in Rust.

3.6.2.2 A human MUST approve language extensions that are not wrapped and that decision MUST be recorded in a language-specific principles file (e.g., principles/RUST.md).

3.6.3 Logic-only third-party dependencies MUST be wrapped but the wrapper MAY be depended on directly.

3.7 A module MUST NOT depend on its consumer in order to implement that consumer's dependency (circular dependency).

3.8 In the shared contract model, the code providing the contract and the code implementing the contract MUST properly divide responsibility.

3.8.1 Stand-ins or mocks of contract functionality SHOULD be the responsibility of the contract provider, not the implementer.

3.8.2 If idiomatic in a given language, contract code MAY provide methods that operate on the contract and values used by the contract.

3.8.3 Contract code MUST NOT provide effects, implementation-specific methods, or configured values.

3.8.3.1 A configured value is a specific value chosen for a given deployment, not a type, default, or immutable constant.

3.9 Only a composition module MAY depend directly on an effect module or on another composition module.

## Kinds

4. A module's kind SHOULD be one, and only one, of the following: (1) logic, (2) effects, (3) composition, (4) contract.

4.1 Logic handles the manipulation of data or other logic.

4.1.1 Logic SHOULD return new data ("immutability").

4.2 Effects handle interaction with the outside world.

4.2.1 The following things are examples of effects: throwing errors, reading time, obtaining randomness, logging.

4.2.1.1 Returning an error is not an effect.

4.2.2 Effect modules MAY include logic that is properly the concern of the effect.

4.3 Composition unites logic and effects modules together.

4.3.1 Composition can be achieved by ordering or by dependency injection.

4.3.2 Composition code SHOULD be clear and readable by a human. Reading compositional code should make it clear which logic is being implemented and what effects it will have.

4.3.3 Composition code MUST NOT itself implement effects.

4.3.4 Composition code SHOULD NOT itself implement complex logic.

4.3.4.1 Simple logic, such as adapter logic, is acceptable in composition code.

4.3.4.2 Simple adapter logic SHOULD be included in composition modules.

4.3.4.3 Complex adapter logic SHOULD be provided via a separate logic module.

4.4 A contract module provides the contract in the shared contract model (see 3.2.1).

4.4.1 Contract modules MAY include logic as permitted by 3.8.

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

9.8 Documentation SHOULD NOT include history (e.g., changes made or the reasons for them) unless the documentation file is a change log or other file dedicated to recording changes.
