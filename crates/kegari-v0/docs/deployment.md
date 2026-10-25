# Production target evidence

`ProductionObserver` reads the actual production `main` commit/tree and every configured deployment target. Its private `VerifiedProductionBaseline` capability contains complete per-target deployed source and artifact maps. Common source/artifact values are present only when every target actually shares them. Empty source maps are omitted from serialization so historical signed records retain their original canonical digest. A deployment-free repository reads and rechecks `main` and its commit tree without inventing deployment activity.

`environment_id` is the canonical positive numeric GitHub environment ID, not its mutable display name. The observer resolves and rereads the exact current name from that ID. This schema has not been deployed; draft configurations using names must replace them with verified canonical IDs.

A GitHub deployment is a request to an integration, and its payload is arbitrary caller data. A successful status alone cannot prove deployed bytes. The adapter therefore requires a reviewed, configured, SHA-pinned reusable workflow to attest the exact target result. This is an explicit repository preparation contract; Kegari does not change repository policies or install a workflow automatically. See GitHub's [deployment API](https://docs.github.com/en/rest/deployments/deployments) and [deployment status API](https://docs.github.com/en/rest/deployments/statuses).

The host captures a private signed challenge only for an exact live release/recovery admission whose tested tree matches the current production source. It passes the configured source/operation inputs plus `kegari_runtime_id`, `kegari_phase_operation`, `kegari_challenge`, `kegari_step_id`, `kegari_target_id`, `kegari_environment_id`, and `kegari_source_tree` through the broker's scoped workflow-dispatch command. The operation input is the host's exact dispatch request UUID. Preparation retains one signed challenge and dispatch UUID per admitted operation/step; exact repeats reuse them, and changed source/contracts conflict. The sealed deployment dispatch also carries the exact expected production commit/tree; provider preflight rereads `heads/main` and its commit tree immediately before the serialized dispatch gate. A provider squash commit may differ from the tested commit only when its tree is identical. Dispatch ownership comes from the actual durable acknowledged broker receipt, never an artifact field.

The challenge binds the actual admitted `Permit` scope. For a plan-level release that is the immutable plan digest; a resumed release also retains the checked original-plan/admission lineage. It is not replaced with the per-step scope digest. Earlier development records with inconsistent scope bindings are rejected rather than reinterpreted. A durably `Prepared` admission can support readback only while the exact operation/runtime/scope remains registered in `ControlHost`; recovering its serialized receipt does not restore a permit.

The successful owned run attempt must publish one artifact named `kegari-deployment-{phase operation UUID}-{attempt}-{canonical environment ID}`. Its ZIP contains exactly `kegari-deployment.json`; no paths, links, encrypted members, alternate files or unsupported compression are accepted. The compressed archive is capped at 4 MiB and its one manifest at 1 MiB. Provider metadata, signed download grant and archive digest are verified before parsing. The manifest has strict schema version 1 and these fields:

```json
{
  "schema_version": 1,
  "runtime_id": "UUID",
  "phase_operation": "UUID",
  "dispatch_request": "UUID",
  "challenge": "UUID",
  "repository_id": 42,
  "step_id": "deploy",
  "target_id": "service-production",
  "environment_id": 88,
  "deployment_id": 91,
  "run_id": 50,
  "run_attempt": 1,
  "source": {"commit": "EXACT_COMMIT", "tree": "EXACT_TREE"},
  "outputs": {"CONFIGURED_OBSERVED_SOURCE_OUTPUT": "EXACT_COMMIT"},
  "artifact_digest": "sha256:EXACT_DEPLOYED_BYTES_DIGEST",
  "proofs": [{"check_run_id": 60, "token": "FRESH_GITHUB_OIDC_JWT"}]
}
```

The trusted workflow must verify the actual deployment, canonical environment, exact checked-out tree and deployed artifact digest before asking for the token. Each successful job's OIDC proof uses the existing `HostedProofExpectation::audience()` protocol plus `context_digest = deployment_context_digest(repository_id, target_id, environment_id, deployment_id, source, artifact_digest)`. This adds a versioned canonical SHA-256 digest of `purpose: deployment_target` and those exact fields to the audience. A source-only token cannot authenticate a different artifact. The verifier checks GitHub's fixed signing-key trust anchor, bounded token lifetime, JTI ownership, actual job/check/run attempt, selected user actor, hosted-runner claim, caller workflow revision and pinned reusable workflow revision. Runner labels never establish hosted execution. No raw bearer token is serialized into retained records or public results.

On success, private HMAC-authenticated records retain the challenge, broker receipt payload identity, exact native deployment/status/environment/run/job observations, downloaded archive identity/digest, target artifact binding and verified proof references. Recovery uses that authenticated record and fresh provider reads; it does not reaccept an expired JWT. Every environment-filtered deployment page and every status page is read, without filtering by the expected SHA. This exposes divergent newer deployments. Pending work, unknown outcomes, newer failures, ambiguous active successes, replaced environment IDs, rerun attempts and changed `main` block a current baseline. Older known failures can be superseded by a later uniquely active success with its own authenticated target proof.

Each successful proof is immutable and has exact dispatch and digest lookup records. Repeated owned observation preserves its original identity. A crash may leave the current pointer absent or still referencing an older valid proof. Current observation first authenticates any present pointer and its referenced record, then permits a bounded, unique authenticated match to the actual current target facts. It still verifies the current run and, for an ordinary baseline, current `main`. A malformed pointer, missing referenced record, ambiguous match or changed native identity blocks recovery. This fallback is read-only and needs no new JWT, artifact download or dispatch; only a still-authorized owned observation may repair lookup records.

`recover_historical_target` returns a separate opaque `HistoricalTargetDeployment`. It selects the original immutable proof, rereads its exact deployment, successful status, environment policy, source tree and original workflow attempt/jobs, and rechecks the actual dispatch receipt. A later target replacement or newer run attempt cannot substitute a different proof. The reader does not assert that the original target is still current. The release-lineage consumer additionally requires the exact signed step, expected effect, original or resumed admission, locally retired owner and reconciled successful dispatch. It cannot resolve an unknown effect or grant current readiness. Merge history continues through its separate Git/provider provenance path.

Initial historical targets without retained authenticated evidence remain an explicit preparation blocker. They need a bounded, explicitly authorized run of a configured trusted validation/deployment route. Ordinary observation never dispatches a workflow. Corrected preparation may call `with_successor` only with a sealed `KnownPredecessorBaseline`. The runtime derives it from an exact signed held attempt, actual retired admission and reconciled effects, original native accepted publications, and complete current repository/target reads. It checks that every source/artifact movement is an observed effect of that predecessor. The capability rereads its protected record and actual receipt, expires after 30 seconds, and cannot clear a hold or grant a permit. The bound observer rereads the complete target vector before creating an explicitly declared successor baseline. Ordinary observation still rejects divergence; a caller-supplied recovery UUID is never sufficient.

The deterministic tests use production parsing, broker receipts, private storage and offline RSA signatures with an injected HTTP boundary. They do not qualify any real GitHub account, deployed target or trusted workflow. Live deployment validation remains unrun and requires separate explicit authorization.
