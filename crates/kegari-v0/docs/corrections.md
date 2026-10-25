# Native correction classification and routing

`CorrectionPublisher` accepts only `VerifiedCoordinationOutcome`, recovered from
the host-authenticated pending outcome, immutable worker admission, actual protected
instruction contract and completed-turn journal. This is completed-turn provenance,
not semantic truth or new mutation authority. The publisher rechecks current
configuration, owner, corrector, parent authorization, native pending revision set,
required-child scope and operation permit before every provider write.

The delivered versioned coordination asset defines the strict
`correction_classification` claim. Informational input publishes a classification
without child routes or a correction generation. Ambiguous input remains pending.
Corrective input must name existing scoped children, concrete instructions and
nonempty validation expectations. Verified explicit CLI corrections remain
corrective and retain their actual mapped human corrector separately from the
host's native posting account. No proposal grants new child/repository/path scope.

Each exact child route is signed in the `correction_route` domain and sent through
the serialized provider broker under its own admitted finite operation. The final
`correction_classification` publication links all exact native route IDs, body and
signature digests, source revisions, worker outcome/admission identity and scope.
It follows a per-parent signed chain. Every prior route is reread and checked
before the final write; final success requires a complete authoritative refresh.
Deterministic per-proposal IDs prevent repeated polls from reposting a route.

ObservationCache reconstructs clearance from the complete current native final
chain and route records. It requires exact acknowledged protected receipts and
actual native writer/target/body provenance. Editing or deleting a final or route
record blocks affected gates, including after restart. A changed source becomes
pending again; an edited known corrective source immediately invalidates prior
evidence. The protected reference store retains only native revision identities
and monotone corrective digest history. These references cannot grant clearance,
authorize scope, or substitute for the actual signed provider records.

Generation and classification-phase identities exclude local polling counters.
The coordination gate additionally binds the native pending revision set, so a
completed result cannot be reused after a timestamp edit or a newly arrived
comment. Ordinary worker draining retains its existing gate behavior. The private
historical gate envelope includes this new field; predeployment old envelopes
without it are rejected rather than migrated into authority.

Unknown provider responses are observed by exact immutable payload and native
identity, never resent automatically. `recover(completed)` observes deterministic existing route/final receipts while
STOPPED using the current control-owner check. It also retires only their exact
finite metadata admissions, after validating the original actor/owner/scope and
actual native effect. It returns no final publication when only a route prefix
exists; it never creates the missing final stage. This grants no worker or
provider-write admission. The result reports pending input and publication
provenance. It does not assert that steering succeeded or any child consumed a
correction.

Deterministic tests in `tests/support/correction_pipeline.rs` run as library tests
against the actual publisher, provider parsers, completed-claim reader, ControlHost
receipts, ObservationCache and private restart references. They use synthetic HTTP
and synthetic completed-turn records only; they do not qualify actual-agent
classification behavior or live Linear mutation. Required live evaluation and
human calibration remain separate.

```sh
RUSTUP_TOOLCHAIN=stable CARGO_HOME=/private/tmp/kegari-v0-cargo-cache \
  cargo test --lib corrections::tests
```

Active child steering retains the original instruction/tool/isolation contract.
`VerifiedCorrectionTransition` requires the old correction generation to be an
exact signed history checkpoint, a strict current-history extension, intact native
classification and route records, and unchanged authorization/scope/settings and
dependencies. Editing or deleting a routed source/record closes the gate. The
per-turn input includes current definitions and latest relevant native revisions;
an informational classification grants no new source change or inferred reversal.

Installation durably signs an immutable per-operation transition record before
exposing the effective epoch or waking the worker. The provider loop and evaluator
share delivery logic. Completion races leave the correction queued for a bounded
next turn. Watermarks are consumed only after the exact delivered turn completes
with its matching processed input report. Pending classification defers publication
without weakening effect gates. Schema-1 claims remain historical; schema-2 claims
require an exact original assembly, signed predecessor chain and ordered actual
turn delivery/processing journal entries. Native restart resumption and verified
unfinished-progress continuation still require additional host integration.
