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

use serial_pcap::{open_async_uart, SerialPacketWriter, UartTxChannel};

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

#[tracing::instrument(skip(uart, tx))]
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
                info!("Zero length read");
                bail!("Read from {ch_name:?} returned 0 bytes.");
            }
            Ok(len) => {
                trace!("Received {len} bytes.");
                tx.send(UartData {
                    ch_name,
                    data: buf.split(),
                })?;
            }
            err => {
                info!("UART read returned with error {err:?}");
                err.with_context(|| format!("Read error from UART '{ch_name:?}'."))?;
            }
        }
    }
}

#[tracing::instrument(skip_all)]
async fn record_streams(
    mut writer: SerialPacketWriter,
    mut rx: UnboundedReceiver<UartData>,
) -> Result<()> {
    let mut prev_ch = UartTxChannel::Node;
    let mut buf = BytesMut::new();
    let mut time = std::time::SystemTime::now();
    let read_timeout = Duration::from_millis(30);

    trace!("Stream recorder running");
    loop {
        let msg = if !buf.is_empty() {
            let r = timeout(read_timeout, rx.recv()).await;
            if r.is_err() || matches!(r, Ok(Some(UartData{ch_name, ..})) if ch_name != prev_ch ) {
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
        let Some(UartData{ch_name, data}) = msg else { return Ok(()); };
        if buf.is_empty() {
            time = std::time::SystemTime::now();
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

    let pcap_writer = SerialPacketWriter::new(args.pcap_file)?;
    let ctrl = open_async_uart(&args.ctrl)?;
    let node = open_async_uart(&args.node)?;

    let (tx, rx) = unbounded_channel();
    let mut recorder = tokio::spawn(record_streams(pcap_writer, rx));

    let res;
    tokio::select! {
        r = await_task(&mut recorder) => { return r.context("Error in stream recorder task."); }
        r = read_uart(ctrl, UartTxChannel::Ctrl, tx.clone()) => {res = r;}
        r = read_uart(node, UartTxChannel::Node, tx) => {res = r;}
        _ = tokio::signal::ctrl_c() => { res = Ok(()) }
    }

    info!("Waiting for the recorder to stop.");

    // Stop the recorder task by dropping all the channel tx handles
    await_task(&mut recorder).await?;

    info!("Shutdown complete.");
    res.context("Error returned from main()")
}
