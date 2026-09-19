use super::*;
use local_app_contract::{InstallSource, MarketListing};
use serde_json::json;

fn fixture() -> (MarketListing, InstallSource, Value) {
    let manifest: Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/market-connector-3.0.0.json"
    ))
    .unwrap();
    let listing: MarketListing = serde_json::from_value(json!({
        "contractVersion": "2.0.0",
        "presentation": { "name": "Reviewed connector", "description": "Description" },
        "marketKey": "test-market",
        "listingId": "00000000-0000-0000-0000-000000000001",
        "frozenVersion": {
            "contractVersion": "1.0.0",
            "source": {
                "application": { "environmentKey": "author-a", "appId": "test-app" },
                "version": "1.0.0"
            },
            "content": {
                "applicationType": "connector",
                "sourceRevision": "commit-1",
                "manifest": manifest,
                "artifacts": [{
                    "artifactId": "00000000-0000-0000-0000-000000000002",
                    "platform": "linux", "architecture": "x86_64",
                    "fileName": "connector.zip", "sizeBytes": 256
                }]
            }
        }
    }))
    .unwrap();
    let selected = InstallSource::Market {
        market_key: listing.market_key.clone(),
        listing_id: listing.listing_id,
        version: listing.frozen_version.source.version.clone(),
        source: listing.frozen_version.source.clone(),
    };
    (listing, selected, manifest)
}

fn validate_package(
    manifest: &Value,
    listing: &MarketListing,
    selected: &InstallSource,
) -> Result<ConnectorInstallProvenance, String> {
    let directory = tempfile::tempdir().unwrap();
    fs::write(
        directory.path().join("connector.json"),
        serde_json::to_vec(manifest).unwrap(),
    )
    .unwrap();
    market_package_provenance(directory.path(), selected, listing, false)
}

#[test]
fn package_identity_uses_registered_version_without_full_manifest_comparison() {
    let (listing, selected, mut package) = fixture();
    package["name"] = json!("Packaged connector name");
    package["runtime"]["ownerExtension"] = json!(false);
    validate_package(&package, &listing, &selected).unwrap();
}

#[test]
fn package_identity_rejects_other_app_or_version() {
    let (listing, selected, package) = fixture();
    for (field, value) in [("appId", "other-app"), ("version", "1.0.1")] {
        let mut changed = package.clone();
        changed[field] = json!(value);
        let error = validate_package(&changed, &listing, &selected).unwrap_err();
        assert!(error.contains("selected frozen source version"), "{error}");
    }
}

#[test]
fn matching_version_does_not_bypass_package_contract_validation() {
    let (listing, selected, mut package) = fixture();
    package["schemaVersion"] = json!("0.0.0");
    assert!(validate_package(&package, &listing, &selected).is_err());
}

#[test]
fn matching_package_does_not_authorize_another_market_source() {
    let (listing, mut selected, package) = fixture();
    if let InstallSource::Market { source, .. } = &mut selected {
        source.application.environment_key = "author-b".to_owned().try_into().unwrap();
    }
    assert!(validate_package(&package, &listing, &selected).is_err());
}
