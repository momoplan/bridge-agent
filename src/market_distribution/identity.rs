use super::validate_listing;
use local_app_contract::{ContractError, InstallSource, MarketListing};
use std::cmp::Ordering;

/// Preserve the complete, explicitly requested identity at the response boundary.
pub fn resolve_exact<'a>(
    requested: &InstallSource,
    returned: &'a MarketListing,
) -> Result<&'a MarketListing, ContractError> {
    validate_listing(returned)?;
    match requested {
        InstallSource::Market {
            market_key,
            listing_id,
            version,
            source,
        } if market_key == &returned.market_key
            && listing_id == &returned.listing_id
            && source == &returned.frozen_version.source
            && version == &source.version =>
        {
            Ok(returned)
        }
        _ => Err(ContractError::new(
            "installSource",
            "returned market, listing, source application or exact version differs from request",
        )),
    }
}

/// Only select a newer release for an already proven source application.
/// Market and listing identify a distribution route, not application lineage.
/// Missing provenance is a migration blocker, never a request to infer an owner.
/// This function does not authorize installation or mutate an installation record.
pub fn select_upgrade(
    installed: Option<&InstallSource>,
    candidate: &MarketListing,
) -> Result<Option<InstallSource>, ContractError> {
    validate_listing(candidate)?;
    let Some(installed) = installed else {
        return Err(ContractError::new(
            "installed.source",
            "proven source application identity required",
        ));
    };
    let source = match installed {
        InstallSource::Market {
            version, source, ..
        } if version == &source.version => source,
        InstallSource::Market { .. } => {
            return Err(ContractError::new(
                "installed.source.version",
                "installed market version differs from source version",
            ));
        }
        InstallSource::Environment { source } => source,
    };
    if source.application != candidate.frozen_version.source.application {
        return Err(ContractError::new(
            "installed.source",
            "upgrade cannot cross source applications",
        ));
    }
    let next = &candidate.frozen_version.source.version;
    if next.precedence_cmp(&source.version) != Ordering::Greater {
        return Ok(None);
    }
    Ok(Some(InstallSource::Market {
        market_key: candidate.market_key.clone(),
        listing_id: candidate.listing_id,
        version: next.clone(),
        source: candidate.frozen_version.source.clone(),
    }))
}

/// A universal archive can run on either CPU architecture of its declared platform.
/// Prefer the exact target when both are present; never cross platform boundaries.
pub fn select_artifact<'a>(
    frozen: &'a local_app_contract::FrozenVersion,
    platform: &str,
    architecture: &str,
) -> Result<Option<&'a local_app_contract::Artifact>, ContractError> {
    frozen.validate()?;
    Ok(frozen
        .content
        .artifacts
        .iter()
        .find(|artifact| artifact.platform == platform && artifact.architecture == architecture)
        .or_else(|| {
            frozen.content.artifacts.iter().find(|artifact| {
                artifact.platform == platform && artifact.architecture == "universal"
            })
        }))
}
