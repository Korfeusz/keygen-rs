use crate::certificate::CartificateFileResponse;
use crate::client::{Client, Response};
use crate::errors::Error;
use crate::machine_file::MachineFile;
use crate::KeygenResponseData;
use chrono::{DateTime, Utc};
use futures::future::{BoxFuture, FutureExt};
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::Arc;
use std::time::Duration;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct MachineAttributes {
    pub fingerprint: String,
    pub name: Option<String>,
    pub platform: Option<String>,
    pub hostname: Option<String>,
    pub cores: Option<i32>,
    #[serde(rename = "requireHeartbeat")]
    pub require_heartbeat: bool,
    #[serde(rename = "heartbeatStatus")]
    pub heartbeat_status: String,
    #[serde(rename = "heartbeatDuration")]
    pub heartbeat_duration: Option<i32>,
    pub created: DateTime<Utc>,
    pub updated: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct MachineResponse {
    pub data: KeygenResponseData<MachineAttributes>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct MachinesResponse {
    pub data: Vec<KeygenResponseData<MachineAttributes>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Machine {
    pub id: String,
    pub fingerprint: String,
    pub name: Option<String>,
    pub platform: Option<String>,
    pub hostname: Option<String>,
    pub cores: Option<i32>,
    #[serde(rename = "requireHeartbeat")]
    pub require_heartbeat: bool,
    #[serde(rename = "heartbeatStatus")]
    pub heartbeat_status: String,
    #[serde(rename = "heartbeatDuration")]
    pub heartbeat_duration: Option<i32>,
    pub created: DateTime<Utc>,
    pub updated: DateTime<Utc>,
}

pub struct MachineCheckoutOpts {
    pub ttl: Option<i64>,
    pub include: Option<Vec<String>>,
}

impl Machine {
    pub(crate) fn from(data: KeygenResponseData<MachineAttributes>) -> Machine {
        Machine {
            id: data.id,
            fingerprint: data.attributes.fingerprint,
            name: data.attributes.name,
            platform: data.attributes.platform,
            hostname: data.attributes.hostname,
            cores: data.attributes.cores,
            require_heartbeat: data.attributes.require_heartbeat,
            heartbeat_status: data.attributes.heartbeat_status,
            heartbeat_duration: data.attributes.heartbeat_duration,
            created: data.attributes.created,
            updated: data.attributes.updated,
        }
    }

    pub async fn deactivate(&self) -> Result<(), Error> {
        let client = Client::default();
        let _response = client
            .delete::<(), serde_json::Value>(&format!("machines/{}", self.id), None::<&()>)
            .await?;
        Ok(())
    }

    pub async fn checkout(&self, options: &MachineCheckoutOpts) -> Result<MachineFile, Error> {
        let client = Client::default();
        let mut query = json!({
            "encrypt": 1,
            "include": "license.entitlements"
        });

        if let Some(ttl) = options.ttl {
            query["ttl"] = ttl.into();
        }

        if let Some(ref include) = options.include {
            query["include"] = json!(include.join(","));
        }

        let response = client
            .post(
                &format!("machines/{}/actions/check-out", self.id),
                None::<&()>,
                Some(&query),
            )
            .await?;

        let machine_file_response: CartificateFileResponse = serde_json::from_value(response.body)?;
        let machine_file = MachineFile::from(machine_file_response.data);
        Ok(machine_file)
    }

    pub async fn ping(&self) -> Result<Machine, Error> {
        let client: Client = Client::default();
        let response: Response<MachineResponse> = client
            .post(
                &format!("machines/{}/actions/ping", self.id),
                None::<&()>,
                None::<&()>,
            )
            .await?;
        let machine = Machine::from(response.body.data);
        Ok(machine)
    }

    pub fn monitor(
        self: Arc<Self>,
        heartbeat_interval: Duration,
        tx: Option<Sender<Result<Machine, Error>>>,
        cancel_rx: Option<Receiver<()>>,
    ) -> BoxFuture<'static, ()> {
        async move {
            let send = |result: Result<Machine, Error>| {
                if let Some(tx) = &tx {
                    tx.send(result).unwrap();
                }
            };

            let mut interval_stream = futures::stream::unfold((), move |_| {
                let delay = futures_timer::Delay::new(heartbeat_interval);
                Box::pin(async move {
                    delay.await;
                    Some(((), ()))
                })
            });

            send(self.ping().await);
            while interval_stream.next().await.is_some() {
              match cancel_rx {
                Some(ref rx) => {
                  if rx.try_recv().is_ok() {
                    break;
                  }
                }
                None => {}
              }
              send(self.ping().await);
            }
        }
        .boxed()
    }
}
