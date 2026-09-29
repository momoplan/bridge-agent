use anyhow::{ensure, Result};
use local_app_contract::{InstallSource, SemanticVersion, SourceVersion};
use serde::Deserialize;
use serde_json::value::RawValue;
use std::collections::BTreeMap;
use uuid::Uuid;

// Legacy protocol is isolated in the owner migration artifact, never the host decoder.
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
enum StoredSource {
    Environment {
        source: SourceVersion,
    },
    Market {
        #[serde(rename = "marketKey")]
        market_key: Option<String>,
        #[serde(rename = "listingId")]
        listing_id: Uuid,
        version: SemanticVersion,
        source: SourceVersion,
    },
}

fn source(bytes: &[u8]) -> Result<Option<Vec<u8>>> {
    let stored: StoredSource = local_app_contract::decode(bytes)?;
    let (source, changed) = match stored {
        StoredSource::Environment { source } => (InstallSource::Environment { source }, false),
        StoredSource::Market {
            market_key,
            listing_id,
            version,
            source,
        } => {
            ensure!(!listing_id.is_nil(), "missing catalog entry");
            ensure!(version == source.version, "conflicting source version");
            if let Some(key) = &market_key {
                ensure!(!key.trim().is_empty(), "empty legacy market key");
            }
            (
                InstallSource::Market {
                    listing_id,
                    version,
                    source,
                },
                market_key.is_some(),
            )
        }
    };
    Ok(changed.then(|| serde_json::to_vec(&source)).transpose()?)
}

pub fn convert(bytes: &[u8], sidecar: bool) -> Result<Option<Vec<u8>>> {
    if sidecar {
        return source(bytes);
    }
    // Only the two provenance fields are transformed; unrelated JSON tokens stay opaque.
    let mut record: BTreeMap<String, Box<RawValue>> = serde_json::from_slice(bytes)?;
    let mut changed = false;
    for field in ["installSource", "previousInstallSource"] {
        if let Some(raw) = record.get_mut(field) {
            if raw.get() == "null" {
                continue;
            }
            if let Some(next) = source(raw.get().as_bytes())? {
                *raw = RawValue::from_string(String::from_utf8(next)?)?;
                changed = true;
            }
        }
    }
    Ok(changed
        .then(|| serde_json::to_vec_pretty(&record))
        .transpose()?)
}
