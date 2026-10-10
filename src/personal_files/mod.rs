//! Device-authorized personal attachments. Workspace File Service owns storage,
//! reservations, permissions and stable file IDs; Relay only carries tool results.
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use url::Url;

mod credentials;
mod http;

#[derive(Clone)]
pub(crate) struct PersonalFileClient {
    base_url: String,
    environment_key: Option<String>,
    workspace_id: Option<u64>,
    device_id: String,
    timeout_secs: u64,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct FileReference {
    pub file_id: String,
    pub path: String,
    pub file_name: String,
    pub mime_type: String,
    pub size_bytes: u64,
    pub source: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PrepareRequest<'a> {
    file_name: &'a str,
    mime_type: &'a str,
    source: &'a str,
    size_bytes: u64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct UploadTicket {
    upload_id: String,
    file_id: String,
    upload_url: String,
    upload_expires_at: u128,
    method: String,
    headers: BTreeMap<String, String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CompleteRequest<'a> {
    upload_id: &'a str,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PersonalFileReference {
    pub contract_version: &'static str,
    pub workspace_id: u64,
    pub user_id: u64,
    pub file: FileReference,
}

impl PersonalFileClient {
    pub fn new(config: &crate::config::AgentConfig) -> Self {
        Self {
            base_url: crate::config::environment::api_base(&config.platform.base_url),
            environment_key: config.platform.environment_key.clone(),
            workspace_id: config.platform.workspace_id,
            device_id: config.relay.agent_id.clone(),
            timeout_secs: config.upload.timeout_secs,
        }
    }

    fn endpoint(&self, operation: &str) -> Result<Url> {
        let mut base = Url::parse(&self.base_url).context("invalid platform API base")?;
        require_https(&base)?;
        anyhow::ensure!(
            base.query().is_none(),
            "platform API base must not contain a query"
        );
        base.set_path(&format!(
            "{}/partner/v1/workspace-file-service/api/personal-files/{operation}",
            base.path().trim_end_matches('/')
        ));
        Ok(base)
    }

    pub async fn upload(
        &self,
        file_name: &str,
        mime_type: &str,
        source: &str,
        bytes: Vec<u8>,
    ) -> Result<PersonalFileReference> {
        let credential = credentials::read(&credentials::path()?, self)?;
        let workspace_id = self.workspace_id.context("device workspace missing")?;
        let transport = http::FileTransport::new(self, &credential.token, workspace_id)?;
        let request = PrepareRequest {
            file_name,
            mime_type,
            source,
            size_bytes: bytes.len() as u64,
        };
        let file = upload_file(&transport, &request, bytes).await?;
        Ok(PersonalFileReference {
            contract_version: "1.0.0",
            workspace_id,
            user_id: credential.user_id,
            file,
        })
    }
}

fn require_https(url: &Url) -> Result<()> {
    anyhow::ensure!(
        url.scheme() == "https"
            && url.host_str().is_some()
            && url.username().is_empty()
            && url.password().is_none()
            && url.fragment().is_none(),
        "file transport requires an HTTPS URL without embedded credentials or fragment"
    );
    Ok(())
}

fn validate_ticket(ticket: &UploadTicket) -> Result<()> {
    uuid::Uuid::parse_str(&ticket.upload_id).context("invalid uploadId")?;
    uuid::Uuid::parse_str(&ticket.file_id).context("invalid fileId")?;
    anyhow::ensure!(ticket.method == "PUT", "unsupported personal upload method");
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_millis();
    anyhow::ensure!(
        ticket.upload_expires_at > now,
        "personal upload ticket expired"
    );
    Ok(())
}

trait UploadTransport {
    async fn prepare(&self, request: &PrepareRequest<'_>) -> Result<UploadTicket>;
    async fn put(&self, ticket: &UploadTicket, bytes: Vec<u8>) -> Result<()>;
    async fn complete(&self, upload_id: &str) -> Result<FileReference>;
}

async fn upload_file(
    transport: &impl UploadTransport,
    request: &PrepareRequest<'_>,
    bytes: Vec<u8>,
) -> Result<FileReference> {
    let ticket = transport.prepare(request).await?;
    validate_ticket(&ticket)?;
    transport.put(&ticket, bytes).await?;
    let file = transport.complete(&ticket.upload_id).await?;
    anyhow::ensure!(
        file.file_id == ticket.file_id
            && file.size_bytes == request.size_bytes
            && file.mime_type == request.mime_type,
        "personal upload completion returned inconsistent file metadata"
    );
    Ok(file)
}

#[cfg(test)]
mod tests;
