<h1 align="center">Kegari — Codex Orchestration</h1>

<p align="center"><strong>0.2.0-alpha.10 · Development preview</strong></p>

<p align="center">
  A fork and extension of the
  <a href="https://github.com/kegari-labs/symphony">OpenAI Symphony specification</a>.
</p>

Kegari is designed to coordinate Codex agents around **Linear work and GitHub
repositories**, bringing scoped implementation, testing, review and human
acceptance into one workflow. Define the outcome and steer corrections while
the system coordinates the work and gathers evidence for your decision.

Its technology-agnostic approach lets agents discover commands from repository
guidance, documentation and configuration. Projects should not need
language-specific adapters or operator-maintained command inventories. The goal
is less routine supervision, clearer delivery evidence and recoverable work
across different repositories and technology stacks.

**Preview status:** you can build and inspect the macOS CLI today. End-to-end
native operation is still being qualified; this preview is not ready for
production use or unattended orchestration.

## Features and roadmap

Checked items are available preview checkpoints. Unchecked items are target
milestones, including capabilities with partial implementations; they are not
release-date commitments.

- [x] **0.2.0-alpha.10 — Development preview.** Build the macOS CLI and inspect
  its commands and setup guidance.
- [ ] **0.2.0 — Repository-aware execution.** Discover project commands and run
  scoped work without toolset-specific configuration requirements.
- [ ] **0.3.0 — Recoverable workflows.** Resume interrupted work while preserving
  context, decisions, execution budgets and unresolved operations.
- [ ] **0.4.0 — Delivery evidence.** Connect external targets and visual evidence
  to the exact changes under review.
- [ ] **0.5.0 — Installation and handover.** Establish supported installation,
  upgrades and operator handover with compatibility checks.
- [ ] **0.6.0 — Repeatable evaluation.** Exercise representative workflows and
  assess observed behavior against defined outcomes.
- [ ] **0.7.0 — Validated managed operation.** Complete native, provider and
  actual-agent qualification for the supported workload scope.
- [ ] **1.0.0 — Production readiness.** A future readiness milestone whose scope
  and timing remain uncommitted.

See the [detailed milestones](crates/kegari-v0/MILESTONES.md) for completion
criteria and remaining work.

## Installation: build the preview

You will need:

- **macOS on Apple silicon**, the currently verified executable build target.
- **Rust 1.99.1**, installed through [rustup](https://rustup.rs/). The repository's
  toolchain file selects the pinned version automatically.
- **Xcode Command Line Tools**, or equivalent build tools supplied by Xcode,
  for the native compiler and linker.
- **Git** to clone this repository.

Clone the source and build the optimized executable:

```sh
git clone https://github.com/kegari-labs/rust.git kegari
cd kegari
cargo build --release --locked -p kegari-cli --bin kegari-v0
```

The executable is written to `target/release/kegari-v0`. Explore it without
creating a runtime or starting services:

```sh
./target/release/kegari-v0 --version
./target/release/kegari-v0 --help
./target/release/kegari-v0 setup --help
```

Rust is required to build from source, not to run an already-built executable.
Launching `kegari-v0` without a command requests local status; before setup it
reports an unavailable runtime rather than starting an orchestrator.

For eventual operation, the current compatibility pins are **Git 2.56.0** and
**Codex CLI 0.164.0-alpha.11.1**, with authorized Linear and GitHub connections.
Remote control additionally requires Tailscale identity and TLS configuration.
Service activation remains subject to the qualification and installation
milestones above; these build instructions do not install a background service.

## Documentation

[Configuration](crates/kegari-v0/docs/configuration.md) ·
[CLI guidance](crates/kegari-v0/docs/cli.md) ·
[Milestones](crates/kegari-v0/MILESTONES.md) ·
[Developer guide](docs/development.md)

## Credits

Luke Gareth Ribchester

## Licenses

Kegari's original Rust implementation is licensed under the [MIT License](LICENSE.md).

The project forks and extends the
[OpenAI Symphony specification](https://github.com/openai/symphony/blob/be10a1b79df723d6d7612b5651c8522704dafb2e/SPEC.md),
retained in the [Kegari Labs Symphony fork](https://github.com/kegari-labs/symphony).
The resulting [Kegari specification](crates/kegari-v0/SPEC.md) is a modified
adaptation under **Apache License 2.0**, with the upstream OpenAI attribution,
NOTICE and license preserved in its
[attribution section](crates/kegari-v0/SPEC.md#15-attribution-and-apache-20-license).
The Rust application implements that adapted specification; it is not a port of
Symphony's Elixir reference implementation.
