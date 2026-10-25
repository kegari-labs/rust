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

## Completed-run and parent adapters

`delivery_child.rs` accepts a protected `VerifiedAuthorizedOutcome` together with
its actual retained `RunCompletion`. The operation, completed session/turn,
current child authority and confirmed local exit must match. Worker JSON remains
a product assertion: `ChildResultClaim` cannot inject passed CI, review or staging
fields. A changed child is sealed by owned, journaled Git commands only after its
writer has exited. `SealedChildSource` requires a new exact-head validation before
branch/PR publication. A no-change claim with dirty source is rejected before a
commit is created. A repeated source-seal attempt is never silently replayed;
`recover_child_source` authenticates its acknowledged receipt and rereads the
actual raw commit, tree, blob, mode and scoped patch bytes.

`advance_child` executes one explicit admitted action: local validation,
branch/PR publication, hosted or exact-revision validation, staging request and
observation, result publication, or tracker closure. Review is a separate
prepared assignment. Its full base/head/tree/delta identity is inserted into the
host context before the review worker starts. `publish_completed_review` requires
that exact context digest, a read-only Review role and the actual completed run
before it signs the review attestation. It does not accept a worker-selected
revision as proof of what was reviewed.

`delivery_parent.rs` constructs complete repository vectors from authenticated
isolated slices and `ProductionObserver` baselines. Recovery checks the actual
candidate Git objects, complete target inventory and current deployment record
identity. Integrated execution accepts only private production check
capabilities, verifies every configured command against the vector after staging,
and retains actual output bytes in bounded private chunks. Deterministic command
fixtures are rejected as production evidence.

Parent synthesis receives that full verified vector, all current required child
results and integrated artifacts as one exact host context. Publication rechecks
those gates, and read-only hydration repeats current provider/Git/artifact
observations. `refresh_delivery` returns a sealed snapshot with the original
FreshGate; the runtime must compare that authority again while replacing cached
readiness. Missing, stale or ambiguous publications yield absent readiness and
specific blockers. Tracker Done/Accepted state cannot produce a passed gate.

These adapters do not qualify native process-tree containment or provider access.
Visual parent readiness remains unavailable until actual authenticated upload,
association and playback verification issues its sealed video capability. The
runtime release decision backend and complete release-plan preparation are still
being integrated; these component APIs are not an assertion of application
completion. Stopped Git hydration must use the separately authenticated host-local
process scope; a fabricated worker lease is not permitted.

`RuntimeDelivery` composes current-provider hydration, protected manual release
intent records, and finite child/parent phase progression. Child validation and
review launch from opaque assignments preserving the exact original source and
context. Parent progression assembles every repository, validates the complete
candidate vector, and binds the coordination result to the integrated evidence.
The permanent capability maps retain original runs and admitted phases even if
an error-report write fails. Tests replace an owned private-directory inode to
exercise that failure path. These runtime paths still require broader integrated
verification; restart phase hydration, visual recording publication, release
backend composition, and authenticated remote Git object acquisition remain
unfinished. A missing remote object is a prerequisite error, never permission to
use an old operator checkout revision.

Before source sealing, configure the human committer's `user.name` and
`user.email` in each configured source repository's local Git configuration.
The host reads only those two local values. It will not infer identity from a
provider account or global Git configuration. Runtime commits preserve human
signoff and record the selected Codex model and exact instruction digest.

A mixed release may include `VerifyExisting` for an unchanged target already
serving its accepted exact source and artifact. This read-only verification
requires full current production validation and covers the target in the finite
plan. It cannot create a merge or deployment. A deployment-only successor whose
main and target still diverge needs separate existing-source recovery modeling;
`VerifyExisting` must not conceal that divergence.

Candidate prerequisites are distinct from final completeness. Every ordinary
child must be complete; the designated E2E child may have its reviewed, validated
and staged test contribution ready while its candidate execution remains pending.
Its final publication waits for the reobserved exact-vector integrated proof.
Parent review still requires all children complete. On recovery, a source-only
E2E child publication is withheld from completed dependency outcomes until a
current authenticated parent also binds its integrated vector and checks.
Standalone E2E recovery before parent publication remains unavailable.

Parent records are keyed by the complete authorized work identity. A later
correction preserves previous generation records and cannot overlap their live
or uncertain effects. A persisted worker launch ID alone is not an admission:
capacity/spacing waits can retry the same effect-free intent; a retained
preparation requires recovery. Admitted phases retain their original settings
while current permission and scope revocations still apply. Maintenance polls
only those phases and never starts the next worker. A known terminal validation
or review failure retains its evidence and retires only proven terminal ownership.
A new repair requires the opaque `UnfinishedChildWork` producer to bind an actual
failed required check or exact-head review finding, original implementation
admission, unchanged sealed source and current scope/profile. A later passing
check supersedes an earlier failure; any later relevant admission invalidates
the retained trigger. Unknown process/provider effects keep their original holds.

The child pipeline hands a genuine trigger to the original finite implementation
phase. Repair uses a fresh clone and session. A materially changed repair tree
can then issue `VerifiedSourceAdvanced` for the original validation/review phase;
its old attempt, turn and review limits remain in force. The complete native
context is retained, with the exact assigned task under
`evidence.delivery_assignment`. The deterministic integration fixture covers
actual owned Git sealing, protected completed-protocol records, assembled
contracts and Control admissions; it does not qualify a live model, native
repository execution or an actual runtime joined-process cache.

After that original budget reaches a real attempt, review or progress blocker,
an explicit operator retry can select the retained obligation. The normal
selector still excludes completed work without this private proof. The native
signed grant must name the complete prior admission set and remain current;
edited, deleted or competing grants cannot authorize a repair. Its first session
uses `RetriedUnfinishedWork`, a fresh assignment/session and the grant's distinct
finite budget. The original completed receipt and limits remain unchanged.
Later sessions use actual history inside that budget rather than reusing the
first-session capability. Superseded budgets cannot continue automatically.

The positive deterministic fixture sends the actual retry command through
DecisionManager, publishes and rereads the native fixture grant, and runs the
new bounded repair/source-validation admission path. It also rejects deleted
and edited grants. It does not manufacture native joined-process evidence.
Corrected phase generations, mixed validation/review grant histories and cold
reconstruction of these private triggers remain explicit recovery prerequisites.
