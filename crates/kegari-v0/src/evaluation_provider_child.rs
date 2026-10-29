//! Captured, finite synthetic Linear issue facts for fresh child evaluations.
//! These DTOs pass through the normal provider adapter and reconciliation gates.
use super::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ChildIssues {
    pub schema_version: u32,
    pub issues: BTreeMap<Uuid, ChildIssue>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ChildIssue {
    pub issue: Value,
    pub labels: Vec<Value>,
    pub history: Vec<Value>,
}
impl ChildIssues {
    pub(crate) fn validate(&self, binding: &EvaluationBinding) -> Result<()> {
        let subject = binding
            .child
            .as_ref()
            .ok_or_else(|| Error::invalid("child fixture requires a version-2 declaration"))?;
        if self.schema_version != 1
            || self.issues.len() < 2
            || self.issues.len() > 32
            || !self.issues.contains_key(&subject.parent_id)
            || !self.issues.contains_key(&subject.child_id)
            || serde_json::to_vec(self)
                .map_err(|_| Error::invalid("child fixture encoding failed"))?
                .len()
                > 1024 * 1024
        {
            return Err(Error::new(
                "evaluation_fidelity",
                "child fixture must declare its bounded complete issue hierarchy",
            ));
        }
        for (id, value) in &self.issues {
            let identity: Uuid = serde_json::from_value(value.issue["id"].clone())
                .map_err(|_| Error::invalid("declared issue identity is invalid"))?;
            let parent: Option<Uuid> = value
                .issue
                .pointer("/parent/id")
                .map(|v| serde_json::from_value(v.clone()))
                .transpose()
                .map_err(|_| Error::invalid("declared parent identity is invalid"))?;
            if id.is_nil()
                || identity != *id
                || value.labels.len() > 64
                || value.history.len() > 128
                || (*id == subject.parent_id && parent.is_some())
                || (*id != subject.parent_id && parent != Some(subject.parent_id))
            {
                return Err(Error::new(
                    "evaluation_fidelity",
                    "declared issue is outside the exact fresh-child hierarchy",
                ));
            }
            let description = value.issue["description"]
                .as_str()
                .ok_or_else(|| Error::invalid("declared issue lacks its literal description"))?;
            if *id == subject.parent_id {
                let metadata: crate::authority::ParentMetadata =
                    crate::authority::metadata(description)?;
                let required: BTreeSet<_> = metadata.required_children.iter().copied().collect();
                if required
                    != self
                        .issues
                        .keys()
                        .filter(|id| **id != subject.parent_id)
                        .copied()
                        .collect()
                    || metadata.schema_version != 1
                    || metadata.kind != "parent"
                    || !required.contains(&metadata.e2e_child)
                    || !metadata.production_targets.is_empty()
                {
                    return Err(Error::new(
                        "evaluation_fidelity",
                        "fresh evaluation parent must declare every child and no production target",
                    ));
                }
            } else {
                let scope: crate::authority::ChildScope = crate::authority::metadata(description)?;
                scope.validate(
                    &BTreeSet::from([binding.repository_id.to_string()]),
                    &BTreeSet::new(),
                )?;
            }
        }
        Ok(())
    }
}

fn connection(nodes: Vec<Value>) -> Value {
    json!({"nodes":nodes,"pageInfo":{"hasNextPage":false,"endCursor":null}})
}
pub(super) fn route(
    d: &Declaration,
    active: &mut Active,
    body: &Value,
    query: &str,
) -> Result<Value> {
    let issues = active.child_issues.as_ref().ok_or_else(|| {
        Error::new(
            "evaluation_unavailable",
            "native child fixture is not active",
        )
    })?;
    issues.validate(&d.binding)?;
    let subject = d.binding.child.as_ref().unwrap();
    let requested = body
        .pointer("/variables/id")
        .and_then(Value::as_str)
        .and_then(|id| Uuid::parse_str(id).ok());
    let selected = || {
        requested
            .and_then(|id| issues.issues.get(&id))
            .ok_or_else(|| {
                Error::new(
                    "scope_violation",
                    "requested issue is outside the active evaluation hierarchy",
                )
            })
    };
    let data = if query.starts_with("query KegariParents(") {
        json!({"issues":connection(vec![issues.issues[&subject.parent_id].issue.clone()])})
    } else if query.starts_with("query KegariRefresh(") {
        let ids: Vec<Uuid> =
            serde_json::from_value(body["variables"]["filter"]["id"]["in"].clone())
                .map_err(|_| Error::invalid("invalid bounded evaluation refresh"))?;
        if ids.len() > 100 {
            return denied();
        }
        json!({"issues":connection(ids.iter().filter_map(|id|issues.issues.get(id).map(|v|v.issue.clone())).collect())})
    } else if query.starts_with("query KegariIssue(") {
        json!({"issue":selected()?.issue})
    } else if query.starts_with("query KegariChildren(") {
        selected()?;
        json!({"issue":{"children":connection(if requested == Some(subject.parent_id) {issues.issues.iter().filter(|(id,_)|**id != subject.parent_id).map(|(_,v)|v.issue.clone()).collect()} else {vec![]})}})
    } else if query.starts_with("query KegariLabels(") {
        json!({"issue":{"labels":connection(selected()?.labels.clone())}})
    } else if query.starts_with("query KegariHistory(") {
        json!({"issue":{"history":connection(selected()?.history.clone())}})
    } else if query.starts_with("query KegariComments(") {
        selected()?;
        json!({"issue":{"comments":connection(active.linear_comments.get(&requested.unwrap()).cloned().unwrap_or_default())}})
    } else if query.starts_with("query KegariDependencies(") {
        selected()?;
        let field = if query.contains("inverseRelations(") {
            "inverseRelations"
        } else {
            "relations"
        };
        json!({"issue":{field:connection(vec![])}})
    } else if query.starts_with("mutation KegariComment(") {
        let input = &body["variables"]["input"];
        let issue: Uuid = serde_json::from_value(input["issueId"].clone())
            .map_err(|_| Error::invalid("invalid evaluation publication target"))?;
        let text = input["body"]
            .as_str()
            .ok_or_else(|| Error::invalid("evaluation publication body missing"))?;
        if issue != subject.parent_id
            || text.len() > 1024 * 1024
            || !text.starts_with("<!-- kegari:v0:parent_authorization -->\n```json\n")
            || !text.ends_with("\n```")
            || active.linear_comments.values().map(Vec::len).sum::<usize>() >= 4
        {
            return denied();
        }
        let digest = canonical_digest(1, &(active.operation, issue, text))?;
        let bytes = hex::decode(digest.rsplit(':').next().unwrap())
            .map_err(|_| Error::invalid("invalid publication digest"))?;
        let id = Uuid::from_slice(&bytes[..16])
            .map_err(|_| Error::invalid("invalid publication identity"))?;
        let value = json!({"id":id,"body":text,"createdAt":chrono::Utc::now(),"updatedAt":chrono::Utc::now(),"editedAt":null,"url":format!("https://linear.{}.evaluation.invalid/comment/{id}",d.workspace),"user":{"id":d.linear_actor},"onBehalfOf":null,"botActor":null,"issue":{"id":issue}});
        active.linear_comments.entry(issue).or_default().push(value);
        json!({"commentCreate":{"success":true,"comment":{"id":id,"issue":{"id":issue},"user":{"id":d.linear_actor}}}})
    } else {
        return denied();
    };
    Ok(json!({"data":data}))
}
