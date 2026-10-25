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
intent records, and finite child/parent/release phase progression. Child validation and
review launch from opaque assignments preserving the exact original source and
context. Parent progression assembles every repository, validates the complete
candidate vector, and binds the coordination result to the integrated evidence.
The driver retains one progression task while short supervisor polls continue;
an interrupted poll cannot discard the phase's join handle. A separate owned
observation task refreshes actual provider facts during long phases, with one
read in flight and a bounded timeout. Refresh failure blocks both new and retained
gates without changing earlier observation timestamps. Pending progression and
observation joins prevent a claim of local clearance. Pause/stop drain existing
finite work; force-stop still fences before cancellation. New work rechecks current
observations between candidates, while active phases retain their admitted settings.
Retained parent contracts use a distinct borrowed input from the actual live
phase and its retained observation registry. Current permission observations
keep their own configuration revision; they are not relabeled as the admitted
configuration. The phase keeps its original workflow and repository contracts,
and exact permit, owner, scope and authority checks still apply. Ordinary parent
contract construction continues to require an exact current revision.
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

A known failed child review can consume a current operator retry grant while
preserving its original exact source and sealed review assignment. The private
`ReviewRetryWork` binds the actual failed completion, terminal receipt, complete
review admission inventory, account/profile and unambiguous native grant chain.
The new review gets its own bounded admission and fresh session. Old receipts,
limits and pipeline review identities remain immutable history. Unknown exits,
pending provider turns, grant deletion/edits/forks, source movement and unrelated
new review admissions block this path. Preparation and final model admission
recheck the grant; the result's publication checks its exact new completed
admission and current grant again inside the provider mutation gate.

Another fully observed transient terminal failed review within that grant mints a new private
per-attempt proof from its actual completion and complete current receipt
inventory. It keeps the original grant, assignment and source, advances the same
budget's contiguous ordinal, and preserves the minimum captured attempt/turn/
review bounds and failure backoff. The consumed proof cannot admit a second run.
Only supported provider terminal classifications qualify automatically; authentication,
unknown and other persistent failures require explicit resolution. Historical
assignment/contract identity is compared independently of observation age, while
every current admission and publication still requires fresh native authority.
A clean result must match that proof's exact next completed admission before
publication. A granted Findings result can now produce a repair within the original
implementation budget. Its opaque completed-review lineage preserves the native
grant and complete review inventory. A materially changed repair tree can then
use the next ordinal of the same granted review budget; source advancement does
not renew either budget. Admission and final publication reread the current grant,
and ordinary mixed-grant continuation remains rejected.

The new fixture proves actual Findings, scoped repair admission and changed Git
source sealing, then asserts the genuine missing-qualified-local-check blocker.
Its downstream changed-PR assignment is an explicitly test-only upstream fact;
that separate component exercises granted new-head admission, clean result and
provider publication. It does not establish the production local-validation/PR
path or populate a native joined-run cache. Validation retry lineage,
cross-correction and cold reconstruction remain separate prerequisites.

Child review rounds now have explicit authenticated ancestry in the complete
review phase, including its optional operator grant. The first review allocates
round one; a supported transient retry keeps that round while consuming another
session attempt. Actual Findings permits its scoped repair in the same round,
including the last allowed round, while preserving the original implementation
attempt and turn limits. A completed changed repair can allocate the next
confirming review only when the review budget has room. Further Findings retains
that exact new round and its own source instead of resetting the counter.

A repeated repair within one Findings/fix pair requires an actual failed required
check on the latest completed repaired source. Its continuation retains the
original Findings, round identity and implementation budget, and binds the new
validation receipt, complete failure inventory and current source. Merely missing
a check does not create this capability. Validation itself allocates no review
round. New review grants have distinct namespaces and cannot renew implementation
limits or overwrite previous admission records.

Preparation records a proposal; only the actual model admission allocates the
round or consumes a session. Both gates verify current source, exact native grant,
actual joined completion handles and the complete relevant receipt inventory.
The model admission also binds the actual closed preparation's whole digest.
A raced or stale proposal cannot allocate twice. Legacy unmarked history stays
conservative: stored counters or serialized round references cannot reconstruct
opaque execution authority or silently restart at round one.

The [round-core validation report](../validation/LOCAL-REVIEW-ROUND-CORE.md)
distinguishes implementation from executed checks. Component fixtures use real
in-process RPC notifications, Control receipts and owned Git objects, with
explicitly synthetic upstream provider/check observations where stated. They do
not populate a native joined-run cache or establish native confinement/live-model
qualification. Paired transient-session recovery and clean retried-validation
handoff require their separate typed producers and consumers.

### Chained retained integrations

Corrected assembly can recover an earlier merged contribution through a finite
chain of known, locally retired release attempts. Every link must retain its
signed original accepted publications, exact actual admission, acknowledged
supersede authorization and command receipts, and matching successor intent.
The chain is capped at 64 attempts and rejects cycles, omitted contributions,
changed receipts, uncertain effects and mismatched repository/target inventories.
The importer acquires the original candidate, squash and current main objects;
actual Git ancestry and patch identity establish that the original contribution
is retained once. Historical proof does not populate generic current readiness.

For admission, an opaque verified lineage can project ancestor hold assignments
onto one exact successor plan in a temporary queue view. The durable holds and
receipts are unchanged; unrelated holds, resource owners, dependencies and spacing
still block. Terminal completion uses a separate sealed proof: every required
step of the final descendant must have succeeded, its actual admission must be
retired, and the finite signed supersede chain and owned effects must still match.
The first observation also requires the complete current production inventory to
be coherent. Provider reads finish before the serialized local commit.

A small signed completion reference binds every actual resumed admission and
completed recovery command/authorization, every prior step effect or no-dispatch
record, the original publication receipts and parent vector, and the immutable
target proofs/challenges. The mutable current-target pointer is excluded. Only after persistence can queue reconstruction
remove those exact ancestor holds. Ancestor failure outcomes remain unchanged;
only the successful descendant supplies release-queue completion and spacing.
Ordinary successful attempts use the same seal with a one-attempt chain and no
ancestor hold removal. Schema-2 completion records retain the original configured
Linear/GitHub accounts and endpoints, native workspace and every baseline
repository. Legacy seals can still settle their original queue history, but their
missing provider binding cannot be inferred for generic dependency credit.

The runtime recovers these opaque completion capabilities independently of current
delivery readiness. Every dependency use rechecks exact immutable references,
current provider/configuration bindings, complete fresh observations, native
prerequisite visibility and read access to all original repositories, including
unchanged repositories without release steps. The dependent's authenticated
generation pin selects the outcome. A later correction or production movement
does not rewrite an earlier completion; an earlier completion cannot satisfy a
pin requiring the correction. A missing reference invalidates its exact completion
without erasing other verified completions. Unsafe storage or unauthenticated
envelopes fail the inventory.

Deployment-free and authenticated owned-target `no-release-required` outcomes use
a separate signed immutable completion. Initial sealing consumes the current parent/acceptance
publications, complete candidate vector, exact retired metadata admission and
publication ancestry, and fresh opaque production baselines for every repository.
Recovery preserves the original outcome digest, generation and time. It validates
all retained references without requiring that generation or production revision
to remain current. Each dependency use still requires the same fresh provider and
repository access checks as released history. Inserting this historical success
never clears current owners or holds and never consumes release spacing.

For deployed targets, the actual ordinary production observer retains each owned
deployment proof. Completion seals bind its immutable proof and challenge,
available immutable locators, exact closed original/resumed ownership ancestry,
and dispatch receipt. Every configured target keeps its own artifact/source map;
a common artifact is not required. Historical reads validate these original local
references without consulting the mutable current-target index or requiring the
old target to remain current. Missing any required target reference denies the
whole completion. A target owner must have actually exited with reconciled effects
before its observation can be retained as closed provenance.

Known seals retain their authenticated identity and complete resource scope when
an immutable reference disappears. Their failed outcome cannot supply dependency
credit, clear a hold, or fall back to a mutable current observation. The queue
quarantines overlapping resources and dependencies pinned to that exact outcome;
disjoint work remains eligible under its normal gates. Valid earlier history
remains usable when a later completion on the same resources fails. Only a valid
completion may consume its exact ancestor IDs; another quarantine's protected
operations cannot be cleared by shared resource identity.

Quarantines retain their real protected envelope readers. Every queue decision
checks those readers before filtering by scope: a present malformed or
unauthenticated outer seal, unsafe storage, failed enumeration or unavailable
unknown scope blocks the whole queue until a complete successful inventory.
Unrelated rebuilds cannot erase that fatal status. A cold ordinary reader requires
all signed scope-bearing attempts; a known opaque anchor may preserve that scope
in the same runtime after references disappear. Runtime anchors also survive a
new delivery facade, preserve the original digest and prevent reminting a removed
known seal. First authenticated reads and initial seal writes remember identity inside the
same Control boundary used by completion commits. Each authenticated prefix is
remembered even if a later inventory entry fails. Complete queue/dependency sync
also holds that boundary through installation, so an older scan cannot overwrite
a newer failure. Shared failure status closes fresh facades, and newly learned
no-release scopes are checked before a later parent can record current readiness.
Ordinary and no-release inventory errors remain distinct; repairing one does not
clear an independent failure in the other. These anchors are protective facts, never successful
outcomes or execution permits.

Inspection displays authenticated identity and its exact diagnostic without a
completion marker. This display grants no authority.
Native parent/acceptance publication effects and the signed vector are required;
later approval-command completion and Linear closure are separate from acceptance.
An externally deployed target without an authenticated owned proof still lacks a
supported production-observation producer. The ordinary baseline reader reports
that prerequisite; native source/status JSON alone cannot prove deployed artifact
identity. No owned deployment dispatch or immutable baseline provenance is fabricated.
The same-parent predecessor selector excludes only ancestors covered by a valid
completion. `release_history()` separates historical failures from `active_hold`
and names `completed_by`; recovery cannot rewrite the completed terminal or its
consumed ancestor records. The control inspection facade includes these rows and
per-chain `completion_error` values.

Later production changes do not invalidate an already observed completion.
Restart recovery checks immutable local references without requiring the
descendant to remain the current deployed version. Missing, corrupt or changed
inner references deny their authenticated scope while preserving unrelated holds
and valid historical facts. Initial completion observations are checked
independently, so one observation failure does not stop other chains from being
checked. Only a complete new inventory clears an installed quarantine; merely
inserting another success cannot do so.

Normal and inspection refresh preserve each failed parent's diagnostic and skip
new observation admissions for that known blocker. Other parents still require
their original fresh-gate checks, with queue resource/dependency isolation enforced
separately. Historical diagnostic rows do not grant queue, recovery or baseline
authority. Errors on resolved records alone do not invalidate current readiness.
Unsafe storage, unauthenticated attempts and unknown completion scope fail
explicitly rather than being treated as an empty history.

The current ordinary completion/dependency/inspection increment is tracked in
[its validation report](../validation/LOCAL-RELEASED-INSPECTION.md). Earlier local
deterministic validation of terminal chain completion is recorded in
[the terminal/session report](../validation/LOCAL-TERMINAL-GRANT-SESSION.md). The actual
`chained_runtime_retains_merged_contribution_through_deployment_only_predecessor`
fixture includes cached/cold inspection during an exact target-proof deletion,
owner-reference diagnostics, and strict queue/restore refusal. Its upstream
accepted evidence remains explicitly synthetic.

Historical successful target actions now use a separate immutable proof selected
by the original challenge/dispatch or exact verification digest. The terminal
consumer requires the exact signed step and effect, locally retired admission,
reconciled dispatch and reread original deployment/status/run-attempt identities.
Replacing the current target does not rewrite that history; the final complete
current baseline is still checked separately. This path cannot reconcile unknown
effects or populate current readiness. Carried-acceptance release history remains
incomplete. The historical-target
recovery boundary passes the local component/full-suite checks recorded in
[the recovery report](../validation/LOCAL-NATIVE-RECOVERY-HISTORY.md).
Deterministic historical accepted-input fixtures are synthetic upstream evidence;
actual journal, provider wire and Git recovery do not replace genuine integrated
checks, live deployment qualification or human acceptance review.
