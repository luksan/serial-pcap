use std::fs::File;
use std::path::Path;

use anyhow::{bail, Context, Result};
use arrayvec::ArrayVec;
use bytes::{Buf, BytesMut};
use chrono::Utc;
use etherparse::{PacketBuilder, SlicedPacket, TransportSlice};
use rpcap::read::PcapReader;
use rpcap::write::{PcapWriter, WriteOptions};
use rpcap::CapturedPacket;
use tokio_serial::{DataBits, Parity, SerialPortBuilderExt, SerialStream, StopBits};

const LINKTYPE_IPV4: u32 = 228; // https://www.tcpdump.org/linktypes.html
const MAX_PACKET_LEN: usize = 200; // the maximum size of a packet in the pcap file

pub struct SerialPacketWriter<W: std::io::Write> {
    pcap_writer: PcapWriter<W>,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[repr(u16)]
pub enum UartTxChannel {
    Ctrl = 422,
    Node = 1422,
}

const CTRL: u16 = UartTxChannel::Ctrl as _;
const NODE: u16 = UartTxChannel::Node as _;

impl SerialPacketWriter<File> {
    pub fn new_file(filename: impl AsRef<Path>) -> Result<Self> {
        let filename = filename.as_ref();
        let writer = File::create(filename).context("Failed to create pcap file {filename}")?;
        SerialPacketWriter::<File>::new(writer)
    }
}

impl<W: std::io::Write> SerialPacketWriter<W> {
    pub fn new(writer: W) -> Result<Self> {
        let pcap_writer = PcapWriter::new(
            writer,
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
        self.write_packet_time(data, channel, std::time::SystemTime::now())
    }

    pub fn write_packet_time(
        &mut self,
        data: &[u8],
        channel: UartTxChannel,
        time: std::time::SystemTime,
    ) -> Result<()> {
        let (ip, ports) = match channel {
            UartTxChannel::Ctrl => (([127, 0, 0, 1], [127, 0, 0, 2]), (CTRL, NODE)),
            UartTxChannel::Node => (([127, 0, 0, 2], [127, 0, 0, 1]), (NODE, CTRL)),
        };

        for data in data.chunks(MAX_PACKET_LEN - 32) {
            // 32 is the UDP header length
            let builder = PacketBuilder::ipv4(ip.0, ip.1, 254).udp(ports.0, ports.1);
            let mut buf = ArrayVec::<u8, MAX_PACKET_LEN>::new();
            builder
                .write(&mut buf, data)
                .context("Writing to packet memory buffer failed.")?;
            self.pcap_writer
                .write(&CapturedPacket {
                    time,
                    data: buf.as_slice(),
                    orig_len: buf.len(),
                })
                .context("Failed to write packet to pcap file")?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct SerialPacket {
    pub ch: UartTxChannel,
    pub data: BytesMut,
    pub time: chrono::DateTime<Utc>,
}

impl<R: std::io::Read> Iterator for SerialPacketReader<R> {
    type Item = Result<SerialPacket>;

    fn next(&mut self) -> Option<Self::Item> {
        self.next_packet().transpose()
    }
}

pub struct SerialPacketReader<R: std::io::Read> {
    pcap_reader: PcapReader<R>,
    ctrl_buf: BytesMut,
    node_buf: BytesMut,
    pub stream_time: std::time::SystemTime,
}

impl<R: std::io::Read> SerialPacketReader<R> {
    pub fn new(reader: R) -> Result<Self> {
        Ok(Self {
            pcap_reader: PcapReader::new(reader)
                .context("Failed to create PcapReader.")?
                .1,
            ctrl_buf: Default::default(),
            node_buf: Default::default(),
            stream_time: std::time::SystemTime::now(),
        })
    }

    pub fn read_bytes(&mut self, ch: UartTxChannel, max_len: usize) -> Result<BytesMut> {
        if self.get_buffer(ch).is_empty() {
            self.fill_buffer(ch)?;
        }
        let buf = self.get_buffer(ch);
        let len = max_len.min(buf.len());
        Ok(buf.split_to(len))
    }

    pub fn next_packet(&mut self) -> Result<Option<SerialPacket>> {
        let Some(pkt) = self.pcap_reader.next().context("Pcap read error")? else { return Ok(None) };
        let time = chrono::DateTime::from(pkt.time);
        assert_eq!(pkt.orig_len, pkt.data.len());
        let pkt = SlicedPacket::from_ip(pkt.data).context("Failed to slice packet")?;
        let Some(TransportSlice::Udp(udp_hdr)) = pkt.transport else { bail!("Failed to find UDP header in pkt.")};
        let source_port = udp_hdr.source_port();
        let ch = match source_port {
            CTRL => UartTxChannel::Ctrl,
            NODE => UartTxChannel::Node,
            1442 => UartTxChannel::Node, // anyhow..
            _ => bail!("Incorrect UDP source port {source_port}."),
        };
        Ok(Some(SerialPacket {
            ch,
            data: BytesMut::from(pkt.payload),
            time,
        }))
    }

    pub fn reader(&mut self, ch: UartTxChannel) -> impl std::io::Read + '_ {
        ReadPcapReadImpl { reader: self, ch }
    }

    fn get_buffer(&mut self, ch: UartTxChannel) -> &mut BytesMut {
        match ch {
            UartTxChannel::Ctrl => &mut self.ctrl_buf,
            UartTxChannel::Node => &mut self.node_buf,
        }
    }

    fn fill_buffer(&mut self, ch: UartTxChannel) -> Result<()> {
        while self.get_buffer(ch).is_empty() && self.extend_one_pkt()? {}
        Ok(())
    }

    fn extend_one_pkt(&mut self) -> Result<bool> {
        let Some(pkt) = self.next_packet()? else { return Ok(false) };
        let buf = match pkt.ch {
            UartTxChannel::Ctrl => &mut self.ctrl_buf,
            UartTxChannel::Node => &mut self.node_buf,
        };
        buf.unsplit(pkt.data);
        Ok(true)
    }
}

impl SerialPacketReader<File> {
    pub fn from_file(filename: impl AsRef<Path>) -> Result<Self> {
        let filename = filename.as_ref();
        Self::new(File::open(filename).context("Failed to open {filename}")?)
    }
}

struct ReadPcapReadImpl<'a, R: std::io::Read> {
    reader: &'a mut SerialPacketReader<R>,
    ch: UartTxChannel,
}

impl<R: std::io::Read> std::io::Read for ReadPcapReadImpl<'_, R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if let Err(e) = self.reader.fill_buffer(self.ch) {
            return Err(std::io::Error::new(std::io::ErrorKind::Other, e));
        }
        self.reader.get_buffer(self.ch).reader().read(buf)
    }
}

/// Open a tokio_serial UART with the correct settings for X3.28
pub fn open_async_uart(uart: &str) -> Result<SerialStream> {
    tokio_serial::new(uart, 9600)
        .parity(Parity::Even)
        .data_bits(DataBits::Seven)
        .stop_bits(StopBits::One)
        .open_native_async()
        .with_context(|| format!("Failed to open serial port {uart}."))
}
