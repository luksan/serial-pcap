#![allow(dead_code)]

use anyhow::{bail, Context, Result};
use chrono::{DateTime, Utc};
use clap::Parser;
use std::iter::Peekable;

use x328_proto::master::SendData;
use x328_proto::node::Node;
use x328_proto::{addr, master, Address, NodeState, Parameter, Value};

use serial_pcap::{SerialPacket, SerialPacketReader, UartTxChannel};

#[derive(Copy, Clone, Debug)]
enum BusCommand {
    Read {
        addr: Address,
        param: Parameter,
    },
    Write {
        addr: Address,
        param: Parameter,
        value: Value,
    },
}

fn parse_x328_uart<R: std::io::Read>(uart_reader: &mut SerialPacketReader<R>) -> Result<()> {
    let pkt_iter = &mut uart_reader.peekable();

    let mut ctrl = master::Master::new();
    let mut node = Node::new(addr(0));
    let mut token = node.reset();
    loop {
        match node.state(token) {
            NodeState::ReceiveData(r) => {
                let Some(pkt) = pkt_iter.next().transpose()? else {
                    return Ok(());
                };
                let data: &[u8] = pkt.data.as_ref();
                if pkt.ch == UartTxChannel::Node {
                    // A node transmitted unexpectedly
                    println!("Unexpected data on node tx channel {data:?}");
                    token = node.reset();
                    loop {
                        // resync
                        match pkt_iter.peek() {
                            None => return Ok(()),
                            Some(Ok(SerialPacket {
                                ch: UartTxChannel::Ctrl,
                                data,
                                ..
                            })) if data[0] == 0x04 => break,
                            _ => {}
                        }
                        pkt_iter.next();
                    }
                    continue;
                }
                if data.is_empty() {
                    return Ok(());
                }
                token = r.receive_data(data.as_ref());
            }
            NodeState::SendData(s) => {
                token = s.data_sent();
            }
            NodeState::ReadParameter(r) => {
                let resp = receive_node_response::<Value>(
                    ctrl.read_parameter(r.address(), r.parameter()),
                    pkt_iter,
                );
                println!(
                    " Read {:?}@{:?} => {resp:?}",
                    // pkt.time,
                    r.parameter(),
                    r.address()
                );
                token = r.send_read_failed();
            }
            NodeState::WriteParameter(w) => {
                let resp = receive_node_response::<_>(
                    ctrl.write_parameter(w.address(), w.parameter(), w.value()),
                    pkt_iter,
                );
                println!(
                    " Write {:?} to {:?}@{:?} => {resp:?}",
                    // pkt.time,
                    w.value(),
                    w.parameter(),
                    w.address()
                );
                token = match resp {
                    Ok((_time, ())) => w.write_ok(),
                    Err(_) => w.write_error(),
                };
            }
        };
    }
}

fn receive_node_response<Response>(
    mut recv: impl SendData<Response = Response>,
    pkt_iter: &mut Peekable<impl Iterator<Item = Result<SerialPacket>>>,
) -> Result<(DateTime<Utc>, Response)> {
    let recv = recv.data_sent();

    if matches!(
        pkt_iter.peek(),
        Some(Ok(SerialPacket {
            ch: UartTxChannel::Ctrl,
            ..
        }))
    ) {
        bail!("Bus controller timed out")
    }
    match pkt_iter.next() {
        Some(Ok(pkt)) => Ok((
            pkt.time,
            recv.receive_data(pkt.data.as_ref())
                .context("Not enough data in packet")??,
        )),
        Some(Err(err)) => Err(err),
        None => bail!("No more packets."),
    }
}

#[derive(Parser, Debug)]
struct CmdlineOpts {
    /// The pcap filename to read the UART data from
    pcap_file: String,
}

fn main() -> Result<()> {
    let args = CmdlineOpts::parse();

    let filename = &args.pcap_file;
    let file = std::fs::File::open(filename).context("Failed to open {filename}.")?;
    let mut uart_reader = SerialPacketReader::new(file)?;
    parse_x328_uart(&mut uart_reader)
}
