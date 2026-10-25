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
