# Delivery coordinator integration

The delivery adapter is being connected in explicit phases. A `DeliveryAdmission`
binds a private reconciler `FreshGate`, an actual host permit, the work generation,
repository, tested revision, phase and typed repository configuration digest.
Each method rechecks current authority. Provider mutations pass through
`ProviderBroker` with stable, persisted per-action request IDs. A missing response
never causes the same logical action to receive a fresh request ID.

`GateCheck` runs inside the control host's serialized mutation gate and must not
reenter `ControlHost`. The host supplies its current reconciled projection and
configuration checks with the required host-before-configuration lock order.
Individual admitted phases do not grant a general run-all capability.

Implemented adapters currently include:

- Local-check conversion from private successful confined-execution capabilities;
  deterministic execution fixtures are rejected as production gate evidence.
- Hosted observations from actual workflow attempts, complete job/check reads and
  signed GitHub OIDC verification. Proof routes come from typed repository
  configuration; neither job labels nor worker-selected workflow identities prove
  hosted execution. Current attempts and required check inventory are enforced.
- Exact no-change dispatch from the configured pinned workflow ref, with one
  durable request ID and host-created source/operation nonce. Actual private
  GitHub artifacts are downloaded through the verified provider grant, parsed in
  memory, and checked against the configured source output, tested tree, run,
  attempt, dispatch request and complete required job inventory. Triggering
  workflow SHA and tested source SHA remain distinct.
- Scoped proposed deltas from actual Git objects and the exact authorized child
  boundary. The pre-merge review digest binds reviewed bytes, paths, modes,
  base/head, scope and work generation without depending on a future squash SHA.
  The full staged delta identity separately includes observed squash provenance.
- Child PR creation with canonical branch naming, selected-user observation and
  signed parent/child/generation correlation.
- Signed Codex review publication, independent reread of the exact provider record,
  and matching current `kegari/codex-review` status. No-change review is published
  to its child without an invented PR/status. Failed reviews remain failed.
- Current-base/squash/status protection checks. Same-account routes that require a
  formal approving or code-owner review are reported incompatible with unattended
  staging; protection settings are never modified or bypassed.
- Staging merge request with separate local, hosted, review and protection gates;
  acknowledgement remains pending until actual provider parent/tree and retained
  Git-object/patch equality prove the squash. Signed intent and delta artifacts
  are retained in host-private storage.
- Signed changed/no-change child result publication after distinct gate checks,
  followed by separately observed configured Linear closure. The original closure
  transition is retained so replay after a successful state change reuses the
  same receipt. Recovery authenticates the whole record and rereads current
  review and hosted attempts; changed results also require the retained Git patch
  and original staging squash provenance.
- `OutcomeSink` registration against the admitted worker contract and storage of
  signed pending claims. Completed-turn, work, instruction and tool-contract
  provenance are checked. A claim or public check record never becomes a passed
  delivery gate merely because it was deserialized.
- Exact child object publication consumes a private `CommittedSource` and matched
  scoped-change/local-check capabilities. It compares acknowledged blob, tree and
  commit IDs with actual local objects, creates an exclusive canonical branch or
  advances a previously sealed owned branch, then rereads ref/tree/parent before
  retaining signed ownership. Retained branch recovery checks those provider facts
  again. The new end-to-end object-publication failure fixtures are still pending;
  compiling this adapter is not qualification of the complete route.

`release::verify_release_eligibility` provides a sealed read-only readiness proof
using the normal release and queue predicates. Manual mode can truthfully remain
awaiting a release command. This proof neither invents an operator command nor
reserves resources; actual admission still requires the authenticated command.

## Evidence and unfinished connections

The delivery integration fixtures exercise real reconciliation, HMAC, durable
control/broker and provider-parsing code with explicitly injected HTTP data.
They verify canonical review publication, one-write replay, lost-acknowledgement
holds, tampered records, stale authority, failed-review refusal, source-bound OIDC
proofs, owned artifact download, bounded ZIP validation, authentic child recovery,
newer failed/rerun invalidation and separate idempotent closure. Synthetic signing
keys and explicitly retained fixture check records remain deterministic evidence. They do not
qualify real provider access, Codex sessions, sandbox execution or media playback.

The following production connections remain in progress: runtime wiring and
complete provider-failure coverage for scoped commit/object publication, complete
isolated parent assembly and integrated evidence, visual
upload/access association, human acceptance and separate closure, production
release step dispatch/observation, owned-run cancellation, and complete explicit
receipt/successor recovery, replacement-result adapter publication, and recovery
of remote dispatches whose acknowledged response lacks a run identity. Existing
Git/evidence/release domain predicates are
not counted as connected application behavior until those adapters and their
failure fixtures exist. Missing retained Git objects block observed integration;
the coordinator never reconstructs a child contribution from a later staging tip.

Run the current deterministic suites from the workspace:

```sh
RUSTUP_TOOLCHAIN=stable CARGO_HOME=/private/tmp/kegari-v0-cargo-cache \
  cargo test -p kegari-v0 --test delivery --test execution --test git_delivery
```

No live mutation, production deployment or actual-agent qualification has been
performed by these fixtures.

## Prepared exact-validation workflow protocol

The configured no-change workflow must implement the union of configured CI check
names, each unique within that route. Evidence identifies this actual workflow ID;
it does not claim the ordinary PR workflow ran. The proof route and its reusable
workflow must be reviewed and pinned in repository configuration before use.

Dispatch receives the configured source and dispatch-operation inputs plus
`kegari_runtime_id`, `kegari_phase_operation` and `kegari_challenge`. The first
operation input is the provider mutation receipt ID; the phase operation identifies
the immutable admitted delivery operation. The trusted workflow must check out and
verify the requested source/tree before minting each job's source-bound OIDC proof.

The run uploads one private artifact named
`kegari-validation-<phase-operation>-<run-attempt>`. Its ZIP contains
`kegari-validation.json` with schema version 1, `runtime_id`, `phase_operation`,
`dispatch_request`, `challenge`, numeric `repository_id`, `run_id`, `run_attempt`,
`tested_tree`, an `outputs` map whose configured `observed_source_output` equals
the actually tested source SHA, and `proofs` entries containing `check_run_id` and
that job's OIDC `token`. Tokens must remain private and are never copied to Linear,
GitHub comments or general logs. Each audience binds runtime, phase operation,
challenge, repository and tested source, using `HostedProofExpectation::audience`.

`acquire_exact_validation` derives all expectations from the host challenge and
provider observations. It does not trust expected identities from token payloads.
All required jobs must be successful GitHub-hosted jobs from the current attempt.
The archive reader accepts only stored/deflated regular entries, at most 16 entries,
1 MiB per entry, 4 MiB total uncompressed data and a 4 MiB compressed archive.
It rejects traversal, links, encryption, duplicate names, corruption and mismatched
sizes without extracting files. Missing or expired proof is a blocker; the host
does not silently dispatch another run.

Canonical signed publications use `evidence::encode_signed_publication` and
`decode_signed_publication`. Recovery requires exact whole-comment framing,
canonical JSON encoding and the expected domain/key HMAC; quoted snippets,
concatenated records, extra/duplicate fields and edited payloads are rejected.

## Read-only hydration and isolated slice preparation

`observation` builds a current read-only context from a sealed reconciler gate.
`hydrate_child_result` can revalidate signed child results while the coordinator is
STOPPED, without a worker permit or remote write. It rereads review/check facts;
a later rerun or correction still invalidates the result. Coordination-only gates
cannot promote completion evidence. Changed child recovery also rereads current
staging protection while allowing its tip to have advanced after the recorded
integration.

`assemble_isolated_slice` is a separately admitted parent preparation phase. It
uses actual provider main commit/tree observations, authenticated retained staged
deltas and live merged PR identities, then runs the bounded Git sequence with a
fresh control gate at each subprocess start. The process handle is registered
before releasing the initial control gate. The signed intent and result bind the
selected contribution inventory and exact local effect receipt. Recovery checks
that record against current main and the actual candidate objects; editing both
Git metadata and an unsigned caller record cannot reassign contribution credit.
Repeated completed calls reread the same candidate; partial or unknown attempts
are retained without resetting or relaunching them.

The actual-Git fixture verifies an unchanged production candidate, preserved
operator staged/unstaged bytes and index, one completed preparation replay, and
rejection of a modified retained contribution mapping. This phase alone provides
no integrated, accepted or release-ready parent proof. Complete parent-vector
assembly, deployed-baseline observation and the remaining provider release path
are still in progress. Exact remote objects must currently be present in the
owned/local object source; network object acquisition is an open integration gap,
not an instruction to mutate or fetch into the operator checkout.
