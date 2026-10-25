# Provider adapters and hosted-job proof contract

The production HTTPS clients and serialized mutation broker live in `src/providers.rs`.
The deterministic wire fixtures in `tests/providers.rs` exercise the same parsing,
pagination, identity, preflight, transmission, receipt and proof paths. They do not
establish live account permissions, remote sandbox behavior, playback, workflow
compatibility or end-to-end coordinator integration.

## Connection and collection guarantees

`HttpPolicy` binds one explicit HTTPS endpoint and canonical account ID. Linear
also binds a workspace UUID. `Credential` cannot be serialized and its Debug
representation is redacted. The reqwest client disables redirects and environmental
proxies, sets request/connect deadlines, and caps each response at a configurable
maximum of 8 MiB. It streams response chunks into that bound. Request payloads and
collection sizes also have finite bounds. Errors retain status, stable reason and
rate metadata, without copying provider response bodies or credential-bearing URLs.

The clients verify `viewer`/`organization` or `/user` before collection reads and
mutation preflight. Repository locators are always checked against their immutable
numeric repository and owner IDs. `resolve_repository` can resolve an explicitly
configured source locator; names alone never authorize a repository. Renames and
transfers fail identity checks instead of following redirects. GitHub requests pin
API version `2026-03-10`; an older Enterprise endpoint requires separate compatibility
qualification and fails closed when its API differs.

Linear requests use a personal API key in `Authorization` without a Bearer prefix.
GitHub uses Bearer authorization. `last_rate_limit()` exposes successful/error
response metadata. Linear reset timestamps use epoch milliseconds; GitHub reset
uses epoch seconds. Linear request, endpoint and complexity limits are combined
conservatively. Retry-After is retained. The adapter never autonomously retries a
mutation. The scheduler still owns shared-account pacing and backoff.

Linear collections traverse every Relay cursor with loop, duplicate-ID, empty-page
and size checks. Any GraphQL errors discard the entire read, including partial data
and earlier pages. HTTP 400 `RATELIMITED` and HTTP 200 GraphQL rate errors receive the
same stable classification. ID refresh returns an explicit partition of requested
IDs into observed issues and missing IDs, or an error; empty refresh sends no request.
Parent discovery implements team AND project AND (creator OR assignee). Children
are independently enumerated in full; their inclusion does not reapply root selection.
Labels are separately paginated and checked against the issue's label-ID set.
Optional malformed timestamps/priority/description become unavailable, while required
identity failures block the read. Native comment, delegation/bot and grouped-history
actor fields are preserved for the authority verifier; a comment body is not identity.

GitHub lists follow the documented Link continuation with unchanged origin, resource,
filters and sequential page. A numeric `/repositories/{id}/...` continuation is allowed
only for the same already-verified repository. Duplicate IDs, changing total counts,
last-page errors and incomplete totals fail the whole result. Organization selection
is unioned with explicit repository selection. Organization names must resolve from
verified accessible repositories or `/user/orgs`; an unresolved configured organization
blocks discovery instead of silently returning an incomplete union.

The GitHub client reads repositories, PR heads/bases/actors, checks, statuses, reviews,
comments, workflows, run attempts/jobs, refs, commit/tree provenance, protection/rules,
deployments/statuses and run artifacts. Check/review/status payloads preserve provider
fields inside repository-bound records for higher-level gate validation. `skipped` or
`neutral` is not converted to success. Job runner labels/names never establish hosted
execution: jobs remain `RunnerEnvironment::Unknown` until independent proof validation.

Artifact archive downloads use the documented one-time 302 from the authenticated
GitHub archive endpoint. Only that returned HTTPS URL is fetched, with no account
credential and no further redirect. Run/repository/source metadata, body bound and
GitHub's archive digest are checked. Signed download URLs and JWT bytes do not belong
in general logs or receipts. Archive extraction and artifact-to-required-suite selection
are still the host's responsibility; never extract arbitrary paths from a ZIP.

## Mutations and uncertainty

`ProviderMutation` is a closed command enum, without arbitrary REST/GraphQL, account
management, repository settings, forced ref changes, formal review approvals or reruns.
The only mutation entry point is `ProviderBroker::mutate`. It replaces caller-supplied
kind/target/payload with the actual command binding, including the configured account,
then enters `ControlHost::mutate` in a blocking dispatcher. Last-moment host gates and
provider identity/revision preflight run inside the serialized control gate. Transmission
starts only after the host persists its sent boundary. A timeout, malformed write result,
lost acknowledgement or contradictory identity remains uncertain and holds the target.
Identical request replay returns the durable receipt; changed command/account conflicts.

Supported writes are Linear comments and configured state transitions, private MP4
upload-grant creation and signed upload PUT; GitHub comments, the fixed
`kegari/codex-review` status, PR creation, expected-head squash merge, exact-workflow
preflight plus dispatch, owned-run cancellation, Git blobs/trees/commits and non-forced
owned refs. The host's `fresh_gate` must enforce parent authority, current corrections,
role, repository/path scope, work generation, test/review gates and appropriate operator
roles. Provider preflight does not replace those product-policy checks. These Rust APIs
are host code, not tools or credentials exposed to managed agents.

An upload is three independently recoverable effects: request a private grant, PUT
exact digest-bound bytes, then associate its URL in the parent comment. Grants remain
in host memory, not receipts; restart does not recreate an uncertain grant. The adapter
accepts bounded MP4 uploads up to 256 MiB, subject to the actual provider rejecting a
smaller limit. Receipt output explicitly keeps `associated: false`; remote association,
authenticated accessibility and usable playback must be observed separately. Local media
must be retained until those checks and recovery requirements are satisfied.

GitHub dispatch accepts a branch or tag name, not an arbitrary commit SHA. Preflight
verifies its current direct commit against the pinned workflow revision and verifies
workflow ID/path/active state. Reserved source and operation inputs cannot be overridden.
The actual run ID is retained when the API returns it. A legacy empty acknowledgement
remains correlation-required. Reference movement between preflight and dispatch is a
real race: the later signed workflow revision and actual-source evidence must match.
Neither dispatch acknowledgement nor the requested source input proves what ran.

Cancellation requires the durable dispatched run association and current run attempt,
workflow, source, configured actor and triggering actor. GitHub's cancel endpoint is
run-level, without an atomic attempt condition. A rerun racing the final check cannot
be ruled out by the API; cancellation remains requested until observed. The same-account
ownership association cannot be replaced by a guessed run ID. Merge acknowledgements
likewise retain `provenance_verified: false`; actual commit/parent/tree observation is
required before integration is accepted.

## Signed GitHub-hosted job evidence

GitHub's OIDC discovery and current documentation include `runner_environment` and
`check_run_id`, as well as `repository_id`, owner ID, actor ID, run ID, run attempt,
`sha`, caller `workflow_sha`, and reusable `job_workflow_sha`. The exact check-run claim
allows distinguishing a required job from a different hosted job in the same run.

`HostedProofVerifier::fetch` obtains keys only from
`https://token.actions.githubusercontent.com/.well-known/jwks`, over bounded HTTPS
without provider credentials. It ignores token-supplied key locations, allows RS256
only, validates the RSA signature, issuer, audience, issue/not-before/expiry bounds,
and every expected repository/run/attempt/check/actor/workflow identity. The maximum
token age/lifetime is 600 seconds with 30 seconds of clock skew. Unknown signing keys
block verification until trusted keys are refreshed; errors never print a JWT.

The host constructs `HostedProofExpectation` from its own admitted operation and
provider observations. It includes a fresh challenge and exact tested source. Its
audience is a versioned digest binding runtime, operation, challenge, repository and
source. The reusable workflow ref must end in its exact commit SHA. Both caller and
reusable workflow revisions are checked, along with `runner_environment=github-hosted`.
A request carrying another job, attempt, revision, actor, repository or audience fails.

The OIDC `sha` claim identifies the triggering revision. In a dispatch workflow that
checks out a different exact source input, it does **not** attest that checkout. Target
preparation must configure and review an immutable reusable workflow which:

1. Receives the runtime operation/challenge/source contract and validates its format.
2. Checks out the exact requested source and verifies the actual tree/revision used by
   the complete required suite. It must never derive “tested SHA” solely from input text.
3. Runs each required job on GitHub-hosted execution and obtains its own OIDC token
   with the source-bound audience after its source check. A sibling job's proof does
   not qualify a required job. All required jobs/check identities remain explicit.
4. Stores each token in a private, bounded evidence artifact and makes it available to
   the host while still valid. Do not log it, publish it in comments, or use a generic
   cloud-resource audience. No cloud credential exchange is required.
5. Provides the source/result artifacts needed by the product gates; token validity
   alone does not prove test success or that the complete suite ran.

Granting `id-token: write` allows token issuance and does not itself grant cloud-resource
permissions. The reusable workflow and its pinned dependencies still require repository
preparation/review. Kegari does not rewrite target security policies or workflow files.
An ordinary arbitrary workflow that requests a source-looking audience is insufficient.

`verify_and_record` verifies a token and then claims its JTI under an OS lock in private,
sealed storage. Reobservation is allowed only for the same token and complete expectation.
Another operation/evidence tuple receives `proof_replay`. Stored evidence contains token,
JTI and verification-key digests, exact expectation, verified time and expiry; no bearer
JWT. Historical sealed records can preserve an already-observed proof after token expiry.
Their identity still has to match the current work/correction and required suite evidence.

This is an implemented cryptographic verification route with offline signed proof tests.
It is not a claim that a target workflow has been prepared, that proof artifact collection
is wired into the executable scheduler, or that a real hosted run has passed qualification.

## Verified compatibility sources and remaining integration work

The Linear query/mutation fields were inspected against the official SDK schema at
revision [`618a737a4e6aa279b7853c650f79f209a5142bbc`](https://github.com/linear/linear/blob/618a737a4e6aa279b7853c650f79f209a5142bbc/packages/sdk/src/schema.graphql).
Authentication, partial errors, cursors, uploads and rate timestamps were checked against
[GraphQL guidance](https://linear.app/developers/graphql),
[pagination](https://linear.app/developers/pagination),
[file uploads](https://linear.app/developers/how-to-upload-a-file-to-linear) and
[rate limiting](https://linear.app/developers/rate-limiting).

GitHub paths, fields and dispatch return details were inspected in the official
[REST OpenAPI description](https://github.com/github/rest-api-description/blob/main/descriptions/api.github.com/api.github.com.json),
retrieved on 2026-10-25, SHA-256
`3177ad6a86ca04c57a9bef3fc24e28a339efb8eb040c6f950af3d3ed2fee7fa3`. Mutable `main` is a retrieval
snapshot, not a pinned repository revision.

OIDC fields/key discovery were checked against the official
[OIDC reference](https://docs.github.com/en/actions/reference/security/oidc),
[issuer discovery](https://token.actions.githubusercontent.com/.well-known/openid-configuration),
[JWKS](https://token.actions.githubusercontent.com/.well-known/jwks) and
[reusable workflow guidance](https://docs.github.com/en/actions/how-tos/secure-your-work/security-harden-deployments/oidc-with-reusable-workflows).

Remaining integration work is explicit: wire configuration/credential ownership, provider
reads and mutation gates into the executable host; implement/qualify target preparation
and trusted workflow artifact extraction/collection; map provider records to the full
current generation gate types; qualify actual account permissions, Git object delivery,
branch protections, dispatch correlation, native same-account review, MP4 accessibility
and playback. Linear structural child creation/metadata edits are not yet commands in
this adapter increment. No live provider mutation or agent trial was run for these tests.
# Required hosted-proof configuration

Every `CiSuite` and `ExactRevisionWorkflow` now requires a strict `proof` object:
`workflow_ref`, `workflow_sha`, `job_workflow_ref`, `job_workflow_sha`, and
`event_name`. Both SHA fields are exact lowercase commit IDs. The reusable
workflow reference must end in its configured `@SHA`; mutable reusable refs are
rejected. Exact-source workflows additionally require `workflow_dispatch` and a
caller SHA equal to their configured `workflow_revision`. Existing undeployed
draft configuration must add reviewed explicit routes; there is no inferred pin
or migration that modifies target workflows.

A static caller SHA describes a prepared trusted workflow revision. An ordinary
PR-triggered caller's `workflow_sha` can change with the PR head, so an operator
cannot assume a fixed pin matches every PR. The prepared validation contract must
establish the configured caller/reusable identity and independently prove the
actual source checkout through the source-bound audience and result manifest.
Installing or reviewing that trusted workflow is a target preparation decision,
not an automatic repository policy change.

Connection import can call `verify_connection_identity` before any workspace
exists. It checks configured account equality through Linear viewer/organization
or GitHub `/user`, using the same bounded HTTPS and GraphQL error handling.
GitHub's returned `type` must be `User`; missing, bot and organization types fail
closed. Codex authentication uses the app-server handshake instead. This helper
never discovers a default account from work text and performs no mutation.

Git commit publication requires explicit author and committer name, email and
RFC3339 date plus the expected validated local commit SHA. GitHub receives those
fields and an acknowledgement with a different SHA is uncertain rather than an
adopted replacement commit. Exact object preservation still needs the delivery
pipeline to verify locally retained/fetched Git objects.


Remote dispatch has a durable `Prepared` reservation before asynchronous provider
preflight. The current actor/work gate runs before preflight and again under the
control lock immediately before `Sent` and owned transport-task launch. Network
reads and response waits do not hold that lock. The owned task survives caller
cancellation; a replay returns the same receipt. An active transport cannot be
manually reconciled or allow service shutdown. Failure after dispatch leaves the
receipt and target hold unresolved; it never authorizes a resend. Completed
observations publish under the same lock so a newly observed correction cannot
race between the final check and launch.

## Private Linear recording readback

`LinearClient::download_uploaded_asset` accepts IDs of actual protected grant,
upload and association receipts. Before any download it verifies their immutable
account, issue, size, content type, grant and content digest bindings and their
actual acknowledgements. It rereads the complete native comment set and requires
the exact acknowledged comment ID, full dispatched body digest, native writer
and canonical recording Markdown prefix. It reads the comment again after the
bytes to detect an association changed during download. A URL mentioned inside
arbitrary comment text is insufficient.

The supported private-storage route is the canonical
`https://api.linear.app/graphql` API and `https://uploads.linear.app` on port 443.
Only the exact acknowledged asset URL receives the configured Linear credential.
Queries, fragments, userinfo, alternate storage hosts, Markdown URL delimiters and
redirects are rejected. A dedicated bounded streaming transport permits recording
bodies up to 256 MiB; ordinary JSON/provider bodies remain limited to 8 MiB. Actual
content type, byte count and SHA-256 must match the upload. Its opaque result
proves byte identity and current association, not media decoding, browser playback
or parent acceptance. The delivery layer verifies the actual MP4 separately.

This follows Linear's official
[file-storage authentication contract](https://linear.app/developers/file-storage-authentication)
and [manual upload/Markdown association contract](https://linear.app/developers/how-to-upload-a-file-to-linear),
checked 2026-10-25. Compatible custom GraphQL endpoints have no inferred storage
authentication contract; that path reports an explicit prerequisite blocker.
Synthetic wire tests do not establish real-account storage access or playback.
