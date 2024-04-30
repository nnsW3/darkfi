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
    fmt,
    sync::{
        atomic::{AtomicBool, Ordering::SeqCst},
        Arc,
    },
    time::UNIX_EPOCH,
};

use darkfi_serial::{
    async_trait, AsyncDecodable, AsyncEncodable, SerialDecodable, SerialEncodable, VarInt,
};
use log::{debug, error, info, trace};
use rand::{rngs::OsRng, Rng};
use smol::{
    io::{self, AsyncRead, AsyncReadExt, AsyncWriteExt, ReadHalf, WriteHalf},
    lock::Mutex,
    Executor,
};
use url::Url;

use super::{
    dnet::{self, dnetev, DnetEvent},
    hosts::HostColor,
    message,
    message::{VersionMessage, MAGIC_BYTES},
    message_subscriber::{MessageSubscription, MessageSubsystem},
    p2p::P2pPtr,
    session::{Session, SessionBitFlag, SessionWeakPtr, SESSION_ALL, SESSION_REFINE},
    transport::PtStream,
};
use crate::{
    system::{StoppableTask, StoppableTaskPtr, Subscriber, SubscriberPtr, Subscription},
    util::time::NanoTimestamp,
    Error, Result,
};

/// Atomic pointer to async channel
pub type ChannelPtr = Arc<Channel>;

/// Channel debug info
#[derive(Clone, Debug, SerialEncodable, SerialDecodable)]
pub struct ChannelInfo {
    pub resolve_addr: Option<Url>,
    pub connect_addr: Url,
    pub start_time: u64,
    pub id: u32,
}

impl ChannelInfo {
    fn new(resolve_addr: Option<Url>, connect_addr: Url, start_time: u64) -> Self {
        Self { resolve_addr, connect_addr, start_time, id: OsRng.gen() }
    }
}

/// Async channel for communication between nodes.
pub struct Channel {
    /// The reading half of the transport stream
    reader: Mutex<ReadHalf<Box<dyn PtStream>>>,
    /// The writing half of the transport stream
    writer: Mutex<WriteHalf<Box<dyn PtStream>>>,
    /// The message subsystem instance for this channel
    message_subsystem: MessageSubsystem,
    /// Subscriber listening for stop signal for closing this channel
    stop_subscriber: SubscriberPtr<Error>,
    /// Task that is listening for the stop signal
    receive_task: StoppableTaskPtr,
    /// A boolean marking if this channel is stopped
    stopped: AtomicBool,
    /// Weak pointer to respective session
    session: SessionWeakPtr,
    /// The version message of the node we are connected to.
    /// Some if the version exchange has already occurred, None
    /// otherwise.
    version: Mutex<Option<Arc<VersionMessage>>>,
    /// Channel debug info
    pub info: ChannelInfo,
}

impl Channel {
    /// Sets up a new channel. Creates a reader and writer [`PtStream`] and
    /// the message subscriber subsystem. Performs a network handshake on the
    /// subsystem dispatchers.
    pub async fn new(
        stream: Box<dyn PtStream>,
        resolve_addr: Option<Url>,
        connect_addr: Url,
        session: SessionWeakPtr,
    ) -> Arc<Self> {
        let (reader, writer) = io::split(stream);
        let reader = Mutex::new(reader);
        let writer = Mutex::new(writer);

        let message_subsystem = MessageSubsystem::new();
        Self::setup_dispatchers(&message_subsystem).await;

        let version = Mutex::new(None);
        let start_time = UNIX_EPOCH.elapsed().unwrap().as_secs();
        let info = ChannelInfo::new(resolve_addr, connect_addr.clone(), start_time);

        Arc::new(Self {
            reader,
            writer,
            message_subsystem,
            stop_subscriber: Subscriber::new(),
            receive_task: StoppableTask::new(),
            stopped: AtomicBool::new(false),
            session,
            version,
            info,
        })
    }

    /// Perform network handshake for message subsystem dispatchers.
    async fn setup_dispatchers(subsystem: &MessageSubsystem) {
        subsystem.add_dispatch::<message::VersionMessage>().await;
        subsystem.add_dispatch::<message::VerackMessage>().await;
        subsystem.add_dispatch::<message::PingMessage>().await;
        subsystem.add_dispatch::<message::PongMessage>().await;
        subsystem.add_dispatch::<message::GetAddrsMessage>().await;
        subsystem.add_dispatch::<message::AddrsMessage>().await;
    }

    /// Starts the channel. Runs a receive loop to start receiving messages
    /// or handles a network failure.
    pub fn start(self: Arc<Self>, executor: Arc<Executor<'_>>) {
        debug!(target: "net::channel::start()", "START {:?}", self);

        let self_ = self.clone();
        self.receive_task.clone().start(
            self.clone().main_receive_loop(),
            |result| self_.handle_stop(result),
            Error::ChannelStopped,
            executor,
        );

        debug!(target: "net::channel::start()", "END {:?}", self);
    }

    /// Stops the channel.
    /// Notifies all subscribers that the channel has been closed in `handle_stop()`.
    pub async fn stop(&self) {
        debug!(target: "net::channel::stop()", "START {:?}", self);
        self.receive_task.stop().await;
        debug!(target: "net::channel::stop()", "END {:?}", self);
    }

    /// Creates a subscription to a stopped signal.
    /// If the channel is stopped then this will return a ChannelStopped error.
    pub async fn subscribe_stop(&self) -> Result<Subscription<Error>> {
        debug!(target: "net::channel::subscribe_stop()", "START {:?}", self);

        if self.is_stopped() {
            return Err(Error::ChannelStopped)
        }

        let sub = self.stop_subscriber.clone().subscribe().await;

        debug!(target: "net::channel::subscribe_stop()", "END {:?}", self);

        Ok(sub)
    }

    pub fn is_stopped(&self) -> bool {
        self.stopped.load(SeqCst)
    }

    /// Sends a message across a channel. Calls `send_message` that creates
    /// a new payload and sends it over the network transport as a packet.
    /// Returns an error if something goes wrong.
    pub async fn send<M: message::Message>(&self, message: &M) -> Result<()> {
        debug!(
             target: "net::channel::send()", "[START] command={} {:?}",
             M::NAME, self,
        );

        if self.is_stopped() {
            return Err(Error::ChannelStopped)
        }

        // Catch failure and stop channel, return a net error
        if let Err(e) = self.send_message(message).await {
            if self.session.upgrade().unwrap().type_id() & (SESSION_ALL & !SESSION_REFINE) != 0 {
                error!(
                    target: "net::channel::send()", "[P2P] Channel send error for [{:?}]: {}",
                    self, e
                );
            }
            self.stop().await;
            return Err(Error::ChannelStopped)
        }

        debug!(
            target: "net::channel::send()", "[END] command={} {:?}",
            M::NAME, self
        );

        Ok(())
    }

    /// Sends an outbound Message by writing data to the given async stream.
    async fn send_message<M: message::Message>(&self, message: &M) -> Result<()> {
        let command = M::NAME.to_string();
        assert!(!command.is_empty());
        assert!(std::mem::size_of::<usize>() <= std::mem::size_of::<u64>());

        let stream = &mut *self.writer.lock().await;
        let mut name_buffer = Vec::<u8>::new();
        let mut msg_buffer = Vec::<u8>::new();
        let mut written: usize = 0;

        dnetev!(self, SendMessage, {
            chan: self.info.clone(),
            cmd: command,
            time: NanoTimestamp::current_time(),
        });

        trace!(target: "net::channel::send_message()", "Sending magic...");
        written += MAGIC_BYTES.encode_async(stream).await?;

        trace!(target: "net::channel::send_message()", "Sent magic");
        trace!(target: "net::channel::send_message()", "Sending command...");

        // First encode the name to an intermediate buffer.
        M::NAME.to_string().encode_async(&mut name_buffer).await?;

        // Then extract the length of the intermediate buffer as a VarInt
        // and write to the stream. This is the length of the name message.
        // Then encode the name itself to the stream.
        written += VarInt(name_buffer.len() as u64).encode_async(stream).await?;
        written += M::NAME.to_string().encode_async(stream).await?;

        trace!(target: "net::channel::send_message()", "Sent command: {}", M::NAME.to_string());
        trace!(target: "net::channel::send_message()", "Sending payload...");

        // Do the same proceedure for the Message.
        message.encode_async(&mut msg_buffer).await?;

        written += VarInt(msg_buffer.len() as u64).encode_async(stream).await?;
        written += message.encode_async(stream).await?;

        trace!(target: "net::channel::send_message()", "Sent payload {} bytes, total bytes {}",
            msg_buffer.len(), written);

        stream.flush().await?;

        Ok(())
    }

    /// Returns a decoded Message command.
    /// We start by extracting the length from the stream, then allocate
    /// the precise buffer for this length using stream.take(). This provides
    /// a basic DDOS protection.
    pub async fn read_command<R: AsyncRead + Unpin + Send + Sized>(
        &self,
        stream: &mut R,
    ) -> Result<String> {
        // Messages should have a 4 byte header of magic digits.
        // This is used for network debugging.
        let mut magic = [0u8; 4];
        trace!(target: "net::channel::read_command()", "Reading magic...");
        stream.read_exact(&mut magic).await?;

        trace!(target: "net::channel::read_command()", "Read magic {:?}", magic);
        if magic != MAGIC_BYTES {
            error!(target: "net::channel::read_command", "Error: Magic bytes mismatch");
            return Err(Error::MalformedPacket)
        }

        let len = VarInt::decode_async(stream).await.unwrap().0;
        let mut take = stream.take(len);
        let command = String::decode_async(&mut take).await.unwrap();

        Ok(command)
    }

    /// Subscribe to a message on the message subsystem.
    pub async fn subscribe_msg<M: message::Message>(&self) -> Result<MessageSubscription<M>> {
        debug!(
            target: "net::channel::subscribe_msg()", "[START] command={} {:?}",
            M::NAME, self
        );

        let sub = self.message_subsystem.subscribe::<M>().await;

        debug!(
            target: "net::channel::subscribe_msg()", "[END] command={} {:?}",
            M::NAME, self
        );

        sub
    }

    /// Handle network errors. Panic if error passes silently, otherwise
    /// broadcast the error.
    async fn handle_stop(self: Arc<Self>, result: Result<()>) {
        debug!(target: "net::channel::handle_stop()", "[START] {:?}", self);

        self.stopped.store(true, SeqCst);

        match result {
            Ok(()) => panic!("Channel task should never complete without error status"),
            // Send this error to all channel subscribers
            Err(e) => {
                self.stop_subscriber.notify(Error::ChannelStopped).await;
                self.message_subsystem.trigger_error(e).await;
            }
        }

        debug!(target: "net::channel::handle_stop()", "[END] {:?}", self);
    }

    /// Run the receive loop. Start receiving messages or handle network failure.
    async fn main_receive_loop(self: Arc<Self>) -> Result<()> {
        debug!(target: "net::channel::main_receive_loop()", "[START] {:?}", self);

        // Acquire reader lock
        let reader = &mut *self.reader.lock().await;

        // Run loop
        loop {
            let command = match self.read_command(reader).await {
                Ok(command) => command,
                Err(err) => {
                    if Self::is_eof_error(&err) {
                        info!(
                            target: "net::channel::main_receive_loop()",
                            "[P2P] Channel inbound connection {} disconnected",
                            self.address(),
                        );
                    } else if self.session.upgrade().unwrap().type_id() &
                        (SESSION_ALL & !SESSION_REFINE) !=
                        0
                    {
                        error!(
                            target: "net::channel::main_receive_loop()",
                            "[P2P] Read error on channel {}: {}",
                            self.address(), err,
                        );
                    }

                    debug!(
                        target: "net::channel::main_receive_loop()",
                        "Stopping channel {:?}", self
                    );
                    return Err(Error::ChannelStopped)
                }
            };

            dnetev!(self, RecvMessage, {
                chan: self.info.clone(),
                cmd: command.clone(),
                time: NanoTimestamp::current_time(),
            });

            // Send result to our subscribers
            match self.message_subsystem.notify(&command, reader).await {
                Ok(()) => {}
                // If we're getting messages without dispatchers, it's spam.
                Err(Error::MissingDispatcher) => {
                    debug!(target: "net::channel::main_receive_loop()", "Stopping channel {:?}", self);

                    // We will reject further connections from this peer
                    self.ban(self.address()).await;

                    return Err(Error::ChannelStopped)
                }
                Err(_) => unreachable!("You added a new error in notify()"),
            }
        }
    }

    /// Ban a malicious peer and stop the channel.
    pub async fn ban(&self, peer: &Url) {
        debug!(target: "net::channel::ban()", "START {:?}", self);
        let last_seen = UNIX_EPOCH.elapsed().unwrap().as_secs();
        self.p2p().hosts().move_host(peer, last_seen, HostColor::Black).await.unwrap();

        self.stop().await;
        debug!(target: "net::channel::ban()", "STOP {:?}", self);
    }

    /// Returns the relevant socket address for this connection.  If this is
    /// an outbound connection, the transport-processed resolve_addr will
    /// be returned.  Otherwise for inbound connections it will default
    /// to connect_addr.
    pub fn address(&self) -> &Url {
        if self.info.resolve_addr.is_some() {
            self.info.resolve_addr.as_ref().unwrap()
        } else {
            &self.info.connect_addr
        }
    }

    /// Returns the socket address that has undergone transport
    /// processing, if it exists. Returns None otherwise.
    pub fn resolve_addr(&self) -> Option<Url> {
        self.info.resolve_addr.clone()
    }

    /// Return the socket address without transport processing.
    pub fn connect_addr(&self) -> &Url {
        &self.info.connect_addr
    }

    /// Set the VersionMessage of the node this channel is connected
    /// to. Called on receiving a version message in `ProtocolVersion`.
    pub(crate) async fn set_version(&self, version: Arc<VersionMessage>) {
        *self.version.lock().await = Some(version);
    }

    /// Returns the inner [`MessageSubsystem`] reference
    pub fn message_subsystem(&self) -> &MessageSubsystem {
        &self.message_subsystem
    }

    fn session(&self) -> Arc<dyn Session> {
        self.session.upgrade().unwrap()
    }

    pub fn session_type_id(&self) -> SessionBitFlag {
        let session = self.session();
        session.type_id()
    }

    fn p2p(&self) -> P2pPtr {
        self.session().p2p()
    }

    fn is_eof_error(err: &Error) -> bool {
        match err {
            Error::Io(ioerr) => ioerr == &std::io::ErrorKind::UnexpectedEof,
            _ => false,
        }
    }
}

impl fmt::Debug for Channel {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "<Channel addr='{}' id={}>", self.address(), self.info.id)
    }
}
