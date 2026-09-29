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
    /// Bind to the configured authenticated consumer endpoint, not a separate market identity.
    pub async fn connect(
        base_url: &str,
        workspace: u64,
        _credential: &str,
        timeout: Duration,
    ) -> Result<Self> {
        Self::new(MarketDistribution::new(base_url, workspace)?, timeout)
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

pub(super) async fn decode<T: DeserializeOwned>(response: Response) -> Result<T> {
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

pub(super) async fn send(client: &Client, url: Url, credential: &str) -> Result<Response> {
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
