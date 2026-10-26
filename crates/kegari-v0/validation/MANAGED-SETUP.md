# Guided setup, managed declarations and alpha.4 artifact

Alpha.4 implements guided initial local setup and binds an explicit managed
foreground declaration through configuration, preparation, worker contracts and
cleanup. It does not yet enable qualified repository execution or registered
background services. The final release suite passed **1,041 tests**, with zero
failures and two native cases ignored in the ordinary suite. Clippy with warnings
denied, formatting, contract inventory and five staged-binary inspections passed.

| Capability | Implemented and locally checked | Remaining boundary |
| --- | --- | --- |
| Initial local setup | Derive host/OS mapping, generate runtime UUID, require operator/Linear UUID/concurrency, review full non-secret TOML, submit only after `create` | Actual setup, Keychain and service actions have not been exercised |
| Explicit setup scope | Initial provider/workspace scope is empty; seven documented operator roles default, Evaluate is opt-in; remote/existing/supplied-TOML routes retained | Provider identity verification and workspace/account configuration remain explicit subsequent work |
| Registration race | Registration appearing during initial draft review refuses submission; reachable fixture receives no request | Actual bootstrap retains its existing exclusive registration gate for the later race |
| Managed declaration | Closed foreground-v1 policy, bounded pinned executable hashes, narrow command-only tool read roots, optional scratch-build execution | Declaration is not native qualification; services are not declared or implemented here |
| Authority and persistence | Policy changes fence the workspace and require drained configuration replacement; actual tool identity binds preparation, model contracts and retained cleanup isolation | No cold record recreates native execution or qualification authority |
| Generated code | Explicit execution subtree is only owned `scratch/build`; Cargo target directory derives from it; existing and dangling redirected roots refuse | No compiler/test binary was run through this native boundary |
| Compatibility | Handover metadata 3 explicitly requires managed schema 1 and process schema 2 | Alpha.3 cannot directly hand over to schema 3; no installed-runtime migration is performed |

Independent review found two material issues: a registration appearing during
setup review could reroute the initial draft to an existing runtime, and a dangling
scratch-build symlink could be mistaken for a genuinely absent directory. Both
were corrected and covered by regressions before the full suite. Final static
reviews found no additional material issue. New config tests exercise invalid
policy-change fencing; valid managed-policy busy-to-drained replacement is
supported by code review and existing generic gate tests, not a dedicated new
managed positive test or native operation.

Commands use Rust/Cargo 1.99.1 with
`RUSTUP_TOOLCHAIN=stable CARGO_HOME=/private/tmp/kegari-v0-cargo-cache`, from the
Rust workspace root:

```sh
cargo test --locked --release -p kegari-v0 --test cli --test local_cli --test config_harness --test configuration -- --test-threads=2
cargo test --locked --release -p kegari-v0 --lib managed_ -- --test-threads=2
cargo test --locked --release -p kegari-v0 --no-fail-fast -- --test-threads=2 --nocapture
cargo clippy --locked -p kegari-v0 --all-targets -- -D warnings
cargo fmt --all -- --check
python3 crates/kegari-v0/tools/contract_inventory.py --check
cargo build --locked --release -p kegari-v0 --bin kegari-v0 --message-format=json
```

The first focused run passed 95 tests before final race/compatibility refinements;
the managed filter passed six tests. The final source passed all 1,041 tests
across 39 groups. The 503-clause/136-requirement inventory is unchanged and has
no implicitly promoted coverage. [Machine evidence](managed-setup-checks.json)
records exact commands, logs/hashes, source inputs, reviews and limits. Earlier
alpha.3 failures and interrupted attempts remain in their historical report.

Artifact: [kegari-v0 0.2.0-alpha.4](/Users/lgr/Development/com.kegari/com.kegari.rust/target/prereleases/kegari-v0-0.2.0-alpha.4-aarch64-apple-darwin-de3b22d64e82/kegari-v0).
Target: `aarch64-apple-darwin`; size: 40,610,592 bytes; mode: `0555`.
Executable SHA-256:
`de3b22d64e827ad31d687733e7e10d88f90ce65b533faa38a098283e79475fae`.
Independently reproduced source/profile identity:
`sha256:edb11e211e6eceae9200a1d0f043e4283cf5e0a0dc5c3bfd20423ffc71d2ca3e`.

The adjacent artifact manifest records 315 build inputs. A separate frozen
711-file manifest covers implementation, test and evaluation inputs and remained
unchanged through the standalone build. All six embedded assets match source;
asset version remains 3. Configuration/API/receipt schemas remain 1, process
journal schema remains 2, and handover metadata is now 3 with managed schema 1.

The staged binary passed `--help`, `--version`, `setup --help`, `__build-info` and
`__handover-info`, each with closed stdin, a minimal environment, an empty owned
working directory and a 15-second timeout. No cwd files were created. Full tests
used Cargo-generated test artifacts before the separate standalone build; only
those five inspections ran against the staged executable. No complete binary
reproducibility or operational qualification is claimed. Older staged artifacts
are preserved, and required license/notice files accompany this one.

The separately approved [NQ-01A native identity check](NATIVE-IDENTITY.md) passed
once against its preserved alpha.3 test executable. It is narrow native evidence,
not permission for setup or other checks and not managed command qualification.
NQ-01B owned-child signalling is separately prepared for explicit approval; its
status is tracked in PROGRESS.md. No service/coordinator was installed or started,
and no live provider/agent/human evaluation, publication or push was performed.

Next work is the finite authenticated native qualification path and managed
command completion through the existing owned-group/journal/stream boundary,
then registered-service lifecycle and dependent writer/cleanup gates. The goal
and milestone 0.2.0 remain incomplete. Full descendant coverage is still required
before untrusted external context qualification, independently of managed v0.
