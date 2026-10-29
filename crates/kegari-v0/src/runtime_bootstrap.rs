//! Preserve an actual initial source commit after a settled native Codex bootstrap.
//! This does not grant initial remote publication, review, acceptance or release.
use super::*;
use crate::control::MutationRequest;
use crate::git::{Git, GitLimits};
use crate::worker_run::RunOutcome;
use kegari_control_api::identity::Actor;
use kegari_git::{SourceCommitMetadata, WriteScope};

impl RuntimeCoordinator {
    pub(super) async fn finish_unborn_bootstrap(
        self: &Arc<Self>,
        completion: &mut RunCompletion,
    ) -> Result<Option<Value>> {
        let Some(contract) = completion.contract.clone() else {
            return Ok(None);
        };
        let Some(bootstrap) = contract.unborn_bootstrap().cloned() else {
            return Ok(None);
        };
        if completion.error.is_some() || completion.hook_error.is_some() {
            return Ok(None);
        }
        if !completion.local_exit_confirmed
            || !completion.state.completion_is_current()
            || !matches!(completion.outcome, RunOutcome::Completed)
        {
            return Err(Error::new(
                "bootstrap_incomplete",
                "bootstrap source is preserved; only a completed, joined native session can seal its initial commit",
            ));
        }
        bootstrap.require_settled()?;
        bootstrap.revalidate(contract.isolation())?;
        let permit = self
            .state
            .lock()
            .map_err(|_| Error::new("runtime_unavailable", "bootstrap registration lock failed"))?
            .runs
            .get(&completion.operation_id)
            .ok_or_else(|| Error::new("ownership_conflict", "bootstrap registration disappeared"))?
            .permit
            .clone();
        let fresh = || {
            self.verify_owner(&permit.lease)?;
            contract
                .authority()
                .compare(&self.retained_authority(contract.authority())?)?;
            bootstrap.require_settled()
        };
        self.control.validate_permit(&permit, fresh)?;
        let scope = match contract.authority() {
            WorkerAuthority::Authorized(gate) => WriteScope {
                paths: gate
                    .child_scope()
                    .ok_or_else(|| {
                        Error::new(
                            "scope_violation",
                            "bootstrap has no current child write scope",
                        )
                    })?
                    .write_paths
                    .clone(),
            },
            _ => {
                return Err(Error::new(
                    "scope_violation",
                    "only an authorized child can seal initial source",
                ));
            }
        };
        scope.validate()?;
        let records = PrivateDir::open(
            &self
                .registration
                .root()
                .path()
                .join("worker-records")
                .join(completion.operation_id.to_string()),
        )?;
        if records
            .path()
            .join("bootstrap-source.json")
            .try_exists()
            .map_err(|e| Error::io("inspect bootstrap source record", e))?
        {
            return Err(Error::new(
                "reconciliation_required",
                "initial source already has a retained result; never repeat it automatically",
            ));
        }
        let git = Git::from_pinned_executable(bootstrap.git_executable())?;
        let processes = Arc::new(crate::process_records::ProcessJournal::new(
            self.registration.root().child("worker-processes")?,
            self.runtime_id,
        )?);
        let start_gate = crate::runtime_workers::scoped_git_gate(
            self.clone(),
            permit.clone(),
            contract.authority().clone(),
        );
        let proof = bootstrap.proof().clone();
        let boundary = contract.isolation().clone();
        let source = proof.identity().operator_checkout.clone();
        let model = contract.instructions().manifest.profile.model.clone();
        let instruction = contract.instructions().manifest_digest.clone();
        let request = MutationRequest {
            request_id: Uuid::new_v4(),
            runtime_id: self.runtime_id,
            actor: Actor {
                identity: format!("owner:{}", permit.lease.owner_id),
                roles: BTreeSet::new(),
                principal: None,
            },
            kind: "worker_bootstrap_source".into(),
            canonical_targets: vec![permit.work_key.clone()],
            expected: BTreeMap::from([
                (
                    "worker_operation".into(),
                    completion.operation_id.to_string(),
                ),
                ("scope".into(), contract.scope_digest().into()),
            ]),
            payload: json!({"parent":Value::Null,"repository":proof.identity().repository,"directory":proof.identity(),"branch":proof.branch(),"instruction_digest":instruction}),
        };
        // Joined app-server ownership does not cover the new owned Git effect.
        // Keep retirement closed until that actual handle has also joined.
        completion.local_exit_confirmed = false;
        let (effect,running) = self.control.begin_effect(request,&permit,fresh,|receipt| {
            let reservation = self.executions.reserve(permit.operation_id,receipt.operation_id)?;
            records.write("bootstrap-intent.json",&self.services.key.sign("bootstrap_source_intent",&json!({"operation":permit.operation_id,"effect":receipt.operation_id,"parent":Value::Null,"directory":proof.identity(),"scope":scope,"instruction_digest":instruction}))?)?;
            let running = git.start_journaled_operation(GitLimits::default(),start_gate,processes,permit.clone(),move |git| {
                let identity=git.configured_commit_identity(&source)?;
                let message=format!("feat: initialize scoped repository source\n\nInstruction-digest: {instruction}\nAssisted-by: Codex ({model})\nSigned-off-by: {} <{}>\n",identity.name,identity.email);
                let timestamp=chrono::DateTime::from_timestamp(Utc::now().timestamp(),0).ok_or_else(||Error::invalid("bootstrap commit time outside range"))?;
                git.commit_unborn_execution(&proof,&boundary,&scope,SourceCommitMetadata {identity,timestamp,message})
            })?;
            reservation.attach(Arc::new(running.cancellation()))?;
            Ok(running)
        })?;
        let cancellation = running.cancellation();
        let result = running.wait().await;
        if !matches!(cancellation.exited(), Ok(true)) {
            self.control.fence(permit.operation_id)?;
            self.executions
                .cancel_fenced(&self.control, permit.operation_id)?;
            return Err(Error::new(
                "operation_uncertain",
                "root commit ownership has not joined; preserve source and receipt",
            ));
        }
        completion.local_exit_confirmed = true;
        self.executions
            .remove(permit.operation_id, effect.operation_id())?;
        let observed = match &result {
            Ok(root) => {
                json!({"parent":Value::Null,"revision":root.revision(),"metadata":root.metadata(),"files":root.files(),"accepted":false,"released":false})
            }
            Err(error) => json!({"parent":Value::Null,"error":error,"source":"retained"}),
        };
        let receipt = self
            .control
            .complete_effect(effect, &permit, fresh, observed.clone())?;
        let root = result?;
        let record = json!({"schema_version":1,"operation":permit.operation_id,"receipt":receipt.request_id,"runtime":self.runtime_id,"work":permit.work_key,"directory":bootstrap.proof().identity(),"branch":bootstrap.proof().branch(),"parent":Value::Null,"revision":root.revision(),"metadata":root.metadata(),"files":root.files(),"scope":contract.scope_digest(),"instruction_digest":contract.instructions().manifest_digest,"accepted":false,"released":false,"next_action":"initial_publication_required"});
        records.write(
            "bootstrap-source.json",
            &self.services.key.sign("bootstrap_source_result", &record)?,
        )?;
        completion.outcome = RunOutcome::Blocked;
        completion.error=Some(Error::new("initial_publication_required","actual initial commit retained; an explicit initial-publication decision and current provider baseline are required before normal PR/review/delivery").detail(json!({"commit":root.revision().commit,"tree":root.revision().tree,"parent":Value::Null,"workspace":bootstrap.proof().identity().repository_path,"record":records.path().join("bootstrap-source.json")})).next("Review the retained initial commit and choose an explicit repository-initialization policy; no remote branch, PR or accepted result was created"));
        Ok(Some(record))
    }
}
