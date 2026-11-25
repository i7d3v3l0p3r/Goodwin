use std::time::Duration;

use anyhow::{Context, Result, ensure};
use prost::Message;
use tokio::{io::AsyncReadExt, net::TcpStream, time::timeout};

use crate::wake::SAMPLE_RATE as RATE;

#[cfg(test)]
mod tests;
#[cfg(test)]
pub(crate) use tests::write_samples;

pub(super) const FRAME_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Clone, Copy)]
pub(crate) enum Framing {
    Typed,
    Legacy,
}

#[derive(Clone, PartialEq, Message)]
struct Empty {}

#[derive(Clone, PartialEq, Message)]
struct Subscription {
    #[prost(message, optional, tag = "1")]
    main: Option<Empty>,
    #[prost(message, optional, tag = "3")]
    vqe: Option<Empty>,
}

#[derive(Clone, PartialEq, Message)]
pub(crate) struct Envelope {
    #[prost(message, optional, tag = "158")]
    legacy_audio: Option<Channels>,
    #[prost(message, optional, tag = "159")]
    legacy_subscription: Option<Subscription>,
    #[prost(bytes, optional, tag = "252")]
    payload: Option<Vec<u8>>,
    #[prost(uint32, optional, tag = "253")]
    kind: Option<u32>,
}

#[derive(Clone, PartialEq, Message)]
pub(super) struct Channels {
    #[prost(message, repeated, tag = "1")]
    channels: Vec<Channel>,
}

#[derive(Clone, PartialEq, Message)]
struct Channel {
    #[prost(string, tag = "2")]
    name: String,
    #[prost(bytes, tag = "3")]
    data: Vec<u8>,
    #[prost(int32, tag = "7")]
    rate: i32,
    #[prost(int32, tag = "8")]
    format: i32,
}

impl Framing {
    pub(crate) fn subscription(self) -> Envelope {
        match self {
            Self::Typed => Envelope {
                payload: Some(
                    Subscription {
                        main: Some(Empty {}),
                        ..Subscription::default()
                    }
                    .encode_to_vec(),
                ),
                kind: Some(159),
                ..Envelope::default()
            },
            Self::Legacy => Envelope {
                legacy_subscription: Some(Subscription {
                    vqe: Some(Empty {}),
                    ..Subscription::default()
                }),
                ..Envelope::default()
            },
        }
    }

    pub(super) fn alternate(self) -> Self {
        match self {
            Self::Typed => Self::Legacy,
            Self::Legacy => Self::Typed,
        }
    }
}

#[cfg_attr(feature = "hotpath-alloc", hotpath::measure)]
pub(super) async fn receive(
    socket: &mut TcpStream,
    data: &mut Vec<u8>,
) -> Result<Option<Channels>> {
    timeout(FRAME_TIMEOUT, async {
        let length = socket.read_u32_le().await? as usize;
        ensure!(
            (1..=262_144).contains(&length),
            "Invalid station frame length"
        );
        data.resize(length, 0);
        socket.read_exact(data).await?;
        let envelope = Envelope::decode(data.as_slice())?;
        if let (Some(158), Some(payload)) = (envelope.kind, envelope.payload) {
            return Ok(Some(Channels::decode(payload.as_slice())?));
        }
        Ok(envelope.legacy_audio)
    })
    .await
    .context("Station microphone timed out")?
}

pub(super) fn capture_channel(channels: &Channels) -> Option<&[u8]> {
    ["vqe_0", "vqe"].into_iter().find_map(|name| {
        channels
            .channels
            .iter()
            .find(|channel| {
                channel.name == name
                    && (channel.rate == 0 || channel.rate == RATE as i32)
                    && matches!(channel.format, 0 | 1)
                    && !channel.data.is_empty()
                    && channel.data.len() % 2 == 0
            })
            .map(|channel| channel.data.as_slice())
    })
}
