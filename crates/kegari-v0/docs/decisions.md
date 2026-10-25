# Human decisions and command ownership

`DecisionManager` composes the authenticated control host, current typed config,
provider observation cache, bound provider connections, host attestation key,
`Housekeeping`, and mandatory `DecisionEvidence` implementation. It cannot create
workers. The host dispatcher owns each bounded background command independently
of its HTTP request. `execute` receives that actual Prepared parent receipt and
the deterministically derived effect ID; it verifies the original arguments,
runtime, principal, targets and payload digest before any provider write.

Individual provider stages have stable derived IDs and bind the original command,
effect, current configuration/control generation, canonical workspace/work and
provider body digest/writer. The final effect receipt records the concrete stage
results. A missing final receipt does not imply the stages were absent or safe to
repeat. Lost acknowledgements remain Sent and require authoritative reconciliation.
Exact completed replay returns the recorded effect without another provider call.

Corrections require both the current `correct` control role and the mapped Linear
user in the workspace corrector allowlist. The complete nonempty UTF-8 message
(up to 1 MiB) is published on the selected parent, followed by an exact signed
`explicit_correction` marker. The marker binds the message digest, canonical
parent, runtime/workspace, operator principal and identity, mapped Linear actor,
actual provider writer, command and publication operation. Plain marker text,
changed message bytes and bot/delegated/wrong-writer provenance fail verification.
`VerifiedExplicitCorrection` retains the actual comment ID/body digest. The
observation cache reconstructs correction intent from provider bytes, including
after restart, and preserves it as pending coordination while stopped or paused.
Ordinary acknowledged host comments are excluded only while their immutable
receipt body digest, target and actual native writer still match; an edited comment
must enter classification again.

Approval requires the supplied exact current parent token and hydrated
`VerifiedParent` capability. It cannot accept a Done label or an agent summary.
The manager publishes authenticated human acceptance, independently rereads its
exact comment and complete parent/child/correction observations, and verifies the
record before applying the configured final parent state. The closure mutation
repeats the current actor, permission, generation, evidence and publication checks
inside the serialized broker gate. Acceptance and closure work while STOPPED;
they do not admit execution or production release. A changed correction or missing
publication preserves the observed history but blocks further stale effects.

Retry selects one existing parent or child phase, checks its complete durable
admission history and resolved authority/dependency gates, refuses active or
uncertain work, and publishes the actor/reason plus a signed finite `RetryGrant`.
The grant binds its exact work generation, prior admission IDs/digests and configured
attempt/turn/review limits. Only a verified publication yields the opaque grant;
its new phase key uses that grant operation UUID. Parent retry does not implicitly
reset every child. Restart reconstruction must validate the signed provider grant
against its exact history before supplying it to run-budget admission.

Release/release-recovery receive private `AuthorizedDecision` capabilities after
current actor/control/provider checks. Their mandatory host adapter still performs
actual release-state/evidence/queue admission and calls `recheck` inside every
serialized effect gate. New releases require RUNNING. Release recovery never
supplies fabricated completion, chooses another candidate silently or bypasses
unknown provider outcomes. `AuthorizedOperationRecovery` similarly binds exact
runtime receipt and command ownership; the adapter must reconcile actual provider
or process facts. There are no default-success adapters.

Recovery intent and completion use closed local metadata operations. They can
record an authenticated recovery request while pending corrections or unknown
effects hold normal execution, including while STOPPED. Completion requires the
actual durable action receipt: a release transition or checked new resume
admission, or the exact Reconciled target of `operation reconcile`. An
authorization-only receipt, substituted result or still-unknown target cannot
stand in for completion. Unrelated holds remain unchanged, even when they cover
the entire runtime. These paths cannot transmit a provider action or grant work.
Only resume's separate ordinary RUNNING admission can create a new permit; it
binds the original plan and a consumed, reconciled predecessor with a new
monotone resume ordinal.

Cleanup inspection and application call the production `Housekeeping` implementation,
including its exact immutable plan and original parent-command ownership. Source
files cannot be removed by a worker or inferred tracker transition.

Provider observation refresh and preflight run outside the control mutex. Final
actor/work/evidence checks use the bounded fresh cached snapshot, and observation
publication takes the same serialization lock as effect launch. The broker records
Prepared before its owned asynchronous preflight, rechecks the synchronous gate,
records Sent before launching transport, and acknowledges after observing the
response. Caller cancellation cannot cancel that owned pipeline. Configuration,
provider and publication uncertainty is reported explicitly. No test here runs an
agent, accesses native Keychain, mutates Linear/GitHub or qualifies real release.

`tests/decisions.rs` exercises real private receipt storage and production provider
parsers with labeled synthetic HTTP facts. Its acceptance fixture creates an actual
isolated no-change Git candidate and passes synthetic checks/reviews/publication
through the production sealed evidence validators; those fixture observations are
not operational qualification. Run the bounded component checks with:

```sh
RUSTUP_TOOLCHAIN=stable CARGO_HOME=/private/tmp/kegari-v0-cargo-cache cargo test --test decisions --test transport --test providers
```

Approval, retry and correction publication is followed by exact current provider
comment validation: target, ID, unchanged signed bytes, one matching publication
and native writer must agree. This also applies when the tracker already shows the
accepted state. Cached acceptance additionally binds the exact current evidence
token, not merely its work generation. The final cache insertion is serialized
with observation commits and contains no network calls.

Before an explicit correction is transmitted, the shared observation cache fences
the parent and its active snapshots. This closes the interval between receiving
an acknowledged corrective publication and reconstructing its correction input.
Complete refresh preserves the fence until that exact signed native comment is
observed, or a durable receipt proves no transmission occurred. A missing, edited
or ambiguously duplicated publication cannot install retry/acceptance/correction
authority. Cache insertion callbacks must not reenter `ControlHost`.

No-release completion records use the same operation recovery command. Recovery
requires the original signed local intent, exact actual admission/effect lineage,
and one unchanged native publication by the originally selected writer. Current
operator authority and the configured provider endpoint/account/workspace are
rechecked before recording the observation. Missing or ambiguous publications
remain unknown; they do not authorize resending. If a restarted original admission
also needs recovery, its sole publication effect must be reconciled first, and no
active local or external owner may remain. Historical receipt recovery never
creates a current dependency capability by itself.
