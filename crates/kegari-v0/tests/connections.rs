//! Credential transaction tests use a labeled wire and secret-store fixture;
//! they do not contact accounts or invoke the native Keychain adapter.
use kegari_v0::{
    config::Connection,
    connections::verify_credential,
    error::{Error, Result},
    providers::{HttpRequest, HttpResponse, HttpTransport},
    secrets::{Secret, SecretStore},
};
use serde_json::json;
use std::{
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex},
    time::Duration,
};

struct IdentityWire {
    account: u64,
}
impl HttpTransport for IdentityWire {
    fn send(
        &self,
        request: HttpRequest,
    ) -> Pin<Box<dyn Future<Output = Result<HttpResponse>> + Send + '_>> {
        Box::pin(async move {
            assert_eq!(request.url.as_str(), "https://api.github.com/user");
            assert_eq!(request.headers["authorization"], "Bearer sentinel-secret");
            Ok(HttpResponse {
                status: 200,
                headers: Default::default(),
                body: serde_json::to_vec(
                    &json!({"id":self.account,"login":"fixture-user","type":"User"}),
                )
                .unwrap(),
            })
        })
    }
}
#[derive(Default)]
struct Store {
    writes: Mutex<Vec<(String, Vec<u8>)>>,
    fail: bool,
}
impl SecretStore for Store {
    fn read(&self, _: &str) -> Result<Secret> {
        Err(Error::new("missing_secret", "fixture absent"))
    }
    fn save(&self, name: &str, secret: &Secret) -> Result<()> {
        if self.fail {
            return Err(Error::new("authentication_required", "fixture locked"));
        }
        self.writes
            .lock()
            .unwrap()
            .push((name.into(), secret.bytes().to_vec()));
        Ok(())
    }
}
fn connection() -> Connection {
    serde_json::from_value(json!({"kind":"github","account_id":"17","endpoint":"https://api.github.com/","credential_ref":"$GITHUB_INPUT"})).unwrap()
}
async fn verified(
    connection: &Connection,
    account: u64,
) -> Result<kegari_v0::connections::VerifiedCredential> {
    verify_credential(
        connection,
        Secret::new(b"sentinel-secret".to_vec()).unwrap(),
        Duration::from_secs(1),
        Some(Arc::new(IdentityWire { account })),
    )
    .await
}

#[tokio::test]
async fn credential_is_imported_only_after_exact_identity_and_persists_reference() {
    let mut configured = connection();
    let store = Store::default();
    assert!(verified(&configured, 18).await.is_err());
    assert!(store.writes.lock().unwrap().is_empty());
    let proof = verified(&configured, 17).await.unwrap();
    assert_eq!(proof.identity().account_id, "17");
    proof
        .import(&mut configured, "github-user", &store)
        .unwrap();
    assert_eq!(
        configured.credential_ref.as_deref(),
        Some("keychain:github-user")
    );
    assert_eq!(store.writes.lock().unwrap().len(), 1);
    assert!(
        !toml::to_string(&configured)
            .unwrap()
            .contains("sentinel-secret")
    );
}

#[tokio::test]
async fn changed_account_or_failed_keychain_does_not_commit_configuration() {
    let mut configured = connection();
    let store = Store::default();
    let proof = verified(&configured, 17).await.unwrap();
    configured.account_id = "18".into();
    assert!(
        proof
            .import(&mut configured, "github-user", &store)
            .is_err()
    );
    assert!(store.writes.lock().unwrap().is_empty());
    let mut configured = connection();
    let before = toml::to_string(&configured).unwrap();
    let proof = verified(&configured, 17).await.unwrap();
    assert!(
        proof
            .import(
                &mut configured,
                "github-user",
                &Store {
                    fail: true,
                    ..Default::default()
                }
            )
            .is_err()
    );
    assert_eq!(toml::to_string(&configured).unwrap(), before);
}
