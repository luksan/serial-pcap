#![allow(dead_code)]

mod x328_chat_test;

use anyhow::{Context, Result};
use etherparse::PacketBuilder;
use rpcap::write::{PcapWriter, WriteOptions};
use rpcap::CapturedPacket;

use std::io::Write;

const LINKTYPE_IPV4: u32 = 228; // https://www.tcpdump.org/linktypes.html

fn test_eth<T: Write>(mut pcap_writer: PcapWriter<T>) -> Result<()> {
    let payload = b"hello world";
    let mut buf = Vec::<u8>::with_capacity(100);

    for _ in 0..10 {
        let builder = PacketBuilder::ipv4([127, 0, 0, 1], [127, 0, 0, 2], 254).udp(422, 1422);
        builder.write(&mut buf, payload).unwrap();

        let c = CapturedPacket {
            time: std::time::SystemTime::now(),
            data: buf.as_slice(),
            orig_len: buf.len(),
        };
        pcap_writer.write(&c).unwrap();
        buf.clear();
    }
    Ok(())
}

fn write_packet<T: Write>(
    pcap_writer: &mut PcapWriter<T>,
    data: &[u8],
    master: bool,
) -> Result<()> {
    let builder = if master {
        PacketBuilder::ipv4([127, 0, 0, 1], [127, 0, 0, 2], 254).udp(422, 1422)
    } else {
        PacketBuilder::ipv4([127, 0, 0, 2], [127, 0, 0, 1], 254).udp(422, 1422)
    };
    let mut buf = Vec::with_capacity(builder.size(data.len()));
    builder.write(&mut buf, data)?;
    pcap_writer
        .write(&CapturedPacket {
            time: std::time::SystemTime::now(),
            data: buf.as_slice(),
            orig_len: buf.len(),
        })
        .context("Failed to write packet to pcap file")
}

fn test_chatter<T: Write>(pcap_writer: &mut PcapWriter<T>) -> Result<()> {
    let mut chat = x328_chat_test::Chat::new();
    let mut buf_a = Vec::new();
    let mut buf_b = Vec::new();

    let mut cnt = 0;

    while chat.next(&mut buf_a, &mut buf_b).is_ok() {
        cnt += 1;
        if !buf_a.is_empty() {
            write_packet(pcap_writer, buf_a.as_slice(), true)?;
        }
        if !buf_b.is_empty() {
            write_packet(pcap_writer, buf_b.as_slice(), false)?;
        }
        buf_a.clear();
        buf_b.clear();
        if cnt > 10 {
            break;
        }
    }
    Ok(())
}

fn main() -> Result<()> {
    let mut pcap_writer = PcapWriter::new(
        std::fs::File::create("test.pcap").context("Failed to create pcap file")?,
        WriteOptions {
            snaplen: 200, // maximum packet size in file
            linktype: LINKTYPE_IPV4,
            high_res_timestamps: false,
            non_native_byte_order: false,
        },
    )
    .context("Failed to create pcap writer")?;
    // test_eth(pcap_writer)
    test_chatter(&mut pcap_writer)
}
