#![allow(dead_code)]

#[allow(unused_imports)]
use anyhow::{Context, Result};

use serial_pcap::{SerialPacketWriter, UartTxChannel};

mod x328_chat_test;

fn test_chatter(pcap: &mut SerialPacketWriter) -> Result<()> {
    let mut chat = x328_chat_test::Chat::new();
    let mut buf_a = Vec::new();
    let mut buf_b = Vec::new();

    let mut cnt = 0;

    while chat.next(&mut buf_a, &mut buf_b).is_ok() {
        cnt += 1;
        if !buf_a.is_empty() {
            pcap.write_packet(buf_a.as_slice(), UartTxChannel::Ctrl)?;
        }
        if !buf_b.is_empty() {
            pcap.write_packet(buf_b.as_slice(), UartTxChannel::Node)?;
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
    let mut pcap_writer = SerialPacketWriter::new("test.pcap")?;
    test_chatter(&mut pcap_writer)
}
