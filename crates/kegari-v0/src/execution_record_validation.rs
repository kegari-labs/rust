//! Descriptive check-record consistency. This reader never reconstructs an
//! execution capability, Control admission, process owner or qualification.
use super::*;

fn malformed(message: &str) -> Error {
    Error::new("check_record_invalid", message)
}

fn outcome_shape(outcome: &CommandOutcome, isolation: &str) -> Result<()> {
    if outcome.schema_version != 1
        || outcome.argv.is_empty()
        || outcome.argv[0].trim().is_empty()
        || outcome.argv.iter().any(|argument| argument.contains('\0'))
        || !outcome.cwd.is_absolute()
        || outcome.isolation_digest != isolation
        || outcome.termination_signal.is_some_and(|signal| signal <= 0)
        || (outcome.exit_code.is_some() && outcome.termination_signal.is_some())
    {
        return Err(malformed(
            "check command identity or terminal shape is inconsistent",
        ));
    }
    Ok(())
}

fn clean(outcome: &CommandOutcome) -> bool {
    outcome.exit_code == Some(0)
        && outcome.termination_signal.is_none()
        && !outcome.timed_out
        && !outcome.output_truncated
}

impl CheckExecutionRecord {
    /// Validate only facts represented in this record. Exact configured argv,
    /// workflow/service membership and actual closed ownership remain duties of
    /// the private producer and its enclosing original workflow/Control gates.
    pub(crate) fn validate_shape(&self) -> Result<()> {
        let schema_matches = match self.schema_version {
            1 => {
                self.codex.is_none()
                    && self.discovered.is_none()
                    && self.correction.is_none()
                    && self.bundle.is_none()
            }
            2 => {
                self.codex.is_none()
                    && self.discovered.is_none()
                    && self.correction.is_some()
                    && self.bundle.is_none()
            }
            3 => self.codex.is_none() && self.discovered.is_none() && self.bundle.is_some(),
            4 => self.codex.is_none() && self.discovered.is_some(),
            5 => {
                self.codex.is_some()
                    && self.discovered.is_none()
                    && self.bundle.is_none()
                    && self.evidence_class == ExecutionEvidenceClass::CodexExecution
            }
            _ => false,
        };
        if !schema_matches
            || self.binding.runtime_id.is_nil()
            || self.binding.operation_id.is_nil()
            || self.check_name.is_empty()
            || self.completed_at < self.started_at
            || self.successful != self.blocker.is_none()
            || (self.successful && (!clean(&self.command) || self.before != self.after))
        {
            return Err(malformed("check schema, result or binding is inconsistent"));
        }
        crate::evidence::validate_digest(&self.binding.isolation_digest)?;
        crate::evidence::validate_digest(&self.workflow_digest)?;
        let command_isolation = match &self.discovered {
            Some(_) => {
                return Err(Error::new(
                    "legacy_execution_evidence",
                    "custom-executor discoveries require fresh Codex validation",
                ));
            }
            None => &self.binding.isolation_digest,
        };
        outcome_shape(&self.command, command_isolation)?;
        if let Some(codex) = &self.codex {
            codex.validate_record(self)?;
        } else if self.evidence_class == ExecutionEvidenceClass::CodexExecution {
            return Err(malformed(
                "legacy execution cannot be relabelled as a Codex result",
            ));
        }
        if let Some(correction) = &self.correction {
            for digest in [
                &correction.original_binding_digest,
                &correction.transition_digest,
                &correction.input_digest,
            ] {
                crate::evidence::validate_digest(digest)?;
            }
        }
        let Some(bundle) = &self.bundle else {
            return Ok(());
        };
        crate::evidence::validate_digest(&bundle.plan_digest)?;
        if bundle.schema_version != 1
            || bundle.effect_id.is_nil()
            || bundle.effect_id == self.binding.operation_id
            || bundle.services.is_empty()
            || bundle.services.len() > kegari_context::workflow::MAX_CHECK_SERVICES
        {
            return Err(malformed(
                "check bundle identity or service count is invalid",
            ));
        }
        let mut names = BTreeSet::new();
        let mut processes = BTreeSet::from([bundle.effect_id, self.binding.operation_id]);
        for service in &bundle.services {
            if service.name.is_empty()
                || service.name.len() > kegari_context::workflow::MAX_SERVICE_NAME_BYTES
                || !service.name.as_bytes()[0].is_ascii_alphanumeric()
                || !service
                    .name
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"_.-".contains(&byte))
                || !names.insert(&service.name)
                || [service.service_process, service.readiness_process]
                    .into_iter()
                    .any(|id| id.is_nil() || !processes.insert(id))
                || service.stopped.leader_running
                || !service.stopped.stop_requested
                || service.stopped.kill_escalated
            {
                return Err(malformed(
                    "check service identity or closed lifecycle is invalid",
                ));
            }
            for outcome in [&service.readiness, &service.terminal] {
                outcome_shape(outcome, command_isolation)?;
                if outcome.cwd != self.command.cwd || !clean(outcome) {
                    return Err(malformed(
                        "check service is not a clean result in the same boundary",
                    ));
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::evaluation::{CheckStatus, observed_check};
    use crate::evidence::Generation;
    use kegari_execution::ServiceObservation;

    // Synthetic descriptive records only: these tests never construct private
    // execution/owner/Control capabilities or claim a real service was launched.
    fn record(schema: u32) -> CheckExecutionRecord {
        let digest = bytes_digest(b"descriptive check fixture");
        let revision = Revision {
            commit: "a".repeat(40),
            tree: "b".repeat(40),
        };
        let command = CommandOutcome {
            schema_version: 1,
            argv: vec!["/fixture/tool".into()],
            cwd: PathBuf::from("/fixture/repository"),
            isolation_digest: digest.clone(),
            exit_code: Some(0),
            termination_signal: None,
            timed_out: false,
            stdout: String::new(),
            stderr: String::new(),
            output_truncated: false,
            elapsed_ms: 1,
        };
        CheckExecutionRecord {
            codex: None,
            discovered: None,
            schema_version: schema,
            correction: (schema == 2).then(|| CorrectionExecutionProvenance {
                original_binding_digest: digest.clone(),
                transition_digest: digest.clone(),
                watermark: 1,
                input_digest: digest.clone(),
            }),
            bundle: (schema == 3).then(|| CheckBundleExecutionRecord {
                schema_version: 1,
                effect_id: Uuid::from_u128(3),
                plan_digest: digest.clone(),
                services: vec![ServiceExecutionRecord {
                    name: "9database.v1".into(),
                    service_process: Uuid::from_u128(4),
                    readiness_process: Uuid::from_u128(5),
                    readiness: command.clone(),
                    terminal: command.clone(),
                    stopped: ServiceObservation {
                        leader_running: false,
                        stop_requested: true,
                        kill_escalated: false,
                    },
                }],
            }),
            binding: ExecutionBinding {
                runtime_id: Uuid::from_u128(1),
                operation_id: Uuid::from_u128(2),
                work: WorkIdentity {
                    linear_endpoint: "https://linear.invalid/graphql".into(),
                    linear_workspace: Uuid::from_u128(6),
                    parent: Uuid::from_u128(7),
                    child: Some(Uuid::from_u128(8)),
                    generation: Generation {
                        authorization: digest.clone(),
                        definition: digest.clone(),
                        correction: digest.clone(),
                        scope: digest.clone(),
                    },
                },
                repository: RepositoryId {
                    hostname: "github.invalid".into(),
                    id: "fixture".into(),
                },
                revision: revision.clone(),
                candidate_vector_digest: None,
                isolation_digest: digest.clone(),
            },
            workflow_digest: digest,
            check_name: "check".into(),
            kind: CheckKind::Local,
            before: SourceSnapshot {
                revision: revision.clone(),
                files: BTreeMap::new(),
            },
            after: SourceSnapshot {
                revision,
                files: BTreeMap::new(),
            },
            artifacts: Vec::new(),
            command,
            started_at: DateTime::from_timestamp(1, 0).unwrap(),
            completed_at: DateTime::from_timestamp(2, 0).unwrap(),
            evidence_class: ExecutionEvidenceClass::DeterministicFixture,
            successful: true,
            blocker: None,
        }
    }

    fn status(record: &CheckExecutionRecord) -> CheckStatus {
        observed_check(record, &BTreeMap::new(), &record.after.revision.commit)
            .unwrap()
            .status
    }

    #[test]
    fn all_three_check_schemas_preserve_actual_candidate_observation() {
        for schema in 1..=3 {
            let mut record = record(schema);
            record.validate_shape().unwrap();
            assert_eq!(status(&record), CheckStatus::Passed);
            let original = serde_json::to_value(&record).unwrap();
            if schema < 3 {
                assert!(original.get("bundle").is_none());
            }
            if schema == 1 {
                assert!(original.get("correction").is_none());
            }
            record.command.exit_code = Some(7);
            record.successful = false;
            record.blocker = Some("actual descriptive failed command".into());
            record.validate_shape().unwrap();
            assert_eq!(status(&record), CheckStatus::Failed);
            assert_eq!(
                observed_check(&record, &BTreeMap::new(), &"c".repeat(40))
                    .unwrap()
                    .status,
                CheckStatus::Unknown
            );
        }
        let mut corrected_bundle = record(3);
        corrected_bundle.correction = record(2).correction;
        corrected_bundle.validate_shape().unwrap();
        assert_eq!(status(&corrected_bundle), CheckStatus::Passed);
    }

    #[test]
    fn schema_presence_conflicts_remain_unknown_without_promoting_old_records() {
        let mut cases = Vec::new();
        for schema in [0, 1, 2, 4] {
            let mut changed = record(3);
            changed.schema_version = schema;
            cases.push(changed);
        }
        let mut missing = record(3);
        missing.bundle = None;
        cases.push(missing);
        let mut missing = record(2);
        missing.correction = None;
        cases.push(missing);
        let mut forbidden = record(1);
        forbidden.correction = record(2).correction;
        cases.push(forbidden);
        for changed in cases {
            assert!(changed.validate_shape().is_err());
            assert_eq!(status(&changed), CheckStatus::Unknown);
        }
    }

    #[test]
    fn bundle_identity_lifecycle_and_outcome_contradictions_are_unknown() {
        let original = serde_json::to_value(record(3)).unwrap();
        let mutations = [
            ("/bundle/schema_version", serde_json::json!(2)),
            ("/bundle/effect_id", serde_json::json!(Uuid::nil())),
            ("/bundle/effect_id", serde_json::json!(Uuid::from_u128(2))),
            ("/bundle/plan_digest", serde_json::json!("invalid")),
            ("/bundle/services", serde_json::json!([])),
            ("/bundle/services/0/name", serde_json::json!("_bad")),
            (
                "/bundle/services/0/service_process",
                serde_json::json!(Uuid::nil()),
            ),
            (
                "/bundle/services/0/readiness_process",
                serde_json::json!(Uuid::from_u128(4)),
            ),
            (
                "/bundle/services/0/stopped/leader_running",
                serde_json::json!(true),
            ),
            (
                "/bundle/services/0/stopped/stop_requested",
                serde_json::json!(false),
            ),
            (
                "/bundle/services/0/stopped/kill_escalated",
                serde_json::json!(true),
            ),
            (
                "/bundle/services/0/readiness/schema_version",
                serde_json::json!(2),
            ),
            (
                "/bundle/services/0/readiness/cwd",
                serde_json::json!("/other"),
            ),
            (
                "/bundle/services/0/readiness/isolation_digest",
                serde_json::json!(bytes_digest(b"other")),
            ),
            (
                "/bundle/services/0/readiness/exit_code",
                serde_json::json!(1),
            ),
            (
                "/bundle/services/0/terminal/timed_out",
                serde_json::json!(true),
            ),
            (
                "/bundle/services/0/terminal/output_truncated",
                serde_json::json!(true),
            ),
            ("/command/schema_version", serde_json::json!(2)),
        ];
        for (pointer, value) in mutations {
            let mut changed = original.clone();
            *changed.pointer_mut(pointer).unwrap() = value;
            let changed: CheckExecutionRecord = serde_json::from_value(changed).unwrap();
            assert!(changed.validate_shape().is_err(), "{pointer}");
            assert_eq!(status(&changed), CheckStatus::Unknown, "{pointer}");
        }
        for terminal in [false, true] {
            let mut changed = record(3);
            if terminal {
                changed.bundle.as_mut().unwrap().services[0]
                    .terminal
                    .termination_signal = Some(15);
            } else {
                changed.command.termination_signal = Some(15);
            }
            assert!(changed.validate_shape().is_err());
            assert_eq!(status(&changed), CheckStatus::Unknown);
        }
    }

    #[test]
    fn bundle_service_and_process_inventories_are_unique_and_bounded() {
        let mut record = record(3);
        let first = record.bundle.as_ref().unwrap().services[0].clone();
        for index in 1..kegari_context::workflow::MAX_CHECK_SERVICES {
            let mut service = first.clone();
            service.name = format!("service-{index}");
            service.service_process = Uuid::from_u128(100 + 2 * index as u128);
            service.readiness_process = Uuid::from_u128(101 + 2 * index as u128);
            record.bundle.as_mut().unwrap().services.push(service);
        }
        record.validate_shape().unwrap();
        for duplicate_name in [false, true] {
            let mut changed = record.clone();
            let second = &mut changed.bundle.as_mut().unwrap().services[1];
            if duplicate_name {
                second.name = first.name.clone();
            } else {
                second.service_process = first.service_process;
            }
            assert!(changed.validate_shape().is_err());
            assert_eq!(status(&changed), CheckStatus::Unknown);
        }
        let mut ninth = first;
        ninth.name = "service-nine".into();
        ninth.service_process = Uuid::from_u128(200);
        ninth.readiness_process = Uuid::from_u128(201);
        record.bundle.as_mut().unwrap().services.push(ninth);
        assert!(record.validate_shape().is_err());
        assert_eq!(status(&record), CheckStatus::Unknown);
    }
}
