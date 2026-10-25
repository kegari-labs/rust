# Production preparation and release adapter

`delivery_release.rs` is the production adapter behind `DeliveryCoordinator`.
Preparation requires a live bounded delivery admission, the complete verified
parent candidate vector, an actual isolated Git candidate, and its published
child evidence. It does not require human acceptance and cannot merge or deploy.

The adapter exports the candidate's actual Git objects through the registered
process journal. Blob bytes, modes/deletions, base tree, commit parent, message,
author, committer and their dates are preserved. Every returned provider object
SHA must match the actual local object. It creates an exclusive owned branch and
PR to `main`; collisions are blockers, never force-updates. A signed derived
review accounts for every selected delta using the current original child
review publications. Its commit status explicitly describes carried reviews,
not another model review.

Each configured production suite must produce a private artifact named
`kegari-production-<phase-operation>-<workflow-id>-<attempt>` containing
`kegari-validation.json`. Its strict schema is the existing `HostedSourceArtifact`:
`schema_version`, `runtime_id`, `phase_operation`, `dispatch_request`, `challenge`,
`repository_id`, `run_id`, `run_attempt`, `tested_tree`, `outputs` and `proofs`.
The required output is `outputs.source_sha`; each proof contains `check_run_id`
and a signed GitHub OIDC `token`. The host independently constructs all expected
claims from its recorded nonce and actual complete workflow/job/check reads.
Artifacts are bounded, their archive hashes checked, and tokens never retained
in public receipts. Later recovery uses authenticated retained proofs and fresh
native run/check observations; an old bearer token is not reaccepted.

For `workflow_dispatch` suites the reviewed workflow consumes fixed `source_sha`
and `operation_id` inputs, plus `kegari_runtime_id`, `kegari_phase_operation` and
`kegari_challenge`. `dispatch_request` is the actual provider mutation request ID
passed in `operation_id`. The pinned caller and reusable workflow must actually
check out/test that source before requesting its source-bound audience. For a
production PR/push trigger the signed PR body supplies the phase nonce and the
artifact's `dispatch_request` is the phase operation ID; the trusted workflow
must obtain that correlation before emitting its proof. Mutable runner labels
are not evidence of hosted execution.

Changed-child validation uses the same schema with artifact name
`kegari-child-<phase-operation>-<workflow-id>-<attempt>`. Automatic acquisition
requires configured pinned `workflow_dispatch` routes with the fixed inputs
above. Every configured suite/check is required. A missing artifact, queued run,
failed/skipped job, changed run attempt, wrong source, incomplete pagination or
invalid signature prevents completion. Repeated polling reuses the same dispatch
receipt. Child and deployment dispatches own the original attempt only; a human
rerun is not adopted or cancelled as that operation.

The runtime owns release queue serialization, signed `ReleaseAttempt`
persistence and the actual `Dispatch` capability produced by `mark_sent`.
`dispatch_release_step` additionally requires the exact accepted parent/carry,
prepared production candidate, verified production tests, and a mandatory
cache-only final callback checking current acceptance, ownership, resources and
holds. The adapter writes the exact command intent before provider transmission.
Existing intents are observed, never blindly resent. Merge completion requires
the actual squash's single parent and exact tested tree. An acknowledgement
alone is pending. Deployment uses only the configured target/workflow and the
`ProductionObserver` challenge, source/artifact/environment contract, complete
job proof and actual current deployment/status observations.

`observe_release_step` polls an active operation and can verify an artifact that
became available after dispatch. `recover_release_step` is read-only and works
with a current observation while stopped. Its opaque result distinguishes
pending, actual success and observed terminal failure. The backend must persist
these facts and reconcile the original receipt before releasing its hold; a
successful read does not erase a still-running network task. Missing/ambiguous
remote identity or unavailable authenticated deployment proof stays held.

The adapter's provider merge observation is not a
`VerifiedProductionIntegration`. Later contribution recovery still requires
retaining and verifying the actual remote squash Git object in an owned cache;
copying provider metadata into a Git proof is not sufficient. A crash before a
preparation result is sealed retains the exact per-action receipts, branch and
PR effects; uncertain effects require explicit observation/reconciliation, not
allocation of replacement mutation IDs. Initial historical deployments without
an authenticated source/artifact proof remain preparation blockers as described
in `deployment.md`.

Validation here uses owned disposable Git repositories and deterministic HTTP
fixtures. No live provider mutation, production workflow, deployment or hosted
OIDC issuance has been executed by this build conversation. The reviewed target
workflow contract and real provider/runner behavior require supervised
qualification before activation.

Mixed releases can include `VerifyExisting` steps for an unchanged repository
whose configured targets already serve the exact accepted baseline. Those steps
require `ReleaseStepEvidence::Existing` with sealed production validation and a
complete `VerifiedProductionBaseline`; they reobserve the actual target vector
and artifact. They do not invent a merge, PR or deployment. Changed steps use
`ReleaseStepEvidence::Changed` and retain all normal production preparation gates.

The registered `runtime_delivery_release` backend exposes `advance_releases`,
`maintain_releases`, `reconcile_releases` and `release_attempts`. It prepares each
repository from the sealed parent source inventory, admits one finite plan
permit under the actual serialized control snapshot, and retains both that
permit and its admitted configuration/workflow snapshot. Maintenance drains only
existing preparations and plans. It cannot start another repository preparation
or release while paused. Current permission fences, mapped manual actor,
acceptor allowlist and exact native parent/acceptance publications remain gates.
A newer same-generation parent publication invalidates the prior selection.

Candidate admissions are ordered across workspaces by verified acceptance time,
then canonical parent identity. Every candidate still passes current dependency,
resource, spacing and authority checks; an ineligible candidate does not block a
later candidate with disjoint resources. Preparation records are indexed by the
exact parent evidence token, preserving old unfinished records while allowing a
corrected accepted generation to prepare separately.

Signed attempt records keep observed terminal effects separate from receipt
reconciliation and local retirement. Resource holds survive failure to reconcile
the exact provider receipt, restart, and failure to persist retirement. Only
reconciled effects plus the actual admission's observed local exit can release
those holds. No restart reconstructs a live dispatch permit from JSON.

Step payloads bind an explicit attempt ordinal and the exact ordinals of their
prerequisites. A completed failure/cancellation advances the ordinal only through
an authenticated explicit resume; old effect records and request IDs remain
immutable. A verification step names its actual deployment attempt, and a missing
new attempt never falls back to an older effect. The immutable plan ID is distinct
from the actual resumed admission used for ownership, cancellation and retirement.

The recovery backend now provides concrete `abandon_release`, `resolve_release`,
`resume_release` and `reconcile_operation` methods. Abandon retires only known
unstarted work and preserves unknown effects. Resolve uses the complete sealed
repository/target inventory and requires a coherent known terminal baseline;
it cannot declare delivery successful. Explicit resume obtains a new checked
control admission from the actual prior terminal receipts and authenticated
operator command, then rechecks the same accepted token, plan and production
proofs. A held idle finite execution can retire without releasing its resource
hold. The ordinary queue still rejects that hold; only checked resume evaluation
may exclude its own exact predecessor hold, never another attempt's.

`recover_owned_release_step` reads signed original effect and actual admission
records independently of current product readiness. Correction, exclusion or
STOPPED state therefore cannot hide the outcome of an earlier action. Its result
grants no write or readiness authority. Completed preflight rejection can retain
a signed no-dispatch observation; remote absence alone is never such evidence.

The concrete human release-recovery trait now delegates to these methods and
`supersede_release`. Supersession requires a known locally retired predecessor,
a freshly accepted corrected generation, complete current baseline observations,
and prepared production validation. It stores a signed exact successor intent
through the authenticated local transition. The predecessor hold is assigned to
that plan and survives until actual complete successor delivery. A separate
RUNNING admission rechecks current evidence, configuration, recover authority and
actual acknowledged transition; the intent itself grants no permit.

Same-result resume can now use a separate historical reader when ordinary
readiness is absent after its own merge. It rechecks original signed parent/vector,
actual local Git objects and integrated execution, current child/native acceptance
publications, and complete main/target changes accounted for by original owned
effects. Production recovery retains the original signed repository-policy
observation and independently verifies current policy at the actual squash; it
never substitutes that current base into the historical proof. Normal production
recovery still rejects a merged PR. The actual Git/provider adapter fixture covers
successful historical production readback and a later unrelated main movement.
The full runtime historical-parent continuation is compile-checked and still
needs its own positive end-to-end fixture. Previous-generation production
integration recovery and visual historical asset readback remain prerequisites.

An accepted outcome whose complete candidate vector is already production now
uses a separate no-release completion path. It reobserves every selected source
and configured target, verifies the exact native integrated-result and acceptance
publications, then admits one finite metadata publication. The signed record is
not dependency evidence until the exact selected writer's native comment and its
actual host effect/admission receipts are read back. The record consumes no release
spacing and sends no production mutation. The owned publication task retires even
if its caller is cancelled; unknown provider effects retain their holds.

Known-not-sent publication failures share the durable metadata attempt planner,
including the original attempt cap, bounded backoff and provider reset bounds.
Unknown or acknowledged effects are never replayed. Explicit operation recovery
can observe the exact old publication while STOPPED, with current actor/account/
endpoint checks; this records the historical effect and does not renew acceptance
or the current work generation. Current dependency proofs are rebuilt from native
readback. Replacing acceptance removes the old no-release proof, dependency credit
and link until revalidation. An offline runtime fixture exercises publication,
retirement, STOPPED readback and acceptance invalidation with actual owned Git and
local provider fixtures. This is deterministic evidence, not live qualification.

Corrected parent preparation can now retain a known predecessor's exact source
and target baseline through the sealed capability described in `deployment.md`.
The original parent comment must have its exact admission receipt, and acceptance
must have its original operation and control actor; copied signed comments cannot
substitute those effects. Native missing, edited or duplicate selected records
fail. Historical carried-acceptance releases remain an explicit prerequisite.

The current assembly imports both the accepted candidate and observed squash
objects under its actual current permit. It verifies the original signed slice,
Git bytes and accepted contribution provenance before carrying those unchanged
contributions into the new candidate. STOPPED hydration can only read already
retained authenticated objects. An absent import cannot be replaced by provider
JSON or an invented integration proof.

`DeployExisting` and `VerifyDeployed` explicitly deploy and verify an accepted
source already on main. They require a declared known-predecessor baseline,
actual prior contribution integration, complete current production validation,
and fresh human acceptance. Targets already serving that source use
`VerifyExisting`; no replacement merge is created. The configured dependency graph
and complete target inventory still apply. Offline coverage includes real Git
integration proofs and a mixed A-already-delivered/B-needs-deployment domain run,
original native publication/receipt readback, and actual local-retirement checks.
A positive complete runtime supersession fixture, full historical-parent runtime
continuation, chained prior integrations, carried-acceptance history and visual
historical assets still need dedicated implementation/qualification work.
