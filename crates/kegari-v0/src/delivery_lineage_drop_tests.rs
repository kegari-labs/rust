//! Plain owned graph data uses the production release queue. No execution,
//! source, joined-run, native grant, or round capability is constructed.
use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Default)]
struct Counts {
    nodes: AtomicUsize,
    carriers: AtomicUsize,
    active: AtomicUsize,
    depth: AtomicUsize,
}
impl Counts {
    fn enter(&self) {
        let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
        self.depth.fetch_max(active, Ordering::SeqCst);
    }
    fn leave(&self) {
        self.active.fetch_sub(1, Ordering::SeqCst);
    }
}
struct Node {
    previous: Option<Arc<Node>>,
    carrier: Option<Arc<Carrier>>,
    counts: Arc<Counts>,
}
struct Carrier {
    previous: Option<Arc<Node>>,
    counts: Arc<Counts>,
}
impl Node {
    fn detach(&mut self, pending: &mut Pending) {
        if let Some(p) = self.previous.take() {
            pending.push(p);
        }
        if let Some(p) = self.carrier.take() {
            pending.push(p);
        }
    }
}
impl Release for Node {
    fn release(mut self, pending: &mut Pending) {
        self.detach(pending);
    }
}
impl Drop for Node {
    fn drop(&mut self) {
        self.counts.enter();
        drain(|pending| self.detach(pending));
        self.counts.nodes.fetch_add(1, Ordering::SeqCst);
        self.counts.leave();
    }
}
impl Release for Carrier {
    fn release(mut self, pending: &mut Pending) {
        if let Some(p) = self.previous.take() {
            pending.push(p);
        }
    }
}
impl Drop for Carrier {
    fn drop(&mut self) {
        self.counts.enter();
        drain(|pending| {
            if let Some(p) = self.previous.take() {
                pending.push(p);
            }
        });
        self.counts.carriers.fetch_add(1, Ordering::SeqCst);
        self.counts.leave();
    }
}
fn chain(count: usize, counts: &Arc<Counts>) -> Arc<Node> {
    let mut current = Arc::new(Node {
        previous: None,
        carrier: None,
        counts: counts.clone(),
    });
    for _ in 1..count {
        current = Arc::new(Node {
            previous: Some(current.clone()),
            carrier: Some(Arc::new(Carrier {
                previous: Some(current),
                counts: counts.clone(),
            })),
            counts: counts.clone(),
        });
    }
    current
}

#[test]
fn deep_heterogeneous_shared_edges_release_once_with_constant_drop_depth() {
    let counts = Arc::new(Counts::default());
    let root = chain(65_536, &counts);
    drop(root);
    assert_eq!(counts.nodes.load(Ordering::SeqCst), 65_536);
    assert_eq!(counts.carriers.load(Ordering::SeqCst), 65_535);
    assert_eq!(counts.active.load(Ordering::SeqCst), 0);
    assert!(counts.depth.load(Ordering::SeqCst) <= 2);
}

#[test]
fn another_real_owner_keeps_shared_tail_until_its_last_release() {
    let counts = Arc::new(Counts::default());
    let tail = chain(65_536, &counts);
    let first = Arc::new(Node {
        previous: Some(tail.clone()),
        carrier: None,
        counts: counts.clone(),
    });
    let second = Arc::new(Node {
        previous: Some(tail.clone()),
        carrier: None,
        counts: counts.clone(),
    });
    drop(tail);
    drop(first);
    assert_eq!(counts.nodes.load(Ordering::SeqCst), 1);
    assert_eq!(counts.carriers.load(Ordering::SeqCst), 0);
    assert_eq!(Arc::strong_count(second.previous.as_ref().unwrap()), 1);
    drop(second);
    assert_eq!(counts.nodes.load(Ordering::SeqCst), 65_538);
    assert_eq!(counts.carriers.load(Ordering::SeqCst), 65_535);
    assert!(counts.depth.load(Ordering::SeqCst) <= 2);
}

#[test]
fn concurrent_last_owners_do_not_lose_the_unique_tail() {
    let counts = Arc::new(Counts::default());
    let tail = chain(1024, &counts);
    let barrier = Arc::new(std::sync::Barrier::new(2));
    let owners: Vec<_> = (0..2)
        .map(|_| {
            Arc::new(Node {
                previous: Some(tail.clone()),
                carrier: None,
                counts: counts.clone(),
            })
        })
        .collect();
    drop(tail);
    let handles: Vec<_> = owners
        .into_iter()
        .map(|owner| {
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                drop(owner);
            })
        })
        .collect();
    for handle in handles {
        handle.join().unwrap();
    }
    assert_eq!(counts.nodes.load(Ordering::SeqCst), 1026);
    assert_eq!(counts.carriers.load(Ordering::SeqCst), 1023);
    assert_eq!(counts.active.load(Ordering::SeqCst), 0);
}
