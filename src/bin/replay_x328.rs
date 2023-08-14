#![allow(dead_code)]

use anyhow::{bail, Context, Result};
use clap::Parser;
use x328_proto::master::Error::ProtocolError;
use x328_proto::master::SendData;
use x328_proto::node::Node;
use x328_proto::{addr, master, Address, NodeState, Parameter, Value};

use serial_pcap::{SerialPacketReader, UartTxChannel};

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
    let mut ctrl = master::Master::new();
    let mut node = Node::new(addr(0));
    let mut token = node.reset();
    loop {
        match node.state(token) {
            NodeState::ReceiveData(r) => {
                let data = uart_reader.read_bytes(UartTxChannel::Ctrl, 1)?;
                if data.is_empty() {
                    return Ok(());
                }
                token = r.receive_data(data.as_ref());
            }
            NodeState::SendData(s) => {
                token = s.data_sent();
            }
            NodeState::ReadParameter(r) => {
                let resp = receive_node_response(
                    ctrl.read_parameter(r.address(), r.parameter()),
                    uart_reader,
                );
                println!("Read {:?}@{:?} => {resp:?}", r.parameter(), r.address());
                token = r.send_reply_ok(resp?);
            }
            NodeState::WriteParameter(w) => {
                let resp = receive_node_response(
                    ctrl.write_parameter(w.address(), w.parameter(), w.value()),
                    uart_reader,
                );
                println!(
                    "Write {:?} to {:?}@{:?} => {resp:?}",
                    w.value(),
                    w.parameter(),
                    w.address()
                );
                token = match resp {
                    Ok(()) => w.write_ok(),
                    Err(_) => w.write_error(),
                };
                resp?;
            }
        };
    }
}

fn receive_node_response<Response, R: std::io::Read>(
    mut recv: impl SendData<Response = Response>,
    uart_reader: &mut SerialPacketReader<R>,
) -> Result<Response> {
    let recv = recv.data_sent();
    loop {
        let data = uart_reader.read_bytes(UartTxChannel::Node, 1)?;
        if data.is_empty() {
            bail!("No data");
        }
        // println!("Node tx {:2x}", data[0]);
        if data[0] == 0 {
            continue;
        }; // Skip NUL bytes FIXME: x328_proto should handle this
        if let Some(value) = recv.receive_data(data.as_ref()) {
            match value {
                Err(ProtocolError) => {
                    println!("Node tx resync event.");
                    continue;
                }
                r => return r.context("Receiver error"),
            };
        }
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
