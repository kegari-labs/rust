//! Grader data for the shipped schema-1 review outcome. These serializable
//! observations are claims, never delivery, check, publication or admission proofs.
use super::{FindingExpectation, ObservedFacts, PreparedFixture, bounded_read, relative};
use crate::{
    delivery::ReviewResultClaim,
    evidence::{Verdict, validate_digest},
};
use kegari_types::error::{Error, Result};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Host-known repository identity. A missing slug is unknown, not inferred from
/// reviewer links or arbitrary context fields.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewSourceRepository {
    pub hostname: String,
    pub id: String,
    pub slug: Option<String>,
}
impl ReviewSourceRepository {
    fn validate(&self) -> Result<()> {
        crate::git::RepositoryId {
            hostname: self.hostname.clone(),
            id: self.id.clone(),
        }
        .validate()?;
        if self.slug.as_ref().is_some_and(|slug| {
            let parts: Vec<_> = slug.split('/').collect();
            parts.len() != 2
                || parts.iter().any(|part| {
                    part.is_empty()
                        || matches!(*part, "." | "..")
                        || !part
                            .bytes()
                            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_' | b'.'))
                })
        }) {
            return Err(Error::new(
                "invalid_outcome",
                "review repository slug is malformed",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservedReviewClaim {
    pub schema_version: u32,
    pub operation: Uuid,
    pub proposal: Uuid,
    pub thread: String,
    pub completed_turn: String,
    /// Bound by the host's admitted manifest, never supplied by the reviewer.
    pub source_revision: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repository: Option<ReviewSourceRepository>,
    pub instruction_digest: String,
    pub tool_contract_digest: String,
    /// Exact outcome sink reference after the proposal's completed-turn record.
    pub outcome_reference: String,
    pub claim: ReviewResultClaim,
}
impl ObservedReviewClaim {
    pub(crate) fn validate(&self) -> Result<()> {
        if let Some(repository) = &self.repository {
            repository.validate()?;
        }
        if self.schema_version != 1
            || self.operation.is_nil()
            || self.proposal.is_nil()
            || self.source_revision.len() != 40
            || !self
                .source_revision
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
            || [&self.thread, &self.completed_turn, &self.outcome_reference]
                .into_iter()
                .any(|value| value.is_empty() || value.len() > 65536)
            || validate_digest(&self.instruction_digest).is_err()
            || validate_digest(&self.tool_contract_digest).is_err()
            || self.claim.schema_version != 1
            || self.claim.unresolved_findings.len() > 256
            || self.claim.supporting_links.len() > 256
            || self
                .claim
                .unresolved_findings
                .iter()
                .chain(&self.claim.supporting_links)
                .any(|value| value.trim().is_empty() || value.len() > 65536)
            || self.claim.verdict == Verdict::Pass && !self.claim.unresolved_findings.is_empty()
        {
            return Err(Error::new(
                "invalid_outcome",
                "recorded review claim has an unsupported shape or binding",
            ));
        }
        Ok(())
    }
}

pub(super) fn validate_expectations(expected: &[FindingExpectation]) -> Result<()> {
    let mut locations = std::collections::BTreeSet::new();
    for finding in expected {
        relative(&finding.path)?;
        if finding.line == 0
            || finding.source_contains.is_empty()
            || finding.concept_groups.is_empty()
            || finding
                .concept_groups
                .iter()
                .any(|group| group.is_empty() || group.iter().any(|word| word.trim().is_empty()))
            || !locations.insert((&finding.path, finding.line))
        {
            return Err(Error::invalid(
                "review rubric requires unique exact source locations and explicit supported concepts",
            ));
        }
    }
    Ok(())
}

/// The host binds the admitted revision; prose may use short SHAs or links.
/// The rubric does not prescribe a line-number encoding or an answer object.
/// Each expected source line is checked independently; prose matching remains an
/// explicit qualitative rubric candidate requiring the normal human calibration.
pub(super) fn matches_review(
    fixture: &PreparedFixture,
    facts: &ObservedFacts,
    verdict: &Verdict,
    expected: &[FindingExpectation],
) -> bool {
    let Some(review) = &facts.review_claim else {
        return false;
    };
    if review.validate().is_err()
        || fixture
            .worker_manifest
            .as_ref()
            .is_some_and(|digest| digest != &review.instruction_digest)
        || review.source_revision != facts.revision
        || !facts
            .evidence_references
            .contains(&review.outcome_reference)
        || &review.claim.verdict != verdict
        || review.claim.unresolved_findings.len() != expected.len()
    {
        return false;
    }
    let mut used = std::collections::BTreeSet::new();
    for expectation in expected {
        let source_matches = relative(&expectation.path)
            .ok()
            .and_then(|path| bounded_read(&fixture.repository.join(path)).ok())
            .and_then(|bytes| String::from_utf8(bytes).ok())
            .is_some_and(|text| {
                text.lines()
                    .nth(expectation.line as usize - 1)
                    .is_some_and(|line| line.contains(&expectation.source_contains))
            });
        if !source_matches {
            return false;
        }
        let candidates: Vec<_> = review
            .claim
            .unresolved_findings
            .iter()
            .enumerate()
            .filter(|(_, finding)| {
                let prose = finding.to_lowercase();
                (contains_reference(finding, &expectation.path)
                    || contains_blob_reference(
                        finding,
                        &expectation.path,
                        expectation.line,
                        &review.source_revision,
                        review.repository.as_ref(),
                    ))
                    && expectation.concept_groups.iter().all(|alternatives| {
                        alternatives
                            .iter()
                            .any(|word| prose.contains(&word.to_lowercase()))
                    })
            })
            .map(|(index, _)| index)
            .collect();
        if candidates.len() != 1 || !used.insert(candidates[0]) {
            return false;
        }
    }
    used.len() == review.claim.unresolved_findings.len()
}

fn contains_reference(text: &str, reference: &str) -> bool {
    fn reference_char(character: char) -> bool {
        character.is_alphanumeric() || matches!(character, '_' | '-' | '.' | '/' | '\\')
    }
    text.match_indices(reference).any(|(offset, _)| {
        text[..offset]
            .chars()
            .next_back()
            .is_none_or(|c| !reference_char(c))
            && text[offset + reference.len()..]
                .chars()
                .next()
                .is_none_or(|c| !reference_char(c))
    })
}

// This recognizes a precise GitHub-shaped source reference; it never fetches a
// URL or establishes repository access/publication. Unknown repository metadata
// permits only github.com lexical candidates, still requiring human review.
fn contains_blob_reference(
    text: &str,
    path: &str,
    line: u32,
    revision: &str,
    repository: Option<&ReviewSourceRepository>,
) -> bool {
    text.split(|c: char| {
        c.is_whitespace() || matches!(c, '<' | '>' | '(' | ')' | '[' | ']' | '\'' | '"' | '`')
    })
    .filter(|token| token.starts_with("https://"))
    .any(|token| {
        let Ok(url) = url::Url::parse(token.trim_end_matches(['.', ',', ';', ':'])) else {
            return false;
        };
        if url.scheme() != "https"
            || !url.username().is_empty()
            || url.password().is_some()
            || url.port().is_some()
            || url.query().is_some()
            || url.host_str() != Some(repository.map_or("github.com", |r| r.hostname.as_str()))
        {
            return false;
        }
        let Some(parts) = url
            .path_segments()
            .and_then(|segments| segments.map(decode_segment).collect::<Option<Vec<_>>>())
        else {
            return false;
        };
        if parts.len() < 5
            || parts[0].is_empty()
            || parts[1].is_empty()
            || parts[2] != "blob"
            || repository
                .and_then(|r| r.slug.as_ref())
                .is_some_and(|slug| slug != &format!("{}/{}", parts[0], parts[1]))
            || !(7..=40).contains(&parts[3].len())
            || !parts[3].bytes().all(|c| c.is_ascii_hexdigit())
            || !revision
                .to_ascii_lowercase()
                .starts_with(&parts[3].to_ascii_lowercase())
            || parts[4..].join("/") != path
        {
            return false;
        }
        // Optional GitHub line anchors must actually include the declared
        // defect line. Absence does not force a model-specific line format.
        url.fragment().is_none_or(|fragment| {
            let Some(lines) = fragment.strip_prefix('L') else {
                return false;
            };
            let (start, end) = lines.split_once("-L").unwrap_or((lines, lines));
            match (start.parse::<u32>(), end.parse::<u32>()) {
                (Ok(start), Ok(end)) => start > 0 && start <= line && line <= end,
                _ => false,
            }
        })
    })
}
fn decode_segment(segment: &str) -> Option<String> {
    let mut bytes = Vec::with_capacity(segment.len());
    let input = segment.as_bytes();
    let mut index = 0;
    while index < input.len() {
        if input[index] == b'%' {
            let pair = input.get(index + 1..index + 3)?;
            let high = (pair[0] as char).to_digit(16)?;
            let low = (pair[1] as char).to_digit(16)?;
            bytes.push((high * 16 + low) as u8);
            index += 3;
        } else {
            bytes.push(input[index]);
            index += 1;
        }
    }
    let value = String::from_utf8(bytes).ok()?;
    (!value.contains(['/', '\\']) && !value.chars().any(char::is_control)).then_some(value)
}

#[cfg(test)]
mod tests {
    use super::contains_reference;
    #[test]
    fn source_references_are_exact_tokens_not_unrelated_path_suffixes() {
        assert!(contains_reference("`src/limit.py`:2", "src/limit.py"));
        assert!(contains_reference(
            "src/limit.py at revision a123",
            "src/limit.py"
        ));
        for prose in ["other/src/limit.py", "src/limit.py.old", "xsrc/limit.py"] {
            assert!(!contains_reference(prose, "src/limit.py"));
        }
    }
}
