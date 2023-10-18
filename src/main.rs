#![allow(dead_code)]

use std::time::Duration;

use anyhow::{bail, Context, Result};
use bytes::BytesMut;
use clap::Parser;
use tokio::io::AsyncReadExt;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use tokio::task::JoinHandle;
use tokio::time::timeout;
use tokio_serial::SerialStream;
use tracing::{info, trace, Level};

use serial_pcap::{open_async_uart, SerialPacketWriter, UartTxChannel, TRIG_BYTE};

#[derive(Parser, Debug)]
struct CmdlineOpts {
    #[clap(long, value_name = "SERIAL_PORT")]
    /// One side of the UART
    ctrl: String,

    /// The other side of the UART
    #[clap(long, value_name = "SERIAL_PORT")]
    node: Option<String>,

    /// The ctrl and node bytes are received on the same UART, with the node bytes having MSB set high.
    #[clap(long = "muxed-stream")]
    muxed: bool,

    /// The pcap filename, will be overwritten if it exists
    pcap_file: String,
}

#[derive(Debug)]
struct UartData {
    ch_name: UartTxChannel,
    data: BytesMut,
    time_received: std::time::SystemTime,
}

#[tracing::instrument(skip(uart, tx))]
async fn read_uart(
    mut uart: SerialStream,
    ch_name: UartTxChannel,
    tx: UnboundedSender<UartData>,
) -> Result<()> {
    let mut buf = BytesMut::with_capacity(1);
    loop {
        buf.reserve(1);
        match uart.read_buf(&mut buf).await {
            Ok(0) => {
                info!("Zero length read");
                bail!("Read from {ch_name:?} returned 0 bytes.");
            }
            Ok(len) => {
                trace!("Received {len} bytes.");
                tx.send(UartData {
                    ch_name,
                    data: buf.split(),
                    time_received: std::time::SystemTime::now(),
                })?;
            }
            err => {
                info!("UART read returned with error {err:?}");
                err.with_context(|| format!("Read error from UART '{ch_name:?}'."))?;
            }
        }
    }
}

async fn read_muxed_uart(mut uart: SerialStream, tx: UnboundedSender<UartData>) -> Result<()> {
    let mut buf = BytesMut::with_capacity(1);
    'read: loop {
        buf.reserve(1);
        match uart.read_buf(&mut buf).await {
            Ok(0) => {
                info!("Zero length read");
                bail!("Read from muxed uart returned 0 bytes.");
            }
            Ok(_len) => {
                let time_received = std::time::SystemTime::now();
                // trace!("Received {_len} bytes.");
                while !buf.is_empty() {
                    let Some(byte) = buf.iter().find(|&&b| b != TRIG_BYTE) else {
                        continue 'read;
                    };
                    let ch = *byte & 0x80;
                    let ch_name = match ch == 0x80 {
                        false => UartTxChannel::Node,
                        true => UartTxChannel::Ctrl,
                    };

                    // \n == Trigger event
                    let l = buf
                        .iter()
                        .take_while(|&b| b & 0x80 == ch || *b == TRIG_BYTE)
                        .count();
                    let mut data = buf.split_to(l);
                    if data.as_ref().contains(&TRIG_BYTE) {
                        info!("Trigger found in data stream");
                    }
                    data.iter_mut().for_each(|b| *b &= 0x7f); // clear bit 8
                    tx.send(UartData {
                        ch_name,
                        data,
                        time_received,
                    })?;
                }
            }
            err => {
                info!("UART read returned with error {err:?}");
                err.with_context(|| "Read error from muxed UART.".to_string())?;
            }
        }
    }
}

#[tracing::instrument(skip_all)]
async fn record_streams<W: std::io::Write>(
    mut writer: SerialPacketWriter<W>,
    mut rx: UnboundedReceiver<UartData>,
) -> Result<()> {
    let mut prev_ch = UartTxChannel::Node;
    let mut buf = BytesMut::new();
    let mut time = std::time::SystemTime::now();
    let read_timeout = Duration::from_millis(5);

    trace!("Stream recorder running");
    loop {
        let msg = if !buf.is_empty() {
            let r = timeout(read_timeout, rx.recv()).await;
            if r.is_err() || matches!(r, Ok(Some(UartData{ch_name, ref data, ..})) if ch_name != prev_ch || data[0] == 0x04 ) {
                tokio::task::block_in_place(|| {
                    writer.write_packet_time(buf.as_ref(), prev_ch, time)
                })
                .context("write_packet_time() returned an error.")?;
                buf = BytesMut::new();
            }
            match r {
                Ok(msg) => msg,
                Err(_) => continue,
            }
        } else {
            rx.recv().await
        };

        // destructure the received message, or stop if the tx side is closed
        let Some(UartData {
            ch_name,
            data,
            time_received,
        }) = msg
        else {
            return Ok(());
        };
        if buf.is_empty() {
            time = time_received;
            prev_ch = ch_name;
            buf = data;
        } else {
            buf.unsplit(data);
        }
    }
}

async fn await_task<E: Into<anyhow::Error>>(handle: &mut JoinHandle<Result<(), E>>) -> Result<()> {
    match handle.await {
        Ok(Ok(result)) => Ok(result),
        Ok(Err(err)) => bail!(err),
        Err(err) => bail!(err),
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = CmdlineOpts::parse();

    let subscriber = tracing_subscriber::FmtSubscriber::builder()
        .with_max_level(Level::TRACE)
        .finish();
    tracing::subscriber::set_global_default(subscriber)?;

    info!("Logging at INFO level.");
    trace!("Logging at TRACE level.");

    let pcap_writer = SerialPacketWriter::new_file(args.pcap_file)?;
    let ctrl = open_async_uart(&args.ctrl)?;

    let (tx, rx) = unbounded_channel();
    let mut recorder = tokio::spawn(record_streams(pcap_writer, rx));

    let res;
    if args.muxed {
        tokio::select! {
            r = await_task(&mut recorder) => { return r.context("Error in stream recorder task."); }
            r = read_muxed_uart(ctrl, tx) => {res = r;}
            _ = tokio::signal::ctrl_c() => { res = Ok(()) }
        }
    } else {
        let node = open_async_uart(args.node.as_ref().unwrap())?;
        tokio::select! {
            r = await_task(&mut recorder) => { return r.context("Error in stream recorder task."); }
            r = read_uart(ctrl, UartTxChannel::Ctrl, tx.clone()) => {res = r;}
            r = read_uart(node, UartTxChannel::Node, tx) => {res = r;}
            _ = tokio::signal::ctrl_c() => { res = Ok(()) }
        }
    }

    info!("Waiting for the recorder to stop.");

    // Stop the recorder task by dropping all the channel tx handles
    await_task(&mut recorder).await?;

    info!("Shutdown complete.");
    res.context("Error returned from main()")
}
