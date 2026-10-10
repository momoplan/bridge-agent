use super::*;
use reqwest::{Client, Response};
use serde::de::DeserializeOwned;
use std::time::Duration;

pub(super) struct FileTransport<'a> {
    client: Client,
    owner: &'a PersonalFileClient,
    token: &'a str,
    workspace_id: u64,
}

impl<'a> FileTransport<'a> {
    pub fn new(owner: &'a PersonalFileClient, token: &'a str, workspace_id: u64) -> Result<Self> {
        let client = Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(owner.timeout_secs))
            .build()?;
        Ok(Self {
            client,
            owner,
            token,
            workspace_id,
        })
    }

    async fn post<I: Serialize, O: DeserializeOwned>(
        &self,
        operation: &str,
        body: &I,
    ) -> Result<O> {
        let response = self
            .client
            .post(self.owner.endpoint(operation)?)
            .bearer_auth(self.token)
            .header("x-workspace-id", self.workspace_id)
            .json(body)
            .send()
            .await
            .map_err(reqwest::Error::without_url)
            .with_context(|| format!("personal file {operation} request failed"))?;
        decode(response, operation).await
    }
}

impl UploadTransport for FileTransport<'_> {
    async fn prepare(&self, request: &PrepareRequest<'_>) -> Result<UploadTicket> {
        self.post("direct-uploads/prepare", request).await
    }

    async fn put(&self, ticket: &UploadTicket, bytes: Vec<u8>) -> Result<()> {
        let url = Url::parse(&ticket.upload_url).context("invalid upload URL")?;
        require_https(&url)?;
        let mut upload = self.client.put(url).body(bytes);
        for (name, value) in &ticket.headers {
            anyhow::ensure!(
                !matches!(
                    name.to_ascii_lowercase().as_str(),
                    "authorization" | "cookie" | "host"
                ),
                "forbidden upload ticket header"
            );
            upload = upload.header(name, value);
        }
        let response = upload
            .send()
            .await
            .map_err(reqwest::Error::without_url)
            .context("personal image transfer failed")?;
        anyhow::ensure!(
            response.status().is_success(),
            "personal image transfer failed: HTTP {}",
            response.status()
        );
        Ok(())
    }

    async fn complete(&self, upload_id: &str) -> Result<FileReference> {
        self.post("direct-uploads/complete", &CompleteRequest { upload_id })
            .await
    }
}

pub(super) async fn decode<T: DeserializeOwned>(response: Response, operation: &str) -> Result<T> {
    let status = response.status();
    let bytes = response
        .bytes()
        .await
        .map_err(reqwest::Error::without_url)
        .context("cannot read personal file response")?;
    baijimu_cmodel_http::decode_optional_data(status, &bytes, operation)?
        .context("personal file success response has no data")
}
