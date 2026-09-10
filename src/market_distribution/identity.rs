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

/// Only select a newer release for an already proven market/source binding.
/// Missing provenance is a migration blocker, never a request to infer an owner.
/// This function does not authorize installation or mutate an installation record.
pub fn select_upgrade(
    installed: Option<&InstallSource>,
    candidate: &MarketListing,
) -> Result<Option<InstallSource>, ContractError> {
    validate_listing(candidate)?;
    let Some(InstallSource::Market {
        market_key,
        listing_id,
        version,
        source,
    }) = installed
    else {
        return Err(ContractError::new(
            "installed.source",
            "proven market installation identity required",
        ));
    };
    if market_key != &candidate.market_key
        || listing_id != &candidate.listing_id
        || source.application != candidate.frozen_version.source.application
        || version != &source.version
    {
        return Err(ContractError::new(
            "installed.source",
            "upgrade cannot cross market, listing or source application",
        ));
    }
    let next = &candidate.frozen_version.source.version;
    if next.precedence_cmp(version) != Ordering::Greater {
        return Ok(None);
    }
    Ok(Some(InstallSource::Market {
        market_key: candidate.market_key.clone(),
        listing_id: candidate.listing_id,
        version: next.clone(),
        source: candidate.frozen_version.source.clone(),
    }))
}
