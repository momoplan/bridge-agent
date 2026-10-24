//! Authenticated consumer access. User credentials never target the central market.
use super::ConsumerFailure;
use super::{resolve_exact, MarketDistribution};
use anyhow::{bail, Context, Result};
use baijimu_cmodel_http::{decode_response, REQUEST_ID_HEADER, TRACE_ID_HEADER};
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
        let response = artifact_response(self.get(url, credential).await?).await?;
        if response.content_length() != Some(expected.size_bytes)
            || response
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                != Some("application/octet-stream")
        {
            let failure = ConsumerFailure::response("安装包响应与所选版本的清单不符。", &response);
            tracing::error!(
                request_id = failure.request_id,
                trace_id = failure.trace_id,
                expected_size = expected.size_bytes,
                actual_size = response.content_length(),
                "consumer artifact metadata does not match selected version"
            );
            return Err(failure.into());
        }
        Ok(response)
    }
}

pub(super) async fn decode<T: DeserializeOwned>(mut response: Response) -> Result<T> {
    let status = response.status();
    let request_id = response
        .headers()
        .get(&REQUEST_ID_HEADER)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let trace_id = response
        .headers()
        .get(&TRACE_ID_HEADER)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    // Pages contain bounded manifests. Never buffer an unbounded error page.
    const MAX_BYTES: usize = 128 * 1024 * 1024;
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|error| {
        tracing::error!(request_id, trace_id, error = ?error.without_url(), "consumer response read failed");
        ConsumerFailure::local("读取应用服务响应失败，请稍后重试。", request_id.clone(), trace_id.clone(), Some(status.as_u16()))
    })? {
        if chunk.len() > MAX_BYTES - bytes.len() {
            return Err(ConsumerFailure::local("应用服务响应超过大小限制。", request_id, trace_id, Some(status.as_u16())).into());
        }
        bytes.extend_from_slice(&chunk);
    }
    let envelope = decode_response::<T>(status, &bytes, "local-app-consumer").map_err(|error| {
        ConsumerFailure::decode(error, request_id.clone(), trace_id.clone(), status.as_u16())
    })?;
    if !envelope.is_success() {
        return Err(ConsumerFailure::downstream(
            envelope.error_code().as_str(),
            envelope.failure_data(),
            status.as_u16(),
            request_id,
            trace_id,
        )
        .into());
    }
    envelope.into_data().ok_or_else(|| {
        ConsumerFailure::local(
            "应用服务响应缺少数据。",
            request_id,
            trace_id,
            Some(status.as_u16()),
        )
        .into()
    })
}

pub(super) async fn artifact_response(response: Response) -> Result<Response> {
    if response.status() != reqwest::StatusCode::OK
        || response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            != Some("application/octet-stream")
    {
        let failure = ConsumerFailure::response("应用服务未返回所选版本的安装包。", &response);
        decode::<serde_json::Value>(response).await?;
        tracing::error!(
            request_id = failure.request_id,
            trace_id = failure.trace_id,
            status = failure.status,
            "consumer artifact response contained JSON success instead of an artifact"
        );
        return Err(failure.into());
    }
    Ok(response)
}

pub(super) async fn send(client: &Client, url: Url, credential: &str) -> Result<Response> {
    if credential.is_empty() || credential.trim() != credential {
        bail!("consumer workspace credential required");
    }
    let mut authorization = HeaderValue::from_str(&format!("Bearer {credential}"))
        .map_err(|_| anyhow::anyhow!("invalid consumer credential"))?;
    authorization.set_sensitive(true);
    let request_id = Uuid::new_v4().to_string();
    let mut response = client
        .get(url)
        .header(reqwest::header::AUTHORIZATION, authorization)
        .header(&REQUEST_ID_HEADER, &request_id)
        .header(&TRACE_ID_HEADER, &request_id)
        .send()
        .await
        .map_err(|error| {
            tracing::error!(request_id, error = ?error.without_url(), "consumer transport failed");
            ConsumerFailure::local(
                "无法连接应用服务，请检查网络或稍后重试。",
                Some(request_id.clone()),
                Some(request_id.clone()),
                None,
            )
        })?;
    if !response.headers().contains_key(&REQUEST_ID_HEADER) {
        response
            .headers_mut()
            .insert(REQUEST_ID_HEADER, request_id.parse().expect("UUID header"));
    }
    if !response.headers().contains_key(&TRACE_ID_HEADER) {
        response
            .headers_mut()
            .insert(TRACE_ID_HEADER, request_id.parse().expect("UUID header"));
    }
    Ok(response)
}
