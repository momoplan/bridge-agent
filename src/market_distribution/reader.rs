//! Authenticated consumer access. User credentials never target the central market.
use super::{resolve_exact, MarketDistribution};
use anyhow::{bail, Context, Result};
use baijimu_cmodel_core::CModelResponse;
use local_app_contract::{InstallSource, MarketListing, MarketPage};
use reqwest::{header::HeaderValue, Client, Response, Url};
use serde::de::DeserializeOwned;
use std::time::Duration;
use uuid::Uuid;

pub struct MarketReader {
    distribution: MarketDistribution,
    client: Client,
}

impl MarketReader {
    /// Discover the market key from the authenticated consumer Owner, never from a domain.
    pub async fn connect(
        base_url: &str,
        workspace: u64,
        credential: &str,
        timeout: Duration,
    ) -> Result<Self> {
        let mut endpoint = Url::parse(base_url).context("invalid consumer API base")?;
        if workspace == 0
            || timeout.is_zero()
            || endpoint.scheme() != "https"
            || endpoint.host_str().is_none()
            || !endpoint.username().is_empty()
            || endpoint.password().is_some()
            || endpoint.query().is_some()
            || endpoint.fragment().is_some()
            || base_url.trim() != base_url
        {
            bail!("explicit consumer HTTPS API base and workspace are required");
        }
        endpoint
            .path_segments_mut()
            .map_err(|_| anyhow::anyhow!("invalid consumer base"))?
            .pop_if_empty()
            .push("context");
        endpoint
            .query_pairs_mut()
            .append_pair("workspaceId", &workspace.to_string());
        let client = Client::builder()
            .https_only(true)
            .redirect(reqwest::redirect::Policy::none())
            .timeout(timeout)
            .build()?;
        let response = send(&client, endpoint, credential).await?;
        let market_key: local_app_contract::MarketKey = decode(response).await?;
        Ok(Self {
            distribution: MarketDistribution::new(market_key, base_url, workspace)?,
            client,
        })
    }

    pub fn new(distribution: MarketDistribution, timeout: Duration) -> Result<Self> {
        if timeout.is_zero() {
            bail!("consumer request timeout must be positive");
        }
        Ok(Self {
            distribution,
            client: Client::builder()
                .https_only(true)
                .redirect(reqwest::redirect::Policy::none())
                .timeout(timeout)
                .build()?,
        })
    }

    async fn get(&self, url: Url, credential: &str) -> Result<Response> {
        send(&self.client, url, credential).await
    }

    pub async fn page(&self, credential: &str, after: Option<Uuid>) -> Result<MarketPage> {
        let page = decode(
            self.get(self.distribution.page_url(after)?, credential)
                .await?,
        )
        .await?;
        self.distribution.validate_page(after, &page)?;
        Ok(page)
    }

    pub async fn version(
        &self,
        credential: &str,
        selection: &InstallSource,
    ) -> Result<MarketListing> {
        let listing = decode(
            self.get(self.distribution.version_url(selection)?, credential)
                .await?,
        )
        .await?;
        resolve_exact(selection, &listing)?;
        Ok(listing)
    }

    /// The response is streamed by the installer, which owns destination and byte accounting.
    pub async fn artifact(
        &self,
        credential: &str,
        selection: &InstallSource,
        artifact: Uuid,
    ) -> Result<Response> {
        let listing = self.version(credential, selection).await?;
        let url = self
            .distribution
            .artifact_url(selection, &listing, artifact)?;
        let expected = listing
            .frozen_version
            .content
            .artifacts
            .iter()
            .find(|entry| entry.artifact_id == artifact)
            .context("artifact missing from selected version")?;
        let response = self.get(url, credential).await?;
        if response.content_length() != Some(expected.size_bytes)
            || response
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                != Some("application/octet-stream")
        {
            bail!("consumer artifact does not match selected frozen version");
        }
        Ok(response)
    }
}

async fn decode<T: DeserializeOwned>(response: Response) -> Result<T> {
    let bytes = response
        .bytes()
        .await
        .context("cannot read consumer response")?;
    let envelope: CModelResponse<T> = local_app_contract::decode(&bytes)?;
    if !envelope.is_success() {
        bail!("consumer local-app service rejected the market request");
    }
    envelope
        .into_data()
        .context("consumer response has no data")
}

async fn send(client: &Client, url: Url, credential: &str) -> Result<Response> {
    if credential.is_empty() || credential.trim() != credential {
        bail!("consumer workspace credential required");
    }
    let mut authorization = HeaderValue::from_str(&format!("Bearer {credential}"))
        .map_err(|_| anyhow::anyhow!("invalid consumer credential"))?;
    authorization.set_sensitive(true);
    let response = client
        .get(url)
        .header(reqwest::header::AUTHORIZATION, authorization)
        .send()
        .await
        .map_err(|_| anyhow::anyhow!("consumer local-app service unavailable"))?;
    if response.status() != reqwest::StatusCode::OK {
        bail!(
            "consumer local-app service returned HTTP {}",
            response.status()
        );
    }
    Ok(response)
}
