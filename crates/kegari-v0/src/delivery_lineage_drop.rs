//! Iterative release of uniquely owned retained-capability edges.
//!
//! Jobs contain actual Arcs; they cannot construct a proof. Arc::into_inner
//! atomically consumes one strong reference and yields the value only to the
//! unique final owner. A shared tail stays owned by its other real references.
//! Every Release implementation detaches children before its leaf fields drop.
use std::sync::Arc;

pub(super) trait Release: Sized + 'static {
    fn release(self, pending: &mut Pending);
}
trait Job {
    fn release(self: Box<Self>, pending: &mut Pending);
}
struct ArcJob<T: Release>(Arc<T>);
impl<T: Release> Job for ArcJob<T> {
    fn release(self: Box<Self>, pending: &mut Pending) {
        if let Some(value) = Arc::into_inner(self.0) {
            value.release(pending);
        }
    }
}
#[derive(Default)]
pub(super) struct Pending {
    jobs: Vec<Box<dyn Job>>,
}
impl Pending {
    pub fn push<T: Release>(&mut self, value: Arc<T>) {
        self.jobs.push(Box::new(ArcJob(value)));
    }
}

// Destruction cannot return a resource/budget error or silently abandon a tail.
// Work is finite in the actual acyclic owned graph: a value is extracted once,
// and each of its strong child edges is consumed once. No native stack recursion
// or thread-local deferred ownership is needed.
pub(super) fn drain(detach: impl FnOnce(&mut Pending)) {
    let mut pending = Pending::default();
    detach(&mut pending);
    while let Some(job) = pending.jobs.pop() {
        job.release(&mut pending);
    }
}

#[cfg(test)]
#[path = "delivery_lineage_drop_tests.rs"]
mod tests;
