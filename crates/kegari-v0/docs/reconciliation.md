# Provider reconciliation and inspection

`Reconciler::refresh` accepts the validated `Config`, already credential-bound
`WorkspaceClients`, loaded workflows and small retained references. It performs
no provider mutation. Each workspace gets either a complete in-memory projection
or an explicit failed projection that preserves previously known parent and
required-child identities. A failing workspace cannot supply admission facts.

The production path reads discovery, explicit retained IDs, full hierarchy,
comments, history, native blocking prerequisites and the configured GitHub
repository union. It follows the prerequisite closure with a finite issue bound.
It repeats the complete collection and rejects differences. Comments are always
read even when issue `updatedAt` is unchanged. This is a consistency check, not a
provider transaction: effects still require a last-moment provider refresh inside
the serialized host mutation gate, and an unseen concurrent edit cannot cancel an
already transmitted action.

`RuntimeProjection` and its issue bodies are transient memory. The API can pass
their JSON through `snapshot::Snapshots` for complete nested/chunked pagination.
They must not be written as recovery receipts or general logs. `ReconcileRefs`
contains authorization IDs/digests, correction revisions and monotonic observation
references, never a copy of task descriptions. Authoritative reconstruction on
restart requires rereading signed records from Linear and operation references.

## Authority and exact gates

`AuthorizationSnapshot` contains the canonical Linear parent, latest native
human label event, full required set, definition/write-scope digests, production
targets and pinned prerequisite generations. The host signs it with purpose
`parent_authorization`, publishes through the broker, rereads its native writer
and payload, and calls `VerifiedAuthorization::verify`. Merely constructing a
snapshot does not admit work. Initial authorization cannot predate known child
creation or description/hierarchy preparation events. Canonical overlap across
configured workspaces becomes an ownership blocker.

Declared, enumerated and previously authorized children remain in the required
set. Missing or reparented children stay explicit. For an inaccessible deletion,
the first observed disappearance is retained; a later permitted label event is
needed before scope removal can be prepared and published. A cancelled child
does not disappear. Repository read/write permissions are checked separately.

Permitted native comments enter `Corrections`; pending or ambiguous classification
blocks normal gates. Coordination may inspect pending input under its own gate.
Host evidence/routing IDs are excluded; explicit CLI corrections must instead be
reconstructed from authenticated host markers and fed through correction intake.
Callers must never add CLI correction IDs to the generic evidence exclusion set.

An interpreted, published in-scope definition correction can be verified with
`VerifiedDefinitionCorrection`. It binds the old authorization, exact before/after
definition digests, correction watermark/generation and current native source
comment digests. Child IDs, write scopes and production targets must remain the
same. This permits actual corrections without another human label cycle; it
cannot widen scope. The resulting current authorization record still must be
published before further admission.

`ParentProjection::fresh_gate` creates a non-deserializable `FreshGate` only after
authority, publication, corrections, relevant child scope/profile, dependency and
freshness checks. Dependency outcomes come from verified child evidence or fully
observed release/already-production evidence, never tracker state. The gate binds
the exact work, configuration/workflow, source observation, profile, write scope
and correction watermark. `bind_permit` additionally matches the canonical work
identity (independent of correction generation) and host-defined broker scope. `compare_current` is intended for the host's
serialized final callback. Ordinary selection exclusion can drain an existing
operation only with all other effective policy and source bindings unchanged.
It cannot create a new operation or bypass revoked repository/actor access.

Independent PR review has a separate `IndependentReviewGate`; it contains no
fabricated Linear authority. Complete selection, repository access, exact PR
ID/number/head/base, configured profile and workflow are bound into its digest.
The worker contract restricts it to read/review feedback capabilities.

## Overview composition

`RuntimeProjection::overview` always returns `ready_for_release`,
`ready_for_review`, `blocked` and `running`, followed by separate history,
independent jobs, runtime health and next action. Every child has a canonical
`workspace UUID/parent UUID` reference and required children remain in counts.
It performs no mutation or dispatch. Parent reference resolution reports all
ambiguous candidates before authorization or receipt creation.

Ready states require opaque verified evidence. `ParentDelivery` accepts
`VerifiedChild`, `VerifiedParent`, `VerifiedAcceptance` and
`VerifiedReleaseEligibility`; a tracker Done state cannot construct them.
Accepted-but-ineligible parents remain blocked. Manual-ready release is distinct
from automatic queued release; current complete evidence can remain available for
human review while execution is paused. Active draining work remains Running with
its current blockers visible. Next action prefers manual release, human review,
then an actionable prerequisite root for an accepted blocked parent; an unknown
dependency root is reported as unknown.

Authenticated release inspection is separate from the current scheduling
projection. Overview adds typed `release_history` rows and summarizes exact
successful completions in history. Show includes attempts, completion/hold
provenance and scoped diagnostics even when the parent or workspace is absent
from current discovery. A consumed ancestor hold is not a successful release of
that ancestor's generation. Display rows never mint dependency or readiness
capabilities, and a later correction remains in its current active category.
History and release details use both runtime workspace UUID and full native issue
identity, so two workspace mappings cannot mix presentation records.

## Integration and validation limits

The host still owns semantic preparation review (whether the prose actually states
adequate acceptance criteria), correction classification/routing, evidence-record
reconstruction, dependency-generation selection from authoritative prior records,
operation claims and actual scheduling. This module validates structure and
authority; it does not claim that arbitrary prose is semantically sufficient.
The delivery pipeline separately verifies local tests, hosted proof, review,
staging, isolated evidence, human acceptance, production gates and policy.

The current deterministic reconciliation suite runs production provider parsing
against injected HTTP fixtures, including unstable/partial reads, missing children,
comment changes without issue timestamps, native dependency cycles, overlap,
filter drain versus revocation, pinned independent review and signed in-scope
corrections. Actual providers, Tailscale and agents are not invoked. Ready-state
proof construction uses the independent evidence/release predicates; a combined
real-provider evidence-to-overview qualification remains necessary.

Active operations retain an opaque `observations::ActiveSnapshot` handle. The
registry keeps weak references, so ending the operation also retires its snapshot.
Every poll re-reads current provider authority using the admitted workflow and
execution settings, current account/path bindings and current permission lists.
Invalid replacement workflow content blocks new work while the separately
validated old snapshot may finish. `active_gate` and `active_independent_gate`
return current capabilities only for that retained snapshot; they still reject
corrections, changed work, lost access, missing complete reads and permission
fences. Their observation commits share the final effect gate's serialization.

The deterministic suite exercises valid/invalid workflow replacement, profile and
capacity changes, selector drain, current actor/path/repository permission loss,
unclassified corrections, and retirement of the retained handle. These fixtures
use actual production parsing/cache paths and synthetic provider responses; they
are not live provider or agent qualifications.
