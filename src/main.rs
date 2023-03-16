#![allow(dead_code)]

#[allow(unused_imports)]
use anyhow::{Context, Result};
use clap::Parser;

use serial_pcap::SerialPacketWriter;

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

fn main() -> Result<()> {
    let args = CmdlineOpts::parse();
    let _pcap_writer = SerialPacketWriter::new(args.pcap_file)?;
    Ok(())
}
