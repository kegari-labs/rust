//! Bounded traversal of the actual retained capability DAG. Keys are process-local
//! object identities, never serialized authority. Expanding a node must only add
//! its immediate children and flat leaves; it must not start another traversal.
use super::*;
use crate::{
    decisions::VerifiedRetryGrant,
    worker_run::{AdmittedSessionResume, RunCompletion},
};

const MAX_NODES: usize = 65_536;
// Each current node has at most eight direct edges and four classes of leaves.
// Allow sixteen work items per protected-history entry, independent of phase caps.
const MAX_WORK: usize = MAX_NODES * 16;

#[derive(Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
pub(super) struct Key {
    kind: &'static str,
    address: usize,
}

impl Key {
    pub fn of<T: ?Sized>(value: &T) -> Self {
        Self {
            kind: std::any::type_name::<T>(),
            address: std::ptr::from_ref(value).cast::<()>() as usize,
        }
    }
}

pub(super) trait Node {
    fn key(&self) -> Key {
        Key::of(self)
    }
    fn expand<'a>(&'a self, children: &mut Vec<&'a dyn Node>, leaves: &mut Inventory)
    -> Result<()>;
    // Local checks only: descendants are checked by this same worklist.
    fn verify_joined_leaf(
        &self,
        _reader: &dyn Fn(Uuid) -> Result<Option<Arc<RunCompletion>>>,
    ) -> Result<()> {
        Ok(())
    }
}

#[derive(Default)]
pub(super) struct Inventory {
    pub bridges: BTreeMap<Uuid, Arc<AdmittedSessionResume>>,
    pub grants: BTreeMap<Uuid, VerifiedRetryGrant>,
    pub sources: BTreeMap<String, DeliverySource>,
    scanned: usize,
    pub legacy_grant_missing: bool,
}
impl Inventory {
    pub fn into_grants(self) -> Result<BTreeMap<Uuid, VerifiedRetryGrant>> {
        if self.legacy_grant_missing {
            return Err(Error::new(
                "review_round_lineage",
                "legacy review fixture cannot supply native grant closure",
            ));
        }
        Ok(self.grants)
    }
    fn charge(&mut self, count: usize) -> Result<()> {
        self.scanned = self.scanned.checked_add(count).ok_or_else(limit)?;
        if self.scanned > MAX_WORK {
            return Err(limit());
        }
        Ok(())
    }
    pub fn sources(&mut self, sources: impl IntoIterator<Item = DeliverySource>) -> Result<()> {
        for source in sources {
            self.charge(1)?;
            review_round::merge_source_inventory(&mut self.sources, vec![source])?;
        }
        Ok(())
    }
    pub fn grants(&mut self, grants: impl IntoIterator<Item = VerifiedRetryGrant>) -> Result<()> {
        for grant in grants {
            self.charge(1)?;
            review_round::merge_native_grants(&mut self.grants, [grant])?;
        }
        Ok(())
    }
    pub fn bridges(&mut self, bridges: &BTreeMap<Uuid, Arc<AdmittedSessionResume>>) -> Result<()> {
        for (id, bridge) in bridges {
            self.charge(1)?;
            review_round::merge_resumed_bridges(
                &mut self.bridges,
                BTreeMap::from([(*id, bridge.clone())]),
            )?;
        }
        Ok(())
    }
    // Flat joined maps are visited by verify_joined_leaf; account for that work
    // even when callers request only one inventory projection.
    pub fn joined(&mut self, count: usize) -> Result<()> {
        self.charge(count)
    }
}

pub(super) fn collect(root: &dyn Node) -> Result<Inventory> {
    traverse(root, None)
}
pub(super) fn verify_joined(
    root: &dyn Node,
    reader: &dyn Fn(Uuid) -> Result<Option<Arc<RunCompletion>>>,
) -> Result<()> {
    traverse(root, Some(reader)).map(|_| ())
}

fn traverse(
    root: &dyn Node,
    reader: Option<&dyn Fn(Uuid) -> Result<Option<Arc<RunCompletion>>>>,
) -> Result<Inventory> {
    let mut leaves = Inventory::default();
    walk(root, Node::key, |node, children| {
        leaves.charge(1)?;
        node.expand(children, &mut leaves)?;
        leaves.charge(children.len())?;
        if let Some(reader) = reader {
            node.verify_joined_leaf(reader)?;
        }
        Ok(())
    })?;
    Ok(leaves)
}

#[derive(Clone, Copy)]
enum Mark {
    Active,
    Done,
}
enum Frame<N, K> {
    Enter(N),
    Exit(K),
}

// Shared by the production heterogeneous graph and plain-data regression tests.
// Enter/exit frames avoid native recursion and distinguish a shared tail from a
// true back-edge. A queued sibling is not Active until it is actually entered.
fn walk<N: Copy, K: Copy + Ord>(
    root: N,
    key: impl Fn(N) -> K,
    mut expand: impl FnMut(N, &mut Vec<N>) -> Result<()>,
) -> Result<()> {
    let mut pending = vec![Frame::Enter(root)];
    let mut marks = BTreeMap::new();
    let mut work = 0usize;
    while let Some(frame) = pending.pop() {
        match frame {
            Frame::Exit(id) => {
                marks.insert(id, Mark::Done);
            }
            Frame::Enter(node) => {
                work = work.checked_add(1).ok_or_else(limit)?;
                if work > MAX_WORK {
                    return Err(limit());
                }
                let id = key(node);
                match marks.get(&id) {
                    Some(Mark::Done) => continue,
                    Some(Mark::Active) => {
                        return Err(Error::new(
                            "review_round_lineage",
                            "retained capability graph contains a cycle",
                        ));
                    }
                    None => (),
                }
                if marks.len() >= MAX_NODES {
                    return Err(limit());
                }
                marks.insert(id, Mark::Active);
                let mut children = Vec::new();
                expand(node, &mut children)?;
                if children.len() > MAX_WORK.saturating_sub(pending.len()) {
                    return Err(limit());
                }
                pending.push(Frame::Exit(id));
                pending.extend(children.into_iter().rev().map(Frame::Enter));
            }
        }
    }
    Ok(())
}
fn limit() -> Error {
    Error::new(
        "history_limit",
        "retained capability traversal exceeds the protected history resource bound",
    )
}

#[cfg(test)]
#[path = "delivery_lineage_tests.rs"]
mod tests;
