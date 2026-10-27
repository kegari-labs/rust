# Local generic environment and resource inspection

Partial SPEC 1.4 evidence for 0.2.0 / M02-2–M02-4, following clean `main` at
`902adc6eaa14e0e951f734778312d0a9113603e3`. No minor milestone closes.
Package remains development `0.2.0-alpha.9`; instruction assets advance to 7.

## Implemented boundary

The command renderer and descriptive environment observation share the exact
minimal environment. Inspection reports the retained original/current domain,
cwd, policy, search order and lifecycle/network limitations without a probe,
executable lookup, inherited environment or qualification claim. Read-only roles
receive explicit unavailability when they have no command domain.

Actual workers expose `context_read` / `execution_environment` and bounded
`resource_list` / `resource_read`. Handles derive from the original admitted
repository/test-resource roots and identity, never a caller-supplied root or
ambient PATH. File grants accept only `.`; directory paths are normalized,
non-Git and descriptor-relative with nofollow/nonblocking opens. Inspection
never executes code or creates another resource grant.

File bytes, directory entries and root identities are checked against retained
bindings. Two directory observations use separate stream offsets and must agree;
this is not an atomic filesystem-snapshot claim. Listings describe their visible
non-Git scope; empty output cannot establish an unborn repository. Forbidden
children, links on a read path, hardlinked/nonregular files, changed observations,
oversized results and inaccessible roots refuse rather than yielding partial facts.
Successful observations receive usable discovery references only after the actual
`tool_finished` journal record. Proposal enrollment rereads those observations,
retains the original operation/correction identity, and checks current authority.

Preparation has one finite five-second root-identity capture. Subsequent new
inspection calls and all new supporting-reference rereads use the original worker
or evaluation cutoff intersected with a five-second inspection bound. Shared
worker accounting covers attempted reads, failed operations and encoded output:
8 MiB and 8,192 inspected entries; each complete listing permits at most 256
entries and a text read at most 48,000 bytes. The tool response still has its
64 KiB encoded limit; environment description has a separate 48 KiB bound.
Synchronous filesystem calls remain nonpreemptible. Legacy Source-only reads
retain their prior validation path and local metadata bound; these are not
relabeled as new original-cutoff inspection. Full launch validation remains required.

## Review corrections

Independent review corrected repeated directory stream offsets, forbidden-child
metadata exposure, failed/growing-read budget accounting, deadline renewal inside
metadata validation, and post-read owner/correction freshness. Retained inspection
now avoids recursive writable-tree scans; actual launch still performs its full
validation. The shared source opener also uses nonblocking final-component opens
so a substituted FIFO cannot block before its regular-file check.

The new FIFO regression initially placed its legacy-source assertion after creating
a deliberately forbidden hard link. That ordering exercised the earlier hard-link
refusal, not FIFO handling. The observed failure is retained and only the ordering
is corrected.

## Validation

All Cargo commands use stable Rust/Cargo 1.99.1 and
`CARGO_HOME=/private/tmp/kegari-v0-cargo-cache`, with `--offline --locked`.
Initial all-target compilation and formatting passed. Full release regression on
source `sha256:43a84a52ea902199a2a484cecff990aa2702b49ef2c7ce22cf0b753618df341c`
finished **1,240 passed, one failed, three native ignored**, across 40 groups.
All 746 inputs matched after the run. The sole failure was the described FIFO
fixture ordering, with actual `scope_violation` versus expected `tool_limit`.
The unchanged assertion now runs before the independent hard-link adversarial
input. No production change was needed for that failure.

Strict all-target lint then found two collapsible guards. They were collapsed
without suppression. Independent comparison of all 746 inputs confirms the final
source differs from the full-run source only in that test and the two equivalent
guard rewrites; no other input changed or disappeared. Evaluation order, error
propagation and original deadline checks are preserved.

Final source:
`sha256:d549c89024700ae66c9a3412538c0bed45e1be94926a742dd3919d1fa001da4e`.
Strict all-target Clippy passes. On that frozen source, **all six worker inspection
tests and all 17 core command/environment observation tests pass**. These cover
13 new test functions. The original failed full-run result is retained; unchanged
passing targets were not rerun or relabelled as an exact-final-source full pass.
The actual-worker cases use ordinary owned files/Git fixtures and injected host
components. They do not manufacture native/model/provider qualification.

Formatting, whitespace and inventory consistency pass; 11 inventory tests pass.
The release-profile development executable builds successfully. Static metadata
after build completion agrees with an independent calculation over 346 build
inputs. All 746 logical inputs still match after testing and build.

- Development executable: `target/release/kegari-v0`, 43,752,480 bytes.
- SHA-256: `2833729477093d9a8408c1f2d56454e1add387df4b9a3a855a39011ee2927a55`.
- Build identity: `sha256:f8bc98922c4e8a54e7f7fee97ae0c8d4e793de1eeff018c3b9e14b75005dc66d`.
- Metadata: development 0.2.0-alpha.9; assets 7; handover 4; managed 5; process 3;
  workflow-source 1; API/config/receipt/instruction schema 1.
- Archive: `target/validation/m02-inspection-d549c8902470`, 531 files plus
  manifest SHA-256 `c4d019a26bf4d1d2ab4f1993ca3e56253691fd5cefc13a591708a5cdf3abd300`.

All archive hashes were verified. Complete snapshots 1–4 use 507 shared content
blobs; failed/successful logs, metadata, independent recomputation and the final
development executable remain available. Source files under `src/`, including
its test modules, enter the build identity; final metadata is deliberately distinct
from the earlier full-regression build. This is not a release package, installation
or operational qualification. All owned Cargo/test handles and delegated work are
terminal.

Commands:

- `cargo +stable check -p kegari-v0 --all-targets --offline --locked`
- `cargo +stable test -p kegari-v0 --release --offline --locked --no-fail-fast`
- `cargo +stable clippy -p kegari-v0 --all-targets --offline --locked -- -D warnings`
- `cargo +stable test -p kegari-v0 --release --lib resource_tests:: --offline --locked`
- `cargo +stable test -p kegari-v0 --release --lib isolation::observation_tests:: --offline --locked`
- `cargo +stable build -p kegari-v0 --release --bin kegari-v0 --offline --locked`
- `cargo +stable fmt --all -- --check`
- `python3 crates/kegari-v0/tools/contract_inventory.py --check`
- `PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover -s crates/kegari-v0/tools -p 'test_contract_inventory.py'`

## Remaining work

External tools outside original scope still need legitimate resource authority;
PATH discovery cannot provide it. Mandatory CI/no-change policy in
`config.rs::Repository::validate` still blocks independent onboarding too early
and needs a separate correction preserving downstream gates. Empty/unborn source
identity, authenticated output/review paging, discovered continuation/reassembly,
native viability and broader actual-agent evidence remain incomplete. Existing
native approvals remain consumed; no native/live request or operation was made.
Follow [MILESTONES.md](../MILESTONES.md).
