//! Private versions are read only from the explicitly authorized source environment.
use super::{
    reader::{artifact_response, decode, send, workspace_client},
    validate_endpoint,
};
use anyhow::{ensure, Result};
use local_app_contract::{
    bundle::{LocatedAppVersion, LocatedFrozenVersion},
    EnvironmentKey, FrozenVersion, SourceVersion,
};
use reqwest::{Client, Response, Url};
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

    pub fn located_version_url(&self, source: &LocatedAppVersion) -> Result<Url> {
        let locator = source.resource_locator.resource_locator();
        ensure!(
            locator.environment_key() == self.environment.as_str(),
            "private application source differs from the authorized environment"
        );
        let mut url = self.endpoint.clone();
        url.path_segments_mut()
            .expect("validated HTTPS base")
            .pop_if_empty()
            .pop()
            .extend([
                "v5",
                "local-apps",
                locator.bundle_id(),
                locator.resource_key(),
                "versions",
                &source.version.to_string(),
            ]);
        Ok(url)
    }

    pub async fn adoption(
        &self,
        credential: &str,
        legacy: &SourceVersion,
    ) -> Result<LocatedAppVersion> {
        let mut url = self.version_url(legacy)?;
        let mut path = url.path_segments_mut().expect("validated HTTPS base");
        path.pop().pop().pop().pop().extend([
            "v5",
            "local-app-adoptions",
            legacy.application.app_id.as_str(),
            "versions",
            &legacy.version.to_string(),
        ]);
        drop(path);
        let located: LocatedAppVersion = decode(send(&self.client, url, credential).await?).await?;
        let locator = located.resource_locator.resource_locator();
        ensure!(
            locator.environment_key() == legacy.application.environment_key.as_str()
                && locator.resource_key() == legacy.application.app_id.as_str()
                && located.version == legacy.version,
            "source environment returned a different legacy adoption",
        );
        Ok(located)
    }

    pub async fn located_version(
        &self,
        credential: &str,
        source: &LocatedAppVersion,
    ) -> Result<LocatedFrozenVersion> {
        let frozen: LocatedFrozenVersion =
            decode(send(&self.client, self.located_version_url(source)?, credential).await?)
                .await?;
        frozen.validate()?;
        ensure!(
            frozen.source == *source,
            "source environment returned a different LOCAL_APP version"
        );
        Ok(frozen)
    }

    pub async fn located_artifact(
        &self,
        credential: &str,
        frozen: &LocatedFrozenVersion,
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
        let mut url = self.located_version_url(&frozen.source)?;
        url.path_segments_mut()
            .expect("validated HTTPS base")
            .extend(["artifacts", &artifact.to_string()]);
        artifact_response(send(&self.client, url, credential).await?).await
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
        artifact_response(send(&self.client, url, credential).await?).await
    }
}

#[cfg(test)]
#[path = "environment_tests.rs"]
mod tests;
