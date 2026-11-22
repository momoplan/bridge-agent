//! Device-initiated calls use the current registered Relay connection.
use anyhow::{anyhow, bail, Context, Result};
use relay::contracts::{cloud_module::CloudModuleCall, device_events::InstalledLocalAppSource};
use relay::device_channel::v1::{self, device_frame};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};

#[derive(Default, Clone)]
pub(crate) struct CloudCalls(Arc<Mutex<Session>>);

#[derive(Default)]
struct Session {
    channel: Option<mpsc::Sender<v1::DeviceFrame>>,
    pending: Vec<Pending>,
}

struct Pending {
    source: InstalledLocalAppSource,
    id: String,
    reply: oneshot::Sender<Result<Receipt>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Request {
    request_id: String,
    module_key: String,
    method: String,
    arguments: Map<String, Value>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Receipt {
    request_id: String,
    success: bool,
    data: Option<Value>,
    error: Option<crate::protocol::InvokeError>,
}

impl CloudCalls {
    pub(crate) fn connect(&self, channel: mpsc::Sender<v1::DeviceFrame>) {
        let mut session = self.0.lock().unwrap();
        if session
            .channel
            .as_ref()
            .is_some_and(|old| old.same_channel(&channel))
        {
            return;
        }
        *session = Session {
            channel: Some(channel),
            pending: Vec::new(),
        };
    }

    pub(crate) fn disconnect(&self, channel: &mpsc::Sender<v1::DeviceFrame>) {
        let mut session = self.0.lock().unwrap();
        if session
            .channel
            .as_ref()
            .is_some_and(|c| c.same_channel(channel))
        {
            *session = Session::default();
        }
    }

    pub(crate) async fn call(
        &self,
        source: InstalledLocalAppSource,
        req: Request,
    ) -> Result<Receipt> {
        let call = CloudModuleCall {
            request_id: req.request_id,
            installed_source: source.clone(),
            module_key: req.module_key,
            method: req.method,
            arguments: req.arguments,
        };
        call.validate().map_err(|e| anyhow!(e))?;
        let waiting = {
            let mut session = self.0.lock().unwrap();
            let channel = session
                .channel
                .as_ref()
                .context("device has no registered Relay connection")?;
            if session.pending.len() >= 32 {
                bail!("device cloud call capacity exceeded");
            }
            if session
                .pending
                .iter()
                .any(|p| p.source == source && p.id == call.request_id)
            {
                bail!("request is already in flight");
            }
            let id = call.request_id.clone();
            let frame = v1::DeviceFrame {
                frame: Some(device_frame::Frame::CloudModuleCall(call.into())),
            };
            channel
                .try_send(frame)
                .map_err(|_| anyhow!("Relay connection cannot accept the call"))?;
            let (reply, waiting) = oneshot::channel();
            session.pending.push(Pending { source, id, reply });
            waiting
        };
        tokio::time::timeout(Duration::from_secs(60), waiting)
            .await
            .context("cloud call timed out; outcome unknown")?
            .context("Relay disconnected; cloud call outcome unknown")?
    }

    pub(crate) fn receive(
        &self,
        channel: &mpsc::Sender<v1::DeviceFrame>,
        reply: v1::CloudModuleReply,
    ) -> Result<()> {
        let mut session = self.0.lock().unwrap();
        if !session
            .channel
            .as_ref()
            .is_some_and(|c| c.same_channel(channel))
        {
            return Ok(());
        }
        let source: InstalledLocalAppSource = reply
            .installed_source
            .context("missing cloud receipt source")?
            .into();
        let result = reply.result.context("missing cloud call receipt")?;
        if let Some(index) = session
            .pending
            .iter()
            .position(|p| p.source == source && p.id == result.request_id)
        {
            let pending = session.pending.swap_remove(index);
            drop(session);
            let _ = pending.reply.send(Receipt::decode(result));
        }
        Ok(())
    }
}

impl Receipt {
    fn decode(result: v1::InvokeResult) -> Result<Self> {
        let data = result
            .data_json
            .map(|raw| serde_json::from_slice(&raw))
            .transpose()?;
        let error = result
            .error
            .map(|e| -> Result<_> {
                Ok(crate::protocol::InvokeError {
                    code: e.code,
                    message: e.message,
                    public_details: e
                        .public_details_json
                        .map(|raw| serde_json::from_slice(&raw))
                        .transpose()?,
                })
            })
            .transpose()?;
        if result.success == error.is_some() {
            bail!("inconsistent cloud receipt");
        }
        Ok(Self {
            request_id: result.request_id,
            success: result.success,
            data,
            error,
        })
    }
}

#[cfg(test)]
mod tests;
