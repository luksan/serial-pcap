#![allow(dead_code)]

use anyhow::{bail, Context, Result};
use bytes::BytesMut;
use chrono::{DateTime, Utc};
use clap::Parser;

use x328_proto::scanner::{ControllerEvent, NodeEvent};
use x328_proto::{Address, Parameter, Value};

use serial_pcap::{SerialPacketReader, UartTxChannel, TRIG_BYTE};

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

struct DataWithTrigger {
    data: BytesMut,
}

impl DataWithTrigger {
    fn new(data: BytesMut) -> Self {
        Self { data }
    }
    fn as_slice(&self) -> &[u8] {
        self.data
            .as_ref()
            .split(|&b| b == TRIG_BYTE)
            .next()
            .unwrap()
    }
    fn is_empty(&self) -> bool {
        self.data.is_empty()
    }
    fn consume(&mut self, len: usize) -> BytesMut {
        self.data.split_to(len)
    }
    fn check_trigger(&mut self) -> bool {
        let Some(trig_pos) = self.data.iter().position(|&b| b == TRIG_BYTE) else {
            return false;
        };
        let tail = self.data.split_off(trig_pos).split_off(1);
        self.data.unsplit(tail);
        true
    }
}

fn parse_x328_uart<R: std::io::Read>(uart_reader: &mut SerialPacketReader<R>) -> Result<()> {
    let pkt_iter = uart_reader;

    let mut scanner = x328_proto::scanner::Scanner::new();
    let mut ctrl_event = None;
    let mut ctrl_time: DateTime<Utc> = DateTime::default();
    'next_packet: loop {
        let Some(pkt) = pkt_iter.next().transpose()? else {
            return Ok(());
        };
        let mut data = DataWithTrigger::new(pkt.data);

        match pkt.ch {
            UartTxChannel::Ctrl => {
                while !data.is_empty() {
                    let slice = data.as_slice();
                    if slice.is_empty() {
                        if data.check_trigger() {
                            println!("Trigger event");
                            continue;
                        }
                        panic!("Empty data slice without trigger.")
                    }
                    let (consumed, event) = scanner.recv_from_ctrl(slice);
                    let consumed = data.consume(consumed);
                    ctrl_event = event;
                    ctrl_time = pkt.time;
                    if ctrl_event.is_none() {
                        if data.check_trigger() {
                            println!("Trigger event");
                            continue;
                        }
                        println!("Consumed without event {consumed:?}");
                        println!("Trailing data in ctrl packet. {:?}", data.as_slice());
                        continue 'next_packet;
                    }
                }
            }
            UartTxChannel::Node => {
                while !data.is_empty() {
                    let slice = data.as_slice();
                    if slice.is_empty() {
                        if data.check_trigger() {
                            println!("Trigger event");
                            continue;
                        }
                        panic!("Empty data slice without trigger.");
                    }
                    let (consumed, event) = scanner.recv_from_node(slice);
                    let consumed = data.consume(consumed);
                    if let Some(event) = event {
                        print!("cmd time: {ctrl_time} ");
                        print!("resp time {} ", pkt.time);
                        match event {
                            NodeEvent::Write(r) => {
                                let Some(ControllerEvent::Write(a, p, v)) = ctrl_event.take()
                                else {
                                    bail!("Expected write from controller")
                                };
                                println!("Write ok {v:?} to {p:?}@{a:?} => {r:?}");
                            }
                            NodeEvent::Read(Ok(val)) => {
                                let Some(ControllerEvent::Read(a, p)) = ctrl_event.take() else {
                                    bail!("Expected read from controller")
                                };
                                println!("Read {p:?}@{a:?} => {val:?}");
                            }
                            NodeEvent::UnexpectedTransmission => {
                                println!("Unexpected data on node tx channel {consumed:?}");
                                continue 'next_packet;
                            }
                            _ => {}
                        }
                    } else {
                        if data.check_trigger() {
                            println!("Trigger event");
                            continue;
                        }
                        println!("Not enough data in node ch packet.");
                        continue 'next_packet;
                    }
                }
            }
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
