use async_std::{future::timeout, sync::Arc};
use std::time::Duration;

use log::error;
use url::Url;

use crate::{Error, Result};

use super::{Channel, ChannelPtr, SettingsPtr, TcpTransport, Transport, TransportName};

/// Create outbound socket connections.
pub struct Connector {
    settings: SettingsPtr,
}

impl Connector {
    /// Create a new connector with default network settings.
    pub fn new(settings: SettingsPtr) -> Self {
        Self { settings }
    }

    /// Establish an outbound connection.
    pub async fn connect(&self, connect_url: Url) -> Result<ChannelPtr> {
        let transport_name = TransportName::try_from(connect_url.clone())?;
        let result =
            timeout(Duration::from_secs(self.settings.connect_timeout_seconds.into()), async {
                self.connect_channel(connect_url, transport_name).await
            })
            .await?;
        result
    }

    async fn connect_channel(
        &self,
        connect_url: Url,
        transport_name: TransportName,
    ) -> Result<Arc<Channel>> {
        match transport_name {
            TransportName::Tcp(upgrade) => {
                let transport = TcpTransport::new(None, 1024);
                let stream = transport.dial(connect_url.clone());

                if let Err(err) = stream {
                    error!("Setup failed: {}", err);
                    return Err(Error::ConnectFailed)
                }

                let stream = stream?.await;

                if let Err(err) = stream {
                    error!("Connection failed: {}", err);
                    return Err(Error::ConnectFailed)
                }

                let channel = match upgrade {
                    None => Channel::new(Box::new(stream?), connect_url.clone()).await,
                    Some(u) if u == "tls" => {
                        let stream = transport.upgrade_dialer(stream?)?.await;
                        Channel::new(Box::new(stream?), connect_url).await
                    }
                    Some(u) => return Err(Error::UnsupportedTransportUpgrade(u)),
                };

                Ok(channel)
            }
            TransportName::Tor(_upgrade) => todo!(),
        }
    }
}
