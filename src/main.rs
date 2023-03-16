#![allow(dead_code)]

use std::time::Duration;

use anyhow::{bail, Context, Result};
use bytes::BytesMut;
use clap::Parser;
use tokio::io::AsyncReadExt;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use tokio::time::timeout;
use tokio_serial::{DataBits, Parity, SerialPort, SerialPortBuilderExt, SerialStream, StopBits};

use serial_pcap::{SerialPacketWriter, UartTxChannel};

#[derive(Parser, Debug)]
struct CmdlineOpts {
    #[clap(long, value_name = "SERIAL_PORT")]
    /// One side of the UART
    ctrl: String,
    /// The other side of the UART
    #[clap(long, value_name = "SERIAL_PORT")]
    node: String,

    /// The pcap filename, will be overwritten if it exists
    pcap_file: String,
}

#[derive(Debug)]
struct UartData {
    ch_name: UartTxChannel,
    data: BytesMut,
}

async fn read_uart(
    mut uart: SerialStream,
    ch_name: UartTxChannel,
    tx: UnboundedSender<UartData>,
) -> Result<()> {
    let mut buf = BytesMut::with_capacity(200);
    loop {
        buf.reserve(20);
        match uart.read_buf(&mut buf).await {
            Ok(0) => {
                bail!("Read from {} returned 0 bytes.", uart.name().unwrap())
            }
            Ok(_) => {
                tx.send(UartData {
                    ch_name,
                    data: buf.split(),
                })?;
            }
            err => {
                err?;
            }
        }
    }
}

async fn open_uart(uart: &str) -> Result<SerialStream> {
    tokio_serial::new(uart, 9600)
        .parity(Parity::Even)
        .data_bits(DataBits::Seven)
        .stop_bits(StopBits::One)
        .open_native_async()
        .with_context(|| format!("Failed to open serial port {uart}."))
}

async fn record_streams(
    mut writer: SerialPacketWriter,
    mut rx: UnboundedReceiver<UartData>,
) -> Result<()> {
    let mut prev_ch = UartTxChannel::Node;
    let mut buf = BytesMut::new();
    let mut time = std::time::SystemTime::now();
    let read_timeout = Duration::from_millis(30);
    loop {
        let msg = if !buf.is_empty() {
            let r = timeout(read_timeout, rx.recv()).await;
            if r.is_err() || matches!(r, Ok(Some(UartData{ch_name, ..})) if ch_name != prev_ch ) {
                tokio::task::block_in_place(|| {
                    writer.write_packet_time(buf.as_ref(), prev_ch, time)
                })?;
                buf = BytesMut::new();
            }
            match r {
                Ok(msg) => msg,
                Err(_) => continue,
            }
        } else {
            rx.recv().await
        };

        let Some(UartData{ch_name, data}) = msg else { return Ok(()) };
        if buf.is_empty() {
            time = std::time::SystemTime::now();
            prev_ch = ch_name;
            buf = data;
        } else {
            buf.unsplit(data);
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = CmdlineOpts::parse();
    let pcap_writer = SerialPacketWriter::new(args.pcap_file)?;
    let ctrl = open_uart(&args.ctrl).await?;
    let node = open_uart(&args.node).await?;

    let (tx, rx) = unbounded_channel();
    tokio::spawn(record_streams(pcap_writer, rx));
    tokio::spawn(read_uart(node, UartTxChannel::Node, tx.clone()));
    read_uart(ctrl, UartTxChannel::Ctrl, tx).await?;

    Ok(())
}
