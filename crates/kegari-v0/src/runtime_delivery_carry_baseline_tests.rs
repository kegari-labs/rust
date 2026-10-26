//! Data-only tests, not opaque provenance or native execution qualification.
use super::*;

fn repo(id: &str) -> RepositoryId {
    RepositoryId {
        hostname: "github.com".into(),
        id: id.into(),
    }
}
fn revision(n: u8) -> Revision {
    Revision {
        commit: format!("{n:040x}"),
        tree: format!("{:040x}", n + 100),
    }
}
fn link(a: u8, b: u8) -> Link {
    Link {
        operation: Uuid::from_u128(u128::from(b)),
        completion_digest: format!("completion-{b}"),
        integration_digest: format!("integration-{b}"),
        repository: repo("42"),
        from: revision(a),
        to: revision(b),
    }
}

#[test]
fn exact_released_path_uses_full_revisions_and_all_intermediate_links() {
    let links = vec![link(2, 3), link(1, 2)];
    assert_eq!(
        exact_chain(&repo("42"), &revision(1), &revision(3), &links).unwrap(),
        vec![1, 0]
    );
    let mut changed_tree = revision(1);
    changed_tree.tree = revision(9).tree;
    assert_eq!(
        exact_chain(&repo("42"), &changed_tree, &revision(3), &links)
            .unwrap_err()
            .message,
        "released baseline chain is missing, duplicated or forked"
    );
}

#[test]
fn missing_duplicate_forked_cross_repository_and_cyclic_paths_refuse() {
    for links in [
        vec![link(2, 3)],
        vec![link(1, 2), link(1, 2)],
        vec![link(1, 2), link(1, 3)],
    ] {
        assert_eq!(
            exact_chain(&repo("42"), &revision(1), &revision(3), &links)
                .unwrap_err()
                .message,
            "released baseline chain is missing, duplicated or forked"
        );
    }
    assert_eq!(
        exact_chain(&repo("99"), &revision(1), &revision(2), &[link(1, 2)])
            .unwrap_err()
            .code,
        "acceptance_carry_baseline"
    );
    assert_eq!(
        exact_chain(
            &repo("42"),
            &revision(1),
            &revision(3),
            &[link(1, 2), link(2, 1)]
        )
        .unwrap_err()
        .message,
        "released baseline chain contains a cycle"
    );
}

#[test]
fn identity_links_cannot_make_progress() {
    assert_eq!(
        exact_chain(&repo("42"), &revision(1), &revision(2), &[link(1, 1)])
            .unwrap_err()
            .message,
        "released baseline link made no source progress"
    );
}
