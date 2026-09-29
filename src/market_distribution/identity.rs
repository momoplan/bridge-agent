use super::validate_listing;
use local_app_contract::{ContractError, InstallSource, MarketListing};
use std::cmp::Ordering;

/// Catalog IDs are distribution details resolved from the requested source application.
/// The selected version may be older than the catalog's latest version.
pub fn resolve_source(
    requested: &local_app_contract::SourceVersion,
    listings: &[MarketListing],
) -> Result<InstallSource, ContractError> {
    let mut matched = None;
    for listing in listings {
        validate_listing(listing)?;
        if listing.frozen_version.source.application == requested.application
            && matched.replace(listing.listing_id).is_some()
        {
            return Err(ContractError::new("source", "ambiguous source application"));
        }
    }
    let listing_id = matched.ok_or_else(|| {
        ContractError::new(
            "source",
            "requested source application is not in the public catalog",
        )
    })?;
    Ok(InstallSource::Market {
        listing_id,
        version: requested.version.clone(),
        source: requested.clone(),
    })
}

/// Preserve the complete, explicitly requested identity at the response boundary.
pub fn resolve_exact<'a>(
    requested: &InstallSource,
    returned: &'a MarketListing,
) -> Result<&'a MarketListing, ContractError> {
    validate_listing(returned)?;
    match requested {
        InstallSource::Market {
            listing_id,
            version,
            source,
        } if listing_id == &returned.listing_id
            && source == &returned.frozen_version.source
            && version == &source.version =>
        {
            Ok(returned)
        }
        _ => Err(ContractError::new(
            "installSource",
            "returned listing, source application or exact version differs from request",
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
