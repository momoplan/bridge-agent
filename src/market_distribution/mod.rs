//! Stateless consumer preparation for the market-owned distribution contract.
//!
//! Not connected to desktop commands, installation records, or existing routes.
//! Installation activation requires an explicit migration of proven source identity.
use local_app_contract::{ContractError, InstallSource, MarketKey, MarketListing, MarketPage};
use std::collections::HashSet;
use url::Url;
use uuid::Uuid;

mod identity;
pub use identity::{resolve_exact, select_upgrade};

/// The caller supplies an authoritative configured market key and API base URL.
/// A source environment's identity must never be inferred from this address.
#[derive(Debug, Clone)]
pub struct MarketDistribution {
    market_key: MarketKey,
    base_url: Url,
}

impl MarketDistribution {
    pub fn new(market_key: MarketKey, base_url: &str) -> Result<Self, ContractError> {
        let url = Url::parse(base_url)
            .map_err(|_| ContractError::new("market.baseUrl", "invalid URL"))?;
        if base_url.trim() != base_url
            || url.scheme() != "https"
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(ContractError::new(
                "market.baseUrl",
                "explicit HTTPS authority without credentials, query or fragment required",
            ));
        }
        Ok(Self {
            market_key,
            base_url: url,
        })
    }

    fn path(&self, segments: &[&str]) -> Url {
        let mut url = self.base_url.clone();
        // new() accepts only hierarchical HTTPS URLs.
        url.path_segments_mut()
            .expect("validated HTTPS base")
            .pop_if_empty()
            .extend(segments);
        url
    }

    pub fn page_url(&self, after: Option<Uuid>) -> Result<Url, ContractError> {
        let mut url = self.path(&["listings"]);
        if let Some(after) = after {
            require_id(after, "after")?;
            url.query_pairs_mut()
                .append_pair("after", &after.to_string());
        }
        Ok(url)
    }

    pub fn version_url(&self, selection: &InstallSource) -> Result<Url, ContractError> {
        let (listing, version) = self.market_selection(selection)?;
        Ok(self.path(&["listings", &listing.to_string(), "versions", &version]))
    }

    /// Download addresses are constructed from the configured market authority,
    /// never from manifest source URLs or the author's environment.
    pub fn artifact_url(
        &self,
        selection: &InstallSource,
        listing: &MarketListing,
        artifact: Uuid,
    ) -> Result<Url, ContractError> {
        self.market_selection(selection)?;
        resolve_exact(selection, listing)?;
        require_id(artifact, "artifactId")?;
        if !listing
            .frozen_version
            .content
            .artifacts
            .iter()
            .any(|a| a.artifact_id == artifact)
        {
            return Err(ContractError::new(
                "artifactId",
                "artifact is not in selected frozen version",
            ));
        }
        let mut url = self.version_url(selection)?;
        url.path_segments_mut()
            .expect("validated HTTPS base")
            .extend(["artifacts", &artifact.to_string()]);
        Ok(url)
    }

    fn market_selection(&self, selection: &InstallSource) -> Result<(Uuid, String), ContractError> {
        match selection {
            InstallSource::Market {
                market_key,
                listing_id,
                version,
                source,
            } if market_key == &self.market_key && version == &source.version => {
                require_id(*listing_id, "listingId")?;
                Ok((*listing_id, version.to_string()))
            }
            _ => Err(ContractError::new(
                "installSource",
                "explicit matching market selection required",
            )),
        }
    }

    /// Validate a single page without remembering or persisting catalog state.
    /// The caller must pass the cursor used for this request.
    pub fn validate_page(
        &self,
        after: Option<Uuid>,
        page: &MarketPage,
    ) -> Result<(), ContractError> {
        let mut ids = HashSet::new();
        let mut applications = HashSet::new();
        let mut previous = after;
        for listing in &page.items {
            validate_listing(listing)?;
            if listing.market_key != self.market_key {
                return Err(ContractError::new(
                    "marketKey",
                    "unexpected market authority",
                ));
            }
            if !ids.insert(listing.listing_id)
                || !applications.insert(&listing.frozen_version.source.application)
                || previous.is_some_and(|id| listing.listing_id <= id)
            {
                return Err(ContractError::new(
                    "items",
                    "duplicate identity or non-advancing catalog page",
                ));
            }
            previous = Some(listing.listing_id);
        }
        if let Some(cursor) = page.next_cursor {
            if page.items.last().map(|item| item.listing_id) != Some(cursor) {
                return Err(ContractError::new(
                    "nextCursor",
                    "cursor must identify the last returned listing",
                ));
            }
        }
        Ok(())
    }
}

fn require_id(id: Uuid, path: &str) -> Result<(), ContractError> {
    if id.is_nil() {
        return Err(ContractError::new(path, "non-nil identity required"));
    }
    Ok(())
}

fn validate_listing(listing: &MarketListing) -> Result<(), ContractError> {
    require_id(listing.listing_id, "listingId")?;
    listing.frozen_version.validate()
}
