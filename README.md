<h1 align="center">Kegari — Codex Orchestration</h1>

<p align="center"><strong>0.2.0-alpha.10 · Development preview</strong></p>

<p align="center">
  <a href="https://rust-lang.org/"><img src="https://img.shields.io/badge/Rust-1.99.1-orange?style=for-the-badge&amp;logo=rust" alt="Rust 1.99.1"></a>
  <a href="#prerequisites"><img src="https://img.shields.io/badge/macOS-Apple_silicon-black?style=for-the-badge&amp;logo=apple" alt="macOS / Apple silicon"></a>
  <a href="LICENSE.md"><img src="https://img.shields.io/badge/Code-MIT-blue?style=for-the-badge" alt="Code / MIT"></a>
</p>

<p align="center">
  An extension of
  <a href="https://github.com/kegari-labs/symphony">OpenAI Symphony</a>.
</p>

Kegari is built around complete, end-to-end features. It coordinates focused
Codex runs across the technologies involved, giving each agent a clearly defined
responsibility, technology boundary and relevant context. Implementation,
integrated testing and review come together around a working outcome, with
evidence for your final acceptance.

Designed to run as a background service on your Mac, Kegari connects work in
Linear with changes in GitHub. Follow progress, guide corrections and review
results from your Mac or another authorized computer. Planned dashboards will
bring work, results and decisions into one view.

**Development preview:** CLI inspection is available; Codex-backed end-to-end
operation remains incomplete.

The approved [SPEC 1.5](crates/kegari-v0/SPEC.md) makes Codex app-server, the
programmatic interface provided by Codex, the repository execution layer; Kegari
orchestrates its work. The native path is being integrated and the duplicated
repository execution harness has been removed. See the revised
[milestones](crates/kegari-v0/MILESTONES.md) and
[six-hour goal](crates/kegari-v0/GOAL.md). The current run state and fixed deadline
are recorded in [PROGRESS.md](crates/kegari-v0/PROGRESS.md). Existing alpha.10
artifacts are unchanged.

## Features

| Feature | Version | Status |
| --- | --- | --- |
| Build and explore the macOS CLI | `0.2.0-alpha.10` | Available preview |
| Orchestrate native Codex work with usable discovery and status | `0.2.0` | Planned |
| Resume interrupted work with context and budgets intact | `0.3.0` | Planned |
| Complete replacement delivery and visual/external evidence | `0.4.0` | Planned |
| Install, upgrade and hand over reliably | `0.5.0` | Planned |
| Evaluate agent behavior across representative workflows | `0.6.0` | Planned |
| Complete validation of supported workflows and integrations | `0.7.0` | Planned |
| Production readiness | `1.0.0` | Future; scope uncommitted |

## Getting Started

### Prerequisites

- **macOS on Apple silicon**, the currently verified executable build target.
- **Rust 1.99.1**, installed through [rustup](https://rustup.rs/). The repository's
  toolchain file selects the pinned version automatically.
- **Xcode Command Line Tools**, or equivalent build tools supplied by Xcode,
  for the native compiler and linker.
- **Git** to clone this repository.

### Installation

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

Running without a command checks local status; it does not start an orchestrator.

## Read More

[Configuration](crates/kegari-v0/docs/configuration.md) ·
[CLI reference](docs/cli.md) ·
[Workspace overview](docs/workspace.md) ·
[Developer guide](docs/development.md)

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
