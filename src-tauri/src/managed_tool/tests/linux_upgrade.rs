use super::*;

#[test]
#[ignore = "downloads released Linux artifacts using scripts/test-linux-cli-upgrade.mjs"]
fn linux_cli_upgrade_acceptance() {
    let fixture = TestInstallation::new();
    let directory = PathBuf::from(std::env::var_os("BAIJIMU_CLI_UPGRADE_FIXTURE_DIR").unwrap());
    let baseline: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../tests/fixtures/cli-upgrade/baseline.json"
    ))
    .unwrap();
    let listing: local_app_contract::MarketListing = serde_json::from_str(include_str!(
        "../../../../tests/fixtures/cli-upgrade/market-listing.json"
    ))
    .unwrap();
    let previous_version = baseline["version"].as_str().unwrap();
    let old = fixture.temp.path().join("previous-cli");
    let bytes = fs::read(directory.join("previous.zip")).unwrap();
    write_executable(
        &old,
        &extract_binary(&bytes, "previous.zip", Some("bin/baijimu")).unwrap(),
    )
    .unwrap();
    fs::write(
        old.with_file_name("previous-cli.market.json"),
        serde_json::to_vec(&baseline["installSource"]).unwrap(),
    )
    .unwrap();
    let installed = bootstrap_bundled(Some(&old)).unwrap();
    assert_eq!(
        installed.installed_version.as_deref(),
        Some(previous_version)
    );
    let selection = bridge_agent::market_distribution::select_upgrade(
        installed.update_source.as_ref(),
        &listing,
    )
    .unwrap()
    .unwrap();
    // Discovery is read-only: installation changes only after the user's action.
    assert_eq!(
        validate_cli(&launcher_path(), None).unwrap(),
        previous_version
    );
    let artifact = bridge_agent::market_distribution::select_artifact(
        &listing.frozen_version,
        "linux",
        "x86_64",
    )
    .unwrap()
    .unwrap();
    let current = fs::read(directory.join("current.zip")).unwrap();
    // A failed install keeps the previous executable and remains retryable.
    assert!(install_market_package(
        &[],
        artifact,
        selection.clone(),
        &listing,
        Some("bin/baijimu")
    )
    .is_err());
    assert_eq!(
        validate_cli(&launcher_path(), None).unwrap(),
        previous_version
    );
    let upgraded =
        install_market_package(&current, artifact, selection, &listing, Some("bin/baijimu"))
            .unwrap();
    let version = listing.frozen_version.source.version.to_string();
    assert_eq!(
        upgraded.installed_version.as_deref(),
        Some(version.as_str())
    );
    assert_eq!(validate_cli(&launcher_path(), None).unwrap(), version);
    assert_eq!(upgraded.previous_version.as_deref(), Some(previous_version));
    let restarted = bootstrap_bundled(Some(&old)).unwrap();
    assert_eq!(restarted.installed_version, upgraded.installed_version);
    assert_eq!(restarted.install_source, upgraded.install_source);
    assert_project_git_routes(&fixture.temp, &launcher_path());
    let rollback = rollback().unwrap();
    assert_eq!(
        rollback.installed_version.as_deref(),
        Some(previous_version)
    );
    assert_eq!(
        validate_cli(&launcher_path(), None).unwrap(),
        previous_version
    );
    println!("Linux released CLI: discovered without install; {previous_version} -> {version}; failed install/retry, restart and rollback passed");
}

// Run the published native executable against a local protocol fixture. An old
// CLI route must fail with 404, so version-only acceptance cannot pass this test.
fn assert_project_git_routes(temp: &tempfile::TempDir, executable: &Path) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let auth = temp.path().join("fixture-auth.json");
    fs::write(&auth, serde_json::to_vec(&serde_json::json!({
        "schemaVersion":2, "currentEnvironment":"test", "currentWorkspaceId":42,
        "environments":{"test":{"environmentKey":"test-environment", "baseUrl":format!("http://{address}")}},
        "credentials":[{"environmentKey":"test-environment", "token":"lc_pat_fixture", "tokenType":"pat", "workspaceIds":[42]}]
    })).unwrap()).unwrap();
    let server = std::thread::spawn(move || {
        for _ in 0..2 {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(10)))
                .unwrap();
            let mut request = [0u8; 16384];
            let count = stream.read(&mut request).unwrap();
            let request = String::from_utf8_lossy(&request[..count]);
            let valid = request.starts_with(
                "GET /partner/v1/project-git-service/api/workspaces/42/projects/71/git/branches ",
            ) || request
                .starts_with("POST /partner/v1/project-service/api/projects/71/summary/info ");
            let status = if valid { "200 OK" } else { "404 Not Found" };
            let body = r#"{"contractVersion":"1.0.0","errorCode":"0","data":{"projectId":71,"defaultBranch":"main","branches":[{"name":"main"}]}}"#;
            write!(stream, "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
            assert!(valid, "published CLI used an obsolete API route");
        }
    });
    for command in ["branches", "get"] {
        let result = Command::new(executable)
            .args(["project", command, "71", "--workspace-id", "42"])
            .env("BAIJIMU_AUTH_FILE", &auth)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{command}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert!(String::from_utf8_lossy(&result.stdout).contains("71"));
    }
    server.join().unwrap();
}
