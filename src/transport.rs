use std::collections::HashMap;
use std::io;
use std::net::{SocketAddr, UdpSocket};

use crate::osc::{self, OscPacket};

pub const MAX_PACKET_BATCH: usize = 4096;

#[derive(Debug)]
pub struct ReceivedPacket {
    pub packet: OscPacket,
    pub peer: SocketAddr,
}

#[derive(Debug, Default)]
pub struct DrainResult {
    pub packets: Vec<ReceivedPacket>,
    pub malformed: Vec<(SocketAddr, String)>,
    pub saturated: bool,
    pub connection_reset: bool,
}

pub fn drain_latest(socket: &UdpSocket, buffer: &mut [u8]) -> io::Result<DrainResult> {
    let mut result = DrainResult::default();
    let mut indices = HashMap::<String, usize>::new();
    let mut received = 0;

    while received < MAX_PACKET_BATCH {
        match socket.recv_from(buffer) {
            Ok((length, peer)) => {
                received += 1;
                match osc::parse_packet(&buffer[..length]) {
                    Ok(packet) => {
                        if let Some(index) = indices.get(&packet.address).copied() {
                            result.packets[index] = ReceivedPacket { packet, peer };
                        } else {
                            indices.insert(packet.address.clone(), result.packets.len());
                            result.packets.push(ReceivedPacket { packet, peer });
                        }
                    }
                    Err(error) => result.malformed.push((peer, error.to_string())),
                }
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
            Err(error) if error.kind() == io::ErrorKind::ConnectionReset => {
                result.connection_reset = true;
                break;
            }
            Err(error) => return Err(error),
        }
    }

    result.saturated = received == MAX_PACKET_BATCH;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::osc::OscArgument;

    fn encode_float(address: &str, value: f32) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(address.as_bytes());
        bytes.push(0);
        while !bytes.len().is_multiple_of(4) {
            bytes.push(0);
        }
        bytes.extend_from_slice(b",f\0\0");
        bytes.extend_from_slice(&value.to_bits().to_be_bytes());
        bytes
    }

    #[test]
    fn coalesces_two_hundred_updates_to_the_latest_zero() {
        let receiver = UdpSocket::bind("127.0.0.1:0").unwrap();
        receiver.set_nonblocking(true).unwrap();
        let sender = UdpSocket::bind("127.0.0.1:0").unwrap();
        let destination = receiver.local_addr().unwrap();
        let address = "/avatar/parameters/joycon/channel/left/force";

        for index in 0..200 {
            let value = (index + 1) as f32 / 200.0;
            sender
                .send_to(&encode_float(address, value), destination)
                .unwrap();
        }
        sender
            .send_to(&encode_float(address, 0.0), destination)
            .unwrap();

        let mut buffer = [0_u8; 2048];
        let result = drain_latest(&receiver, &mut buffer).unwrap();
        assert_eq!(result.packets.len(), 1);
        assert_eq!(result.packets[0].packet.address, address);
        assert_eq!(
            result.packets[0].packet.arguments,
            vec![OscArgument::Float(0.0)]
        );
        assert!(!result.saturated);
    }

    #[test]
    fn keeps_distinct_addresses_independent_and_ignores_malformed_packets() {
        let receiver = UdpSocket::bind("127.0.0.1:0").unwrap();
        receiver.set_nonblocking(true).unwrap();
        let sender = UdpSocket::bind("127.0.0.1:0").unwrap();
        let destination = receiver.local_addr().unwrap();

        sender
            .send_to(&encode_float("/channel/left", 0.25), destination)
            .unwrap();
        sender.send_to(b"not osc", destination).unwrap();
        sender
            .send_to(&encode_float("/channel/right", 0.75), destination)
            .unwrap();

        let mut buffer = [0_u8; 2048];
        let result = drain_latest(&receiver, &mut buffer).unwrap();
        assert_eq!(result.packets.len(), 2);
        assert_eq!(result.malformed.len(), 1);
    }
}
