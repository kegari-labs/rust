//! Immutable carry provenance. These private-field records authenticate history
//! only; deserializing them never creates current acceptance or readiness.
use super::*;

const MAX_REFERENCES: usize = 4096;
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(deny_unknown_fields, tag = "kind", rename_all = "snake_case")]
pub(crate) enum CarryNamespace {
    Root,
    Workspace { workspace: Uuid },
    Deployment { workspace: Uuid },
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CarryFile {
    namespace: CarryNamespace,
    leaf: String,
    purpose: String,
    digest: String,
}
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CarryReferences {
    files: Vec<CarryFile>,
    receipts: BTreeMap<Uuid, String>,
}
impl CarryReferences {
    pub(crate) fn retain_file(
        &mut self,
        namespace: CarryNamespace,
        records: &PrivateDir,
        leaf: &str,
        purpose: &str,
        key: &AttestationKey,
    ) -> Result<()> {
        let signed: Signed<Value> = records.read(leaf)?;
        key.verify(purpose, &signed)?;
        let reference = CarryFile {
            namespace,
            leaf: leaf.into(),
            purpose: purpose.into(),
            digest: signed.digest()?,
        };
        if let Some(prior) = self
            .files
            .iter()
            .find(|p| p.namespace == reference.namespace && p.leaf == reference.leaf)
        {
            if prior != &reference {
                return Err(invalid("carry reference changed during collection"));
            }
        } else {
            self.files.push(reference);
            self.files
                .sort_by(|a, b| (&a.namespace, &a.leaf).cmp(&(&b.namespace, &b.leaf)));
        }
        self.check_bound()
    }
    pub(crate) fn retain_receipt(&mut self, receipt: &Receipt) -> Result<()> {
        let digest = canonical_digest(1, receipt)?;
        if self
            .receipts
            .insert(receipt.operation_id, digest.clone())
            .is_some_and(|prior| prior != digest)
        {
            return Err(invalid("carry receipt changed during collection"));
        }
        self.check_bound()
    }
    pub(super) fn require_receipt(&self, receipt: &Receipt) -> Result<()> {
        if self.receipts.get(&receipt.operation_id) != Some(&canonical_digest(1, receipt)?) {
            return Err(invalid("carry closure omits exact required receipt"));
        }
        Ok(())
    }
    pub(crate) fn extend(&mut self, other: &Self) -> Result<()> {
        for reference in &other.files {
            if let Some(prior) = self
                .files
                .iter()
                .find(|p| p.namespace == reference.namespace && p.leaf == reference.leaf)
            {
                if prior != reference {
                    return Err(invalid("carry reference closure conflicts"));
                }
            } else {
                self.files.push(reference.clone());
            }
            self.check_bound()?;
        }
        self.files
            .sort_by(|a, b| (&a.namespace, &a.leaf).cmp(&(&b.namespace, &b.leaf)));
        for (id, digest) in &other.receipts {
            if self
                .receipts
                .insert(*id, digest.clone())
                .is_some_and(|prior| prior != *digest)
            {
                return Err(invalid("carry receipt closure conflicts"));
            }
            self.check_bound()?;
        }
        Ok(())
    }
    fn check_bound(&self) -> Result<()> {
        if self
            .files
            .len()
            .checked_add(self.receipts.len())
            .is_none_or(|n| n > MAX_REFERENCES)
        {
            return Err(invalid(
                "carry immutable reference closure exceeds retained inventory bound",
            ));
        }
        Ok(())
    }
    pub(crate) fn validate(
        &self,
        root: &PrivateDir,
        control: &ControlHost,
        key: &AttestationKey,
    ) -> Result<()> {
        self.check_bound()?;
        let mut seen = BTreeSet::new();
        for reference in &self.files {
            if !seen.insert((&reference.namespace, &reference.leaf)) {
                return Err(invalid("duplicate carry file reference"));
            }
            evidence::validate_digest(&reference.digest)?;
            let directory = match reference.namespace {
                CarryNamespace::Root => None,
                CarryNamespace::Workspace { workspace } => {
                    Some(PrivateDir::open(&root.path().join(workspace.to_string()))?)
                }
                CarryNamespace::Deployment { workspace } => Some(PrivateDir::open(
                    &root.path().join(workspace.to_string()).join("deployment"),
                )?),
            };
            let signed: Signed<Value> = directory.as_ref().unwrap_or(root).read(&reference.leaf)?;
            key.verify(&reference.purpose, &signed)?;
            if signed.digest()? != reference.digest {
                return Err(invalid("carry immutable reference disappeared or changed"));
            }
        }
        for (id, digest) in &self.receipts {
            if canonical_digest(1, &control.operation(*id)?)? != *digest {
                return Err(invalid("carry immutable receipt disappeared or changed"));
            }
        }
        Ok(())
    }
}
fn invalid(message: &str) -> Error {
    Error::new("acceptance_carry_history", message)
}

#[cfg(test)]
mod tests {
    use super::*;
    /// Protected-file reference mechanics only: no execution, parent,
    /// acceptance, or release-completion capability is manufactured.
    #[test]
    fn exact_workspace_reference_refuses_missing_shadow_and_authenticated_change() {
        let temporary = tempfile::tempdir().unwrap();
        let root =
            PrivateDir::create(&temporary.path().canonicalize().unwrap().join("records")).unwrap();
        let workspace = Uuid::new_v4();
        let records = PrivateDir::create(&root.path().join(workspace.to_string())).unwrap();
        let control = ControlHost::setup(
            PrivateDir::create(&temporary.path().canonicalize().unwrap().join("control")).unwrap(),
            Uuid::new_v4(),
            Uuid::new_v4(),
        )
        .unwrap();
        let key = AttestationKey::new("component key".into(), vec![19; 32]).unwrap();
        let signed = key
            .sign("component_record", &json!({"source":"original"}))
            .unwrap();
        records.write("source.json", &signed).unwrap();
        let mut refs = CarryReferences::default();
        refs.retain_file(
            CarryNamespace::Workspace { workspace },
            &records,
            "source.json",
            "component_record",
            &key,
        )
        .unwrap();
        refs.validate(&root, &control, &key).unwrap();
        std::fs::remove_file(records.path().join("source.json")).unwrap();
        root.write("source.json", &signed).unwrap();
        assert_eq!(
            refs.validate(&root, &control, &key).unwrap_err().code,
            "record_missing"
        );
        records.write("source.json", &signed).unwrap();
        refs.validate(&root, &control, &key).unwrap();
        records
            .write(
                "source.json",
                &key.sign("component_record", &json!({"source":"other"}))
                    .unwrap(),
            )
            .unwrap();
        let error = refs.validate(&root, &control, &key).unwrap_err();
        assert_eq!(error.code, "acceptance_carry_history");
        assert_eq!(
            error.message,
            "carry immutable reference disappeared or changed"
        );
    }
    #[test]
    fn copied_reference_data_never_accepts_duplicate_or_wrong_domain_content() {
        let temporary = tempfile::tempdir().unwrap();
        let root =
            PrivateDir::create(&temporary.path().canonicalize().unwrap().join("records")).unwrap();
        let control = ControlHost::setup(
            PrivateDir::create(&temporary.path().canonicalize().unwrap().join("control")).unwrap(),
            Uuid::new_v4(),
            Uuid::new_v4(),
        )
        .unwrap();
        let key = AttestationKey::new("component key".into(), vec![31; 32]).unwrap();
        let signed = key.sign("component_record", &json!({"record":1})).unwrap();
        root.write("source.json", &signed).unwrap();
        let mut refs = CarryReferences::default();
        refs.retain_file(
            CarryNamespace::Root,
            &root,
            "source.json",
            "component_record",
            &key,
        )
        .unwrap();
        let mut duplicate = refs.clone();
        duplicate.files.push(duplicate.files[0].clone());
        assert_eq!(
            duplicate
                .validate(&root, &control, &key)
                .unwrap_err()
                .message,
            "duplicate carry file reference"
        );
        root.write(
            "source.json",
            &key.sign("different_domain", &json!({"record":1})).unwrap(),
        )
        .unwrap();
        assert_eq!(
            refs.validate(&root, &control, &key).unwrap_err().code,
            "invalid_attestation"
        );
    }
}
