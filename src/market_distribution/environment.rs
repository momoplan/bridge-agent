//! Private versions are read only from the explicitly authorized source environment.
use super::{
    reader::{decode, send},
    validate_endpoint,
};
use anyhow::{ensure, Result};
use local_app_contract::{EnvironmentKey, FrozenVersion, SourceVersion};
use reqwest::{header, Client, ClientBuilder, Response, Url};
use std::time::Duration;
use uuid::Uuid;

pub struct EnvironmentReader {
    environment: EnvironmentKey,
    endpoint: Url,
    client: Client,
}

impl EnvironmentReader {
    pub fn new(
        environment: EnvironmentKey,
        endpoint: &str,
        workspace: u64,
        timeout: Duration,
    ) -> Result<Self> {
        // Reuse the same HTTPS authority validation; no market lookup is performed.
        validate_endpoint(endpoint)?;
        Ok(Self {
            environment,
            endpoint: Url::parse(endpoint)?,
            client: workspace_client(workspace, timeout)?.build()?,
        })
    }

    pub fn version_url(&self, source: &SourceVersion) -> Result<Url> {
        ensure!(
            source.application.environment_key == self.environment,
            "private application source differs from the authorized environment"
        );
        let mut url = self.endpoint.clone();
        url.path_segments_mut()
            .expect("validated HTTPS base")
            .pop_if_empty()
            .extend([
                source.application.app_id.as_str(),
                "versions",
                &source.version.to_string(),
            ]);
        Ok(url)
    }

    pub async fn version(&self, credential: &str, source: &SourceVersion) -> Result<FrozenVersion> {
        let frozen: FrozenVersion =
            decode(send(&self.client, self.version_url(source)?, credential).await?).await?;
        frozen.validate()?;
        ensure!(
            frozen.source == *source,
            "source environment returned a different application version"
        );
        Ok(frozen)
    }

    pub async fn artifact(
        &self,
        credential: &str,
        frozen: &FrozenVersion,
        artifact: Uuid,
    ) -> Result<Response> {
        frozen.validate()?;
        ensure!(
            frozen
                .content
                .artifacts
                .iter()
                .any(|item| item.artifact_id == artifact),
            "artifact is not in the frozen version"
        );
        let mut url = self.version_url(&frozen.source)?;
        url.path_segments_mut()
            .expect("validated HTTPS base")
            .extend(["artifacts", &artifact.to_string()]);
        send(&self.client, url, credential).await
    }
}

// The PAT can authorize more than one workspace; the source owner must receive
// the device's selected recipient workspace for both metadata and artifact reads.
fn workspace_client(workspace: u64, timeout: Duration) -> Result<ClientBuilder> {
    ensure!(workspace > 0, "consumer workspace required");
    ensure!(!timeout.is_zero(), "request timeout must be positive");
    let mut headers = header::HeaderMap::new();
    headers.insert("x-workspace-id", workspace.to_string().parse()?);
    Ok(Client::builder()
        .https_only(true)
        .redirect(reqwest::redirect::Policy::none())
        .default_headers(headers)
        .timeout(timeout))
}

#[cfg(test)]
#[path = "environment_tests.rs"]
mod tests;
