//! Bounded reads of the existing authenticated validation archive.
//! Reading pages grants no execution or evidence-acceptance authority.
use super::*;
use crate::validation_archive::ResolvedValidationArchive;
use kegari_types::artifact::ArtifactProof;

const PAGE_BYTES: usize = 24 * 1024;
const DESCRIPTORS: [&str; 2] = [
    "/evidence/delivery_assignment/delivery_review/validation_pages",
    "/evidence/delivery_assignment/integrated_validation_review/validation_pages",
];

#[derive(Clone, Debug)]
pub(super) struct ReviewPages {
    archive: ResolvedValidationArchive,
    history: Arc<Vec<u8>>,
    index: Arc<Vec<u8>>,
    history_digest: String,
    index_digest: String,
    proofs: BTreeMap<String, ArtifactProof>,
}

#[derive(Default)]
pub(super) struct PageReads(BTreeMap<String, usize>);

pub(super) fn has_descriptor(context: &Value) -> bool {
    DESCRIPTORS
        .iter()
        .any(|pointer| context.pointer(pointer).is_some())
}

impl WorkerContract {
    pub(crate) fn attach_review_archive(
        &mut self,
        archive: ResolvedValidationArchive,
    ) -> Result<()> {
        let descriptor = archive.review_descriptor()?;
        let supplied = DESCRIPTORS
            .iter()
            .filter_map(|pointer| self.context.pointer(pointer))
            .collect::<Vec<_>>();
        if self.assembled.manifest.role != AgentRole::Review
            || supplied.len() != 1
            || supplied[0] != &descriptor
            || self.review_pages.is_some()
        {
            return Err(Error::new(
                "worker_binding",
                "review archive differs from the admitted context",
            ));
        }
        let history = serde_json::to_vec(&json!({"validation":archive.evidence(),"validation_attempt_references":archive.evidence().attempt_references()?}))
            .map_err(|_| Error::invalid("encode review history"))?;
        let proofs = archive
            .artifact_index()
            .into_iter()
            .map(|(digest, proof)| (digest.to_owned(), proof.clone()))
            .collect::<BTreeMap<_, _>>();
        let index = serde_json::to_vec(
            &proofs
                .iter()
                .map(|(digest, proof)| json!({"proof_digest":digest,"proof":proof}))
                .collect::<Vec<_>>(),
        )
        .map_err(|_| Error::invalid("encode review artifact index"))?;
        // Check complete original bodies before splitting: a protected value
        // straddling two pages must not evade the existing secret boundary.
        for bytes in std::iter::once(history.as_slice())
            .chain(std::iter::once(index.as_slice()))
            .chain(
                proofs
                    .values()
                    .map(|proof| archive.artifact_bytes(proof))
                    .collect::<Result<Vec<_>>>()?,
            )
        {
            if self
                .known_secrets
                .iter()
                .filter(|secret| !secret.is_empty())
                .any(|secret| {
                    bytes
                        .windows(secret.len())
                        .any(|window| window == secret.as_bytes())
                })
            {
                return Err(Error::new(
                    "access_denied",
                    "required review input contains a protected value",
                ));
            }
        }
        self.review_pages = Some(ReviewPages {
            archive,
            history_digest: bytes_digest(&history),
            index_digest: bytes_digest(&index),
            history: Arc::new(history),
            index: Arc::new(index),
            proofs,
        });
        Ok(())
    }
}

impl ReviewPages {
    fn body(&self, resource: &str) -> Result<(&[u8], String)> {
        match resource {
            "history" => Ok((&self.history, self.history_digest.clone())),
            "index" => Ok((&self.index, self.index_digest.clone())),
            _ => {
                let proof = resource
                    .strip_prefix("artifact:")
                    .and_then(|digest| self.proofs.get(digest))
                    .ok_or_else(|| {
                        Error::new(
                            "missing_context",
                            "review resource is outside the authenticated archive",
                        )
                    })?;
                Ok((self.archive.artifact_bytes(proof)?, proof.digest.clone()))
            }
        }
    }
    fn complete(&self, reads: &PageReads) -> bool {
        reads.0.get("history") == Some(&self.history.len())
            && reads.0.get("index") == Some(&self.index.len())
            && self.proofs.iter().all(|(digest, proof)| {
                reads
                    .0
                    .get(&format!("artifact:{digest}"))
                    .is_some_and(|bytes| *bytes as u64 == proof.bytes)
            })
    }
}

impl ScopedWorker {
    pub(super) fn review_context_read(&mut self, args: &ContextRead) -> Result<Value> {
        let pages = self.contract.review_pages.as_ref().ok_or_else(|| {
            Error::new(
                "missing_context",
                "no authenticated review archive is attached",
            )
        })?;
        let Some(resource) = args.resource.as_deref() else {
            if args.offset.is_some() || args.archive_digest.is_some() {
                return Err(Error::invalid(
                    "review page requires a resource, offset and archive_digest",
                ));
            }
            return bounded_value(pages.archive.review_descriptor()?);
        };
        if args.archive_digest.as_deref()
            != Some(pages.archive.reference().manifest_digest.as_str())
        {
            return Err(Error::new(
                "stale_context",
                "review page belongs to another archive",
            ));
        }
        let offset = args
            .offset
            .ok_or_else(|| Error::invalid("review page offset is required"))?;
        let (body, digest) = pages.body(resource)?;
        let observed = self.review_reads.0.get(resource).copied().unwrap_or(0);
        let (page, end) = page(body, offset, observed)?;
        let response = bounded_value(json!({
            "schema_version":1,"archive_digest":pages.archive.reference().manifest_digest,
            "evidence_digest":pages.archive.reference().evidence_digest,"resource":resource,
            "body_digest":digest,"total_bytes":body.len(),"offset":offset,
            "next_offset":if end == body.len() {None} else {Some(end)},
            "page_digest":bytes_digest(&body[offset..end]),"page":page
        }))?;
        self.review_reads
            .0
            .insert(resource.to_owned(), observed.max(end));
        Ok(response)
    }

    pub(super) fn verify_review_page_coverage(&self, claim: &Value) -> Result<()> {
        let passing = claim.get("verdict").and_then(Value::as_str) == Some("pass")
            || claim
                .pointer("/validation/sufficient")
                .and_then(Value::as_bool)
                == Some(true);
        if passing
            && self
                .contract
                .review_pages
                .as_ref()
                .is_some_and(|pages| !pages.complete(&self.review_reads))
        {
            return Err(Error::new(
                "review_context_incomplete",
                "read the complete validation history, artifact index and every original artifact page before a satisfactory assessment",
            ));
        }
        Ok(())
    }
}

fn page(body: &[u8], offset: usize, observed: usize) -> Result<(Value, usize)> {
    use base64::Engine;
    if offset > body.len() || offset > observed || offset == body.len() && !body.is_empty() {
        return Err(Error::new(
            "invalid_page",
            "read review resources from offset zero and follow next_offset; no bytes may be skipped",
        ));
    }
    let mut end = offset.saturating_add(PAGE_BYTES).min(body.len());
    let text = match std::str::from_utf8(&body[offset..end]) {
        Ok(text) => Some(text),
        Err(error) if error.error_len().is_none() && error.valid_up_to() > 0 => {
            end = offset + error.valid_up_to();
            std::str::from_utf8(&body[offset..end]).ok()
        }
        Err(_) => None,
    }
    .filter(|text| {
        !text
            .chars()
            .any(|ch| ch.is_control() && !matches!(ch, '\n' | '\r' | '\t'))
    });
    let value = match text {
        Some(text) => {
            json!({"encoding":"utf8","data":text})
        }
        None => {
            json!({"encoding":"base64","data":base64::engine::general_purpose::STANDARD.encode(&body[offset..end])})
        }
    };
    Ok((value, end))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounded_pages_preserve_utf8_binary_and_refuse_skipped_bytes() {
        use base64::Engine;
        let bodies = [
            "a€".repeat(PAGE_BYTES).into_bytes(),
            (0..PAGE_BYTES * 3).map(|i| i as u8).collect(),
        ];
        for body in bodies {
            let mut offset = 0;
            let mut joined = Vec::new();
            assert_eq!(page(&body, 1, 0).unwrap_err().code, "invalid_page");
            loop {
                let (value, next) = page(&body, offset, offset).unwrap();
                assert!(serde_json::to_vec(&value).unwrap().len() < MAX_TOOL_BYTES);
                if value["encoding"] == "utf8" {
                    joined.extend_from_slice(value["data"].as_str().unwrap().as_bytes());
                } else {
                    joined.extend(
                        base64::engine::general_purpose::STANDARD
                            .decode(value["data"].as_str().unwrap())
                            .unwrap(),
                    );
                }
                if next == body.len() {
                    break;
                }
                offset = next;
            }
            assert_eq!(joined, body);
        }
    }
}
