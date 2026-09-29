use super::*;

fn legacy(environment: &str) -> String {
    format!(
        r#"{{"kind":"market","marketKey":"legacy-market","listingId":"00000000-0000-0000-0000-000000000001","version":"1.0.0","source":{{"application":{{"environmentKey":"{environment}","appId":"example-app"}},"version":"1.0.0"}}}}"#
    )
}

#[test]
fn removes_only_market_identity_and_preserves_exact_source() {
    let original = legacy("author-a");
    let next = source::convert(original.as_bytes(), true).unwrap().unwrap();
    let parsed: local_app_contract::InstallSource = local_app_contract::decode(&next).unwrap();
    let local_app_contract::InstallSource::Market { source, .. } = parsed else {
        panic!()
    };
    assert_eq!(source.application.environment_key.as_str(), "author-a");
    assert!(source::convert(&next, true).unwrap().is_none());
}

#[test]
fn preserves_manifest_bytes_and_both_rollback_sources() {
    let manifest = r#"{ "appId" : "example-app", "custom": [1, 2] }"#;
    let record = format!(
        r#"{{"installSource":{},"previousInstallSource":{},"manifest":{manifest},"installedAtEpochMs":123}}"#,
        legacy("author-a"),
        legacy("author-b")
    );
    let next = source::convert(record.as_bytes(), false).unwrap().unwrap();
    let text = String::from_utf8(next).unwrap();
    assert!(text.contains(manifest));
    assert!(text.contains("author-a") && text.contains("author-b"));
    assert!(!text.contains("marketKey"));
}

#[test]
fn source_less_records_are_unchanged_and_never_claimed() {
    for original in [
        br#"{"installSource":null,"manifest":{}}"#.as_slice(),
        br#"{"manifest":{}}"#,
    ] {
        assert!(source::convert(original, false).unwrap().is_none());
    }
}

#[test]
fn preflight_failure_leaves_all_records_untouched() {
    let root = tempfile::tempdir().unwrap();
    let first = root.path().join("first.market.json");
    let second = root.path().join("second.market.json");
    let original = legacy("author-a");
    fs::write(&first, &original).unwrap();
    fs::write(
        &second,
        original.replace("\"environmentKey\":\"author-a\",", ""),
    )
    .unwrap();
    assert!(migrate(&[(first.clone(), true), (second, true)]).is_err());
    assert_eq!(fs::read_to_string(first).unwrap(), original);
}

#[test]
fn migration_backs_up_original_and_can_be_repeated() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("app.market.json");
    let original = legacy("author-a");
    fs::write(&path, &original).unwrap();
    let files = vec![(path.clone(), true)];
    migrate(&files).unwrap();
    let migrated = fs::read(&path).unwrap();
    migrate(&files).unwrap();
    assert_eq!(fs::read(&path).unwrap(), migrated);
    assert_eq!(
        fs::read_to_string(path.with_extension("json.before-environment-identity-3")).unwrap(),
        original
    );
}

#[test]
fn discovers_actual_local_app_records_and_managed_rollback_sidecars() {
    let root = tempfile::tempdir().unwrap();
    let apps = root.path().join("local-apps");
    let managed = root.path().join("apps");
    for directory in [apps.join("example"), managed.join("cli/versions/1.0.0")] {
        fs::create_dir_all(directory).unwrap();
    }
    let paths = [
        apps.join("example/install.json"),
        managed.join("cli/state.json"),
        managed.join("cli/versions/1.0.0/baijimu.market.json"),
    ];
    for path in &paths {
        fs::write(path, "{}").unwrap();
    }
    let found = discover(&apps, &managed).unwrap();
    assert_eq!(found.len(), 3);
    for path in paths {
        assert!(found.iter().any(|(item, _)| *item == path));
    }
}
