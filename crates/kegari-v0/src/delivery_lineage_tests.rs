//! Plain graph data exercises the exact production walker. These tests create
//! no worker, joined completion, native grant, resume or delivery capability.
use super::*;

#[test]
fn deep_shared_tail_is_expanded_once_without_recursion() {
    let mut expanded = vec![0u8; MAX_NODES];
    walk(
        MAX_NODES - 1,
        |node| node,
        |node, children| {
            expanded[node] += 1;
            if node > 0 {
                // Models Round.previous and Round.unfinished->the same previous
                // identity after the heterogeneous node adds its immediate edges.
                children.extend([node - 1, node - 1]);
            }
            Ok(())
        },
    )
    .unwrap();
    assert!(expanded.iter().all(|count| *count == 1));
}

#[test]
fn queued_sibling_is_not_a_cycle_but_an_active_ancestor_is() {
    let mut counts = [0; 4];
    walk(
        0usize,
        |node| node,
        |node, children| {
            counts[node] += 1;
            match node {
                0 => children.extend([1, 2]),
                1 => children.extend([2, 3]),
                2 => children.push(3),
                _ => (),
            }
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(counts, [1, 1, 1, 1]);
    let error = walk(
        0usize,
        |node| node,
        |node, children| {
            children.push((node + 1) % 3);
            Ok(())
        },
    )
    .unwrap_err();
    assert_eq!(error.code, "review_round_lineage");
}

#[test]
fn distinct_typed_objects_are_not_collapsed_by_equal_content() {
    struct First;
    struct Second;
    impl Node for First {
        fn expand<'a>(
            &'a self,
            _children: &mut Vec<&'a dyn Node>,
            _leaves: &mut Inventory,
        ) -> Result<()> {
            Ok(())
        }
    }
    impl Node for Second {
        fn expand<'a>(
            &'a self,
            _children: &mut Vec<&'a dyn Node>,
            _leaves: &mut Inventory,
        ) -> Result<()> {
            Ok(())
        }
    }
    let first = First;
    let second = Second;
    // Zero-sized nodes may have the same address, but never the same typed key.
    assert!(first.key() != second.key());
    let same_content = ["same digest", "same digest"];
    let mut expanded = Vec::new();
    walk(
        0usize,
        |node| node,
        |node, children| {
            expanded.push(same_content[node]);
            if node == 0 {
                children.push(1);
            }
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(expanded, same_content);
}

#[test]
fn history_resource_limits_fail_without_truncating_or_resetting() {
    let mut count = 0;
    let error = walk(
        MAX_NODES,
        |node| node,
        |node, children| {
            count += 1;
            if node > 0 {
                children.push(node - 1);
            }
            Ok(())
        },
    )
    .unwrap_err();
    assert_eq!(count, MAX_NODES);
    assert_eq!(error.code, "history_limit");

    let mut leaves = Inventory::default();
    leaves.joined(MAX_WORK).unwrap();
    assert_eq!(leaves.joined(1).unwrap_err().code, "history_limit");
    // A repeated visit is still finite work even when the leaf is already Done.
    let error = walk(
        0usize,
        |node| node,
        |node, children| {
            if node == 0 {
                children.resize(MAX_WORK, 1);
            }
            Ok(())
        },
    )
    .unwrap_err();
    assert_eq!(error.code, "history_limit");
}

#[test]
fn a_failure_in_any_shared_leaf_is_not_hidden_by_deduplication() {
    let error = walk(
        0usize,
        |node| node,
        |node, children| {
            match node {
                0 => children.extend([1, 2]),
                1 => children.push(3),
                2 => children.push(3),
                3 => return Err(Error::new("worker_binding", "retained identity changed")),
                _ => unreachable!(),
            }
            Ok(())
        },
    )
    .unwrap_err();
    assert_eq!(error.code, "worker_binding");
}
