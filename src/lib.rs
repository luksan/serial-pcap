use std::fs::File;
use std::path::Path;

use anyhow::{Context, Result};
use arrayvec::ArrayVec;
use etherparse::PacketBuilder;
use rpcap::write::{PcapWriter, WriteOptions};
use rpcap::CapturedPacket;

const LINKTYPE_IPV4: u32 = 228; // https://www.tcpdump.org/linktypes.html
const MAX_PACKET_LEN: usize = 200; // the maximum size of a packet in the pcap file

pub struct SerialPacketWriter {
    pcap_writer: PcapWriter<File>,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum UartTxChannel {
    Ctrl,
    Node,
}

impl SerialPacketWriter {
    pub fn new(filename: impl AsRef<Path>) -> Result<Self> {
        let filename = filename.as_ref();
        let pcap_writer = PcapWriter::new(
            File::create(filename).context("Failed to create pcap file {filename}")?,
            WriteOptions {
                snaplen: MAX_PACKET_LEN, // maximum packet size in file
                linktype: LINKTYPE_IPV4,
                high_res_timestamps: false,
                non_native_byte_order: false,
            },
        )
        .context("Couldn't create PcapWriter.")?;
        Ok(Self { pcap_writer })
    }

    pub fn write_packet(&mut self, data: &[u8], channel: UartTxChannel) -> Result<()> {
        let builder = if channel == UartTxChannel::Ctrl {
            PacketBuilder::ipv4([127, 0, 0, 1], [127, 0, 0, 2], 254).udp(422, 1422)
        } else {
            PacketBuilder::ipv4([127, 0, 0, 2], [127, 0, 0, 1], 254).udp(422, 1422)
        };
        let mut buf = ArrayVec::<u8, MAX_PACKET_LEN>::new();
        builder.write(&mut buf, data)?;
        self.pcap_writer
            .write(&CapturedPacket {
                time: std::time::SystemTime::now(),
                data: buf.as_slice(),
                orig_len: buf.len(),
            })
            .context("Failed to write packet to pcap file")
    }
}
