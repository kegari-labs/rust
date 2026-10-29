//! Legacy custom-command bundle records are retained, never resumed as Codex work.
use super::*;
pub(super) const PLAN_KEY: &str = "check_bundle";
pub(super) fn record_never_started(_: &mut Receipt) -> Result<()> {
    Err(Error::new(
        "recovery_incompatible",
        "legacy command bundle cannot be rewritten by the Codex execution contract",
    ))
}
impl ControlHost {
    pub(crate) fn verify_check_bundle_inventory(
        &self,
        receipts: &[Receipt],
        bindings: &[kegari_execution::journal::SpawnBinding],
    ) -> Result<()> {
        if receipts
            .iter()
            .any(|receipt| receipt.expected.contains_key(PLAN_KEY))
            || bindings
                .iter()
                .any(|binding| binding.check_bundle.is_some())
        {
            return Err(Error::new(
                "recovery_incompatible",
                "retained custom-command bundle requires supervised reconciliation; it is not Codex execution evidence",
            ));
        }
        Ok(())
    }
}
