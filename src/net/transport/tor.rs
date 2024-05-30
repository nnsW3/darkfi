/* This file is part of DarkFi (https://dark.fi)
 *
 * Copyright (C) 2020-2024 Dyne.org foundation
 *
 * This program is free software: you can redistribute it and/or modify
 * it under the terms of the GNU Affero General Public License as
 * published by the Free Software Foundation, either version 3 of the
 * License, or (at your option) any later version.
 *
 * This program is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
 * GNU Affero General Public License for more details.
 *
 * You should have received a copy of the GNU Affero General Public License
 * along with this program.  If not, see <https://www.gnu.org/licenses/>.
 */

use std::{
    io::{self, ErrorKind},
    pin::Pin,
    sync::Arc,
    time::Duration,
};

use arti_client::{
    config::{onion_service::OnionServiceConfigBuilder, BoolOrAuto},
    DataStream, StreamPrefs, TorClient,
};
use async_trait::async_trait;
use futures::{
    future::{select, Either},
    pin_mut,
    stream::StreamExt,
    Stream,
};
use log::{debug, error, info, warn};
use smol::{
    lock::{Mutex, OnceCell},
    Timer,
};
use tor_cell::relaycell::msg::Connected;
use tor_error::ErrorReport;
use tor_hsservice::{HsNickname, RendRequest, RunningOnionService};
use tor_proto::stream::IncomingStreamRequest;
use tor_rtcompat::PreferredRuntime;
use url::Url;

use super::{PtListener, PtStream};

/// A static for `TorClient` reusability
static TOR_CLIENT: OnceCell<TorClient<PreferredRuntime>> = OnceCell::new();

/// Tor Dialer implementation
#[derive(Debug, Clone)]
pub struct TorDialer;

impl TorDialer {
    /// Instantiate a new [`TorDialer`] object
    pub(crate) async fn new() -> io::Result<Self> {
        Ok(Self {})
    }

    /// Internal dial function
    pub(crate) async fn do_dial(
        &self,
        host: &str,
        port: u16,
        conn_timeout: Option<Duration>,
    ) -> io::Result<DataStream> {
        debug!(target: "net::tor::do_dial", "Dialing {}:{} with Tor...", host, port);

        // Initialize or fetch the static TOR_CLIENT that should be reused in
        // the Tor dialer
        let client = match TOR_CLIENT
            .get_or_try_init(|| async {
                debug!(target: "net::tor::do_dial", "Bootstrapping...");
                TorClient::builder().create_bootstrapped().await
            })
            .await
        {
            Ok(client) => client,
            Err(e) => {
                warn!("{}", e.report());
                return Err(io::Error::new(
                    ErrorKind::Other,
                    "Internal Tor error, see logged warning",
                ))
            }
        };

        let mut stream_prefs = StreamPrefs::new();
        stream_prefs.connect_to_onion_services(BoolOrAuto::Explicit(true));

        // If a timeout is configured, run both the connect and timeout futures
        // and return whatever finishes first. Otherwise, wait on the connect future.
        let connect = client.connect_with_prefs((host, port), &stream_prefs);

        match conn_timeout {
            Some(t) => {
                let timeout = Timer::after(t);
                pin_mut!(timeout);
                pin_mut!(connect);

                match select(connect, timeout).await {
                    Either::Left((Ok(stream), _)) => Ok(stream),

                    Either::Left((Err(e), _)) => {
                        warn!("{}", e.report());
                        Err(io::Error::new(
                            ErrorKind::Other,
                            "Internal Tor error, see logged warning",
                        ))
                    }

                    Either::Right((_, _)) => Err(io::ErrorKind::TimedOut.into()),
                }
            }

            None => {
                match connect.await {
                    Ok(stream) => Ok(stream),
                    Err(e) => {
                        // Extract error reports (i.e. very detailed debugging)
                        // from arti-client in order to help debug Tor connections.
                        // https://docs.rs/arti-client/latest/arti_client/#reporting-arti-errors
                        // https://gitlab.torproject.org/tpo/core/arti/-/issues/1086
                        warn!("{}", e.report());
                        Err(io::Error::new(
                            ErrorKind::Other,
                            "Internal Tor error, see logged warning",
                        ))
                    }
                }
            }
        }
    }
}

/// Tor Listener implementation
#[derive(Clone, Debug)]
pub struct TorListener;

impl TorListener {
    /// Instantiate a new [`TorListener`]
    pub async fn new() -> io::Result<Self> {
        Ok(Self {})
    }

    /// Internal listen function
    pub(crate) async fn do_listen(&self, port: u16) -> io::Result<TorListenerIntern> {
        // Initialize or fetch the static TOR_CLIENT that should be reused in
        // the Tor dialer
        let client = match TOR_CLIENT
            .get_or_try_init(|| async {
                debug!(target: "net::tor::do_dial", "Bootstrapping...");
                TorClient::builder().create_bootstrapped().await
            })
            .await
        {
            Ok(client) => client,
            Err(e) => {
                warn!("{}", e.report());
                return Err(io::Error::new(
                    ErrorKind::Other,
                    "Internal Tor error, see logged warning",
                ))
            }
        };

        let hs_nick = HsNickname::new("darkfi_tor".to_string()).unwrap();

        let hs_config = match OnionServiceConfigBuilder::default().nickname(hs_nick).build() {
            Ok(v) => v,
            Err(e) => {
                error!(
                    target: "net::tor::do_listen",
                    "[P2P] Failed to create OnionServiceConfig: {}", e,
                );
                return Err(io::Error::new(ErrorKind::Other, "Internal Tor error"))
            }
        };

        let (onion_service, rendreq_stream) = match client.launch_onion_service(hs_config) {
            Ok(v) => v,
            Err(e) => {
                error!(
                    target: "net::tor::do_listen",
                    "[P2P] Failed to launch Onion Service: {}", e,
                );
                return Err(io::Error::new(ErrorKind::Other, "Internal Tor error"))
            }
        };

        info!(
            target: "net::tor::do_listen",
            "[P2P] Established Tor listener on tor://{}:{}",
            onion_service.onion_name().unwrap(), port,
        );

        Ok(TorListenerIntern {
            port,
            _onion_service: onion_service,
            rendreq_stream: Mutex::new(Box::pin(rendreq_stream)),
        })
    }
}

/*
/// Internal Tor Listener implementation, used with `PtListener`
pub struct TorListenerIntern<'a> {
    port: u16,
    _onion_service: Arc<RunningOnionService>,
    rendreq_stream: Mutex<BoxStream<'a, RendRequest>>,
}

unsafe impl Sync for TorListenerIntern<'_> {}
*/

pub struct TorListenerIntern {
    port: u16,
    _onion_service: Arc<RunningOnionService>,
    //rendreq_stream: Mutex<BoxStream<'a, RendRequest>>,
    rendreq_stream: Mutex<Pin<Box<dyn Stream<Item = RendRequest> + Send>>>,
}

unsafe impl Sync for TorListenerIntern {}

#[async_trait]
impl PtListener for TorListenerIntern {
    async fn next(&self) -> io::Result<(Box<dyn PtStream>, Url)> {
        let mut rendreq_stream = self.rendreq_stream.lock().await;

        let Some(rendrequest) = rendreq_stream.next().await else {
            return Err(io::Error::new(ErrorKind::ConnectionAborted, "Connection Aborted"))
        };

        let mut streamreq_stream = match rendrequest.accept().await {
            Ok(v) => v,
            Err(e) => {
                error!(
                    target: "net::tor::PtListener::next",
                    "[P2P] Failed accepting Tor RendRequest: {}", e,
                );
                return Err(io::Error::new(ErrorKind::ConnectionAborted, "Connection Aborted"))
            }
        };

        let Some(streamrequest) = streamreq_stream.next().await else {
            return Err(io::Error::new(ErrorKind::ConnectionAborted, "Connection Aborted"))
        };

        // Validate port correctness
        match streamrequest.request() {
            IncomingStreamRequest::Begin(begin) => {
                if begin.port() != self.port {
                    return Err(io::Error::new(ErrorKind::ConnectionAborted, "Connection Aborted"))
                }
            }
            &_ => return Err(io::Error::new(ErrorKind::ConnectionAborted, "Connection Aborted")),
        }

        let stream = match streamrequest.accept(Connected::new_empty()).await {
            Ok(v) => v,
            Err(e) => {
                error!(
                    target: "net::tor::PtListener::next",
                    "[P2P] Failed accepting Tor StreamRequest: {}", e,
                );
                return Err(io::Error::new(ErrorKind::Other, "Internal Tor error"))
            }
        };

        Ok((Box::new(stream), Url::parse(&format!("tor://127.0.0.1:{}", self.port)).unwrap()))
    }
}
