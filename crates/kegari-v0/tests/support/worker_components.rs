// Shared injected workflow host and outcome sink.
// Included by the complete worker fixture and the actual delivery-assignment test.
fn git_path() -> PathBuf {
    std::env::split_paths(&std::env::var_os("PATH").unwrap())
        .map(|p| p.join("git"))
        .find(|p| p.is_file())
        .unwrap()
        .canonicalize()
        .unwrap()
}
fn git_run(path: &Path, args: &[&str]) {
    let r = kegari_process::group::OwnedGroup::spawn_std(
        Command::new(git_path())
            .current_dir(path)
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_AUTHOR_NAME", "Disposable Fixture Human")
            .env("GIT_AUTHOR_EMAIL", "fixture@example.test")
            .env("GIT_COMMITTER_NAME", "Disposable Fixture Human")
            .env("GIT_COMMITTER_EMAIL", "fixture@example.test")
            .args([
                "-c",
                "core.hooksPath=/dev/null",
                "-c",
                "commit.gpgSign=false",
            ])
            .args(args)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped()),
    )
    .expect("spawn owned disposable fixture")
    .0
    .wait_with_output()
    .unwrap();
    assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
}
struct Host {
    authority: Mutex<WorkerAuthority>,
    revoked: AtomicBool,
    owner: AtomicBool,
    rates: AtomicBool,
    waiting: AtomicBool,
}
impl WorkerHost for Host {
    fn observed_session(&self, _: Uuid, _: &SessionState) -> Result<()> {
        Ok(())
    }
    fn current_authority(&self, _: &WorkerAuthority) -> Result<WorkerAuthority> {
        if self.revoked.load(Ordering::SeqCst) {
            return Err(Error::new("access_denied", "fixture revoked authorization"));
        }
        if self.waiting.load(Ordering::SeqCst) {
            return Err(Error::new(
                "correction_classification_pending",
                "fixture native input pending",
            ));
        }
        Ok(self.authority.lock().unwrap().clone())
    }
    fn correction_waiting(&self, _: Uuid) -> Result<bool> {
        Ok(self.waiting.load(Ordering::SeqCst))
    }
    fn verify_owner(&self, _: &Lease) -> Result<()> {
        if !self.owner.load(Ordering::SeqCst) {
            return Err(Error::new("ownership_conflict", "fixture owner exited"));
        }
        Ok(())
    }
    fn rate_limits(&self, _: Uuid, _: &Value) -> Result<()> {
        self.rates.store(true, Ordering::SeqCst);
        Ok(())
    }
    fn reconcile_prior(&self, _: Uuid, prior: Option<&Value>) -> Result<()> {
        if prior.is_some_and(|p| p["status"] == "unknown") {
            Err(Error::new(
                "reconciliation_required",
                "unknown provider effect",
            ))
        } else {
            Ok(())
        }
    }
    fn observed_state(&self, _: Uuid) -> Result<Value> {
        Ok(json!({"delivery":"pending"}))
    }
    fn correction_event(&self, _: Uuid, watermark: u64, _: &str) -> Result<String> {
        Ok(format!("fixture-correction:{watermark}"))
    }
}
#[derive(Default)]
struct Sink(Mutex<Vec<Value>>);
impl OutcomeSink for Sink {
    fn record(&self, outcome: &WorkerOutcome) -> Result<String> {
        self.0
            .lock()
            .unwrap()
            .push(serde_json::to_value(outcome).unwrap());
        Ok(format!("pending:{}", outcome.proposal_id))
    }
}
