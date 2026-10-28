use kegari_git as model;
use model::*;
use std::collections::BTreeSet;
mod support;

#[test]
fn repository_and_object_ids_preserve_exact_validation_errors() {
    support::repository().validate().unwrap();
    for hostname in [
        "",
        "GitHub.com",
        "github.com/path",
        "github.com:443",
        "host name",
        "host\\name",
        "host\0name",
    ] {
        let mut repository = support::repository();
        repository.hostname = hostname.into();
        assert_eq!(repository.validate().unwrap_err().code, "invalid_config");
    }
    for id in ["", " \t", "line\nfeed"] {
        let mut repository = support::repository();
        repository.id = id.into();
        assert_eq!(repository.validate().unwrap_err().code, "invalid_config");
    }
    for length in [40, 64] {
        validate_oid(&"a".repeat(length)).unwrap();
        validate_oid(&"0".repeat(length)).unwrap();
    }
    for value in [
        "a".repeat(39),
        "a".repeat(41),
        "a".repeat(63),
        "a".repeat(65),
        "A".repeat(40),
        "g".repeat(40),
        "日本".repeat(20),
    ] {
        let error = validate_oid(&value).unwrap_err();
        assert_eq!(error.code, "invalid_revision");
        assert_eq!(
            error.message,
            "an exact lowercase Git object ID is required"
        );
    }
    let mut revision = support::revision('1');
    revision.tree = "bad".into();
    assert_eq!(revision.validate().unwrap_err().code, "invalid_revision");
}

#[test]
fn unicode_titles_are_preserved_without_shortening_or_transliteration() {
    assert_eq!(
        title_suffix("Full Ångström / 日本語 Title @{Draft} .. Final.lock").unwrap(),
        "Full-Ångström-日本語-Title-@-Draft}-Final-lock"
    );
    assert_eq!(title_suffix("name.lock-").unwrap(), "name-lock");
    assert_eq!(title_suffix("a...b\\c\t~^:?*[").unwrap(), "a-b-c");
    let long = "日本語".repeat(1024);
    assert_eq!(title_suffix(&long).unwrap(), long);
    assert_eq!(
        title_suffix(".. / \n ~").unwrap_err().code,
        "invalid_branch_name"
    );
}

#[test]
fn syntactic_paths_and_scope_boundaries_keep_existing_semantics() {
    for path in [
        "",
        ".",
        "../file",
        "src/../file",
        "/tmp/file",
        "src\\file",
        "file\0name",
        ".git/config",
        "src/.GIT/config",
    ] {
        assert_eq!(
            validate_repo_path(path).unwrap_err().code,
            "scope_violation",
            "{path:?}"
        );
    }
    // Path::components currently accepts these lexical forms; extraction does
    // not canonicalize them or add filesystem requirements.
    for path in [
        "src/file",
        "src//file",
        "src/./file",
        "nonexistent/日本語",
        "src/",
    ] {
        validate_repo_path(path).unwrap();
    }
    let scope = WriteScope {
        paths: vec!["src/".into(), "README.md".into()],
    };
    scope.validate().unwrap();
    for path in ["src/lib.rs", "src/nested/file", "README.md"] {
        assert!(scope.permits(path));
    }
    for path in [
        "src-other/file",
        "README.md/child",
        "../secret",
        "src/.git/config",
    ] {
        assert!(!scope.permits(path));
    }
    let all = WriteScope {
        paths: vec![".".into()],
    };
    all.validate().unwrap();
    assert!(all.permits("any/file"));
    assert!(!all.permits(".git/config"));
    for paths in [
        vec![],
        vec!["src/", "src/"],
        vec!["src/*"],
        vec!["src/?"],
        vec!["src/[ab]"],
    ] {
        let scope = WriteScope {
            paths: paths.into_iter().map(str::to_owned).collect(),
        };
        assert_eq!(scope.validate().unwrap_err().code, "invalid_config");
    }
    let unsafe_glob = WriteScope {
        paths: vec!["../*".into()],
    };
    assert_eq!(unsafe_glob.validate().unwrap_err().code, "scope_violation");
}

#[test]
fn human_identity_and_signoff_checks_preserve_bounds_and_order() {
    support::identity().validate().unwrap();
    let mut identity = support::identity();
    identity.name = "n".repeat(1024);
    identity.validate().unwrap();
    identity.name.push('n');
    assert!(identity.validate().is_err());
    for invalid in [
        "",
        " ",
        "line\nbreak",
        "carriage\rreturn",
        "<angle",
        "angle>",
        "nul\0byte",
    ] {
        for name in [true, false] {
            let mut identity = support::identity();
            if name {
                identity.name = invalid.into();
            } else {
                identity.email = invalid.into();
            }
            assert_eq!(identity.validate().unwrap_err().code, "invalid_config");
        }
    }
    let original = support::metadata();
    original.validate().unwrap();
    for message in [
        "",
        "no signoff",
        "Signed-off-by: Other <other@example.test>\n",
        "Signed-off-by: Fixture Human <fixture@example.test>",
        "bad\0\nSigned-off-by: Fixture Human <fixture@example.test>\n",
    ] {
        let mut metadata = original.clone();
        metadata.message = message.into();
        assert!(metadata.validate().is_err());
    }
    let mut metadata = original.clone();
    metadata.timestamp = chrono::DateTime::from_timestamp(1_700_000_000, 1).unwrap();
    assert!(metadata.validate().is_err());
    let signoff = "Signed-off-by: Fixture Human <fixture@example.test>\n";
    metadata = original.clone();
    metadata.message = "x".repeat(65536 - signoff.len()) + signoff;
    metadata.validate().unwrap();
    metadata.message.insert(0, 'x');
    assert!(metadata.validate().is_err());
    metadata.identity.name.clear();
    assert_eq!(
        metadata.validate().unwrap_err().message,
        "a valid configured human commit identity is required"
    );
}

#[test]
fn replacements_retain_exact_contributions_and_bounded_history() {
    let original = support::delta(2);
    let expected = BTreeSet::from([
        original.digest().unwrap(),
        support::delta(3).digest().unwrap(),
    ]);
    assert_eq!(
        support::replacement().original_contributions().unwrap(),
        expected
    );
    let mut duplicate = original.clone();
    duplicate.replacement = Some(ReplacementMapping {
        originals: vec![original.clone(), original.clone()],
        reason: "duplicate".into(),
        staging_result_digest: support::digest('a'),
    });
    assert_eq!(
        duplicate.original_contributions().unwrap_err().message,
        "original contribution is accounted for more than once"
    );
    duplicate.replacement.as_mut().unwrap().originals.clear();
    assert_eq!(
        duplicate.original_contributions().unwrap_err().message,
        "replacement drops its original contribution inventory"
    );
    let mut nested = original.clone();
    for _ in 0..8 {
        let mut parent = original.clone();
        parent.replacement = Some(ReplacementMapping {
            originals: vec![nested],
            reason: "retained".into(),
            staging_result_digest: support::digest('b'),
        });
        nested = parent;
    }
    assert_eq!(
        nested.original_contributions().unwrap(),
        BTreeSet::from([original.digest().unwrap()])
    );
    let mut too_deep = original;
    too_deep.replacement = Some(ReplacementMapping {
        originals: vec![nested],
        reason: "too deep".into(),
        staging_result_digest: support::digest('b'),
    });
    assert_eq!(
        too_deep.original_contributions().unwrap_err().message,
        "replacement history exceeds supported depth"
    );
}

#[test]
fn review_identity_excludes_future_staging_but_binds_reviewed_inputs() {
    let original = support::delta(2);
    let review = original.review_digest().unwrap();
    let mut staged = original.clone();
    staged.staging_parent = support::revision('5');
    staged.staging_commit = support::revision('6');
    assert_eq!(staged.review_digest().unwrap(), review);
    assert_ne!(staged.digest().unwrap(), original.digest().unwrap());
    let changes: [fn(&mut DeltaManifest); 8] = [
        |v| v.scope.paths.push("extra".into()),
        |v| v.validated_head = support::revision('7'),
        |v| v.validated_base = support::revision('8'),
        |v| v.patch_digest = support::digest('d'),
        |v| v.paths.push("extra".into()),
        |v| v.work_generation = support::digest('e'),
        |v| v.correction_generation = support::digest('f'),
        |v| {
            v.replacement = Some(ReplacementMapping {
                originals: vec![support::delta(3)],
                reason: "replacement".into(),
                staging_result_digest: support::digest('a'),
            })
        },
    ];
    for change in changes {
        let mut changed = original.clone();
        change(&mut changed);
        assert_ne!(changed.review_digest().unwrap(), review);
    }
    let artifact = DeltaArtifact {
        manifest: original,
        patch: vec![0, 255, b'\n'],
    };
    let cloned = artifact.clone();
    assert_eq!(cloned.patch, artifact.patch);
    assert_eq!(cloned.manifest, artifact.manifest);
}
