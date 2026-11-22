use tokio::io::AsyncWriteExt;

use super::*;
use crate::station::capture::VAD_SAMPLES;

#[test]
fn station_subscription_has_documented_wire_layout() {
    assert_eq!(
        Framing::Typed.subscription().encode_to_vec(),
        [0xe2, 0x0f, 2, 0x0a, 0, 0xe8, 0x0f, 0x9f, 1]
    );
    assert_eq!(
        Framing::Legacy.subscription().encode_to_vec(),
        [0xfa, 9, 2, 0x1a, 0]
    );
}

pub(in crate::station) async fn write_samples(socket: &mut TcpStream, sample: i16, frames: usize) {
    let channels = Channels {
        channels: vec![Channel {
            name: "vqe_0".into(),
            data: sample.to_le_bytes().repeat(VAD_SAMPLES * frames),
            rate: RATE as i32,
            format: 1,
        }],
    };
    let message = Envelope {
        payload: Some(channels.encode_to_vec()),
        kind: Some(158),
        ..Envelope::default()
    }
    .encode_to_vec();
    socket.write_u32_le(message.len() as u32).await.unwrap();
    socket.write_all(&message).await.unwrap();
}
