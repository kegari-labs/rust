//! Explicit synthetic upstream PR assignment; real scoped changed source is
//! verified separately and the qualified local-check barrier remains asserted.
use super::*;
use kegari_providers::HttpTransport;
use kegari_providers::{HttpRequest, HttpResponse};
use std::sync::Mutex;
pub(super) struct ReviewWire {
    revision: RevisionBinding,
    state: Mutex<(Vec<Value>, Vec<Value>)>,
}
impl ReviewWire {
    pub(super) fn new(revision: RevisionBinding) -> Self {
        Self {
            revision,
            state: Mutex::new((Vec::new(), Vec::new())),
        }
    }
}
impl HttpTransport for ReviewWire {
    fn send(
        &self,
        request: HttpRequest,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<HttpResponse>> + Send + '_>>
    {
        Box::pin(async move {
            let path = request.url.path();
            let post = request.method == kegari_providers::Method::Post;
            let body: Value = if request.body.is_empty() {
                Value::Null
            } else {
                serde_json::from_slice(&request.body).unwrap()
            };
            let mut state = self.state.lock().unwrap();
            let repo = json!({"id":42,"owner":{"id":7,"login":"fixture","type":"User"},"name":"repo","default_branch":"main","private":true,"archived":false,"disabled":false,"allow_squash_merge":true,"permissions":{"pull":true,"push":true}});
            let value = if path == "/user" {
                json!({"id":7,"login":"fixture","type":"User"})
            } else if path == "/repos/fixture/repo" {
                repo.clone()
            } else if path == "/user/repos" {
                json!([repo])
            } else if path == "/repos/fixture/repo/pulls" {
                json!([])
            } else if path.ends_with("/pulls/1") {
                json!({"id":1,"number":1,"state":"open","draft":false,"merged":false,"user":{"id":7,"login":"fixture"},"head":{"sha":self.revision.head,"ref":"fixture-repair","repo":{"id":42}},"base":{"sha":self.revision.base,"ref":"staging","repo":{"id":42}},"html_url":"https://github.invalid/fixture/repo/pull/1"})
            } else if path.contains("/git/commits/") {
                json!({"sha":self.revision.head,"tree":{"sha":self.revision.tested_tree},"parents":[{"sha":self.revision.base}]})
            } else if path.ends_with("/issues/1/comments") {
                if post {
                    let item = json!({"id":601,"body":body["body"],"user":{"id":7},"created_at":Utc::now(),"updated_at":Utc::now(),"html_url":"https://github.invalid/comment/601"});
                    state.0.push(item.clone());
                    item
                } else {
                    json!(state.0)
                }
            } else if path.contains("/statuses/") && post {
                let mut item = body;
                item["id"] = json!(602);
                item["creator"] = json!({"id":7});
                item["created_at"] = json!(Utc::now());
                state.1.push(item.clone());
                item
            } else if path.ends_with("/statuses") {
                json!(state.1)
            } else {
                panic!(
                    "unexpected downstream review fixture request {:?} {}",
                    request.method, path
                )
            };
            Ok(HttpResponse {
                status: if post { 201 } else { 200 },
                headers: Default::default(),
                body: serde_json::to_vec(&value).unwrap(),
            })
        })
    }
}
