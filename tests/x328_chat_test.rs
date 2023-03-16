use std::io::Write;

use anyhow::Result;
use x328_proto::{addr, node, param, value, Master, NodeState};

use serial_pcap::{SerialPacketWriter, UartTxChannel};

pub struct Chat {
    master: Master,
    nodes: Vec<Node>,
    read: bool,
}

fn new_node(address: usize) -> Node {
    Node::new(address)
}

impl Chat {
    pub fn new() -> Self {
        Chat {
            master: Master::new(),
            nodes: vec![new_node(21), new_node(31)],
            read: true,
        }
    }

    pub fn next<T: Write>(&mut self, mut master_tx: T, mut client_tx: T) -> Result<()> {
        if self.read {
            let send = self.master.read_parameter(addr(21), param(23));
            master_tx.write_all(send.get_data())?;
            for node in &mut self.nodes {
                node.next(send.get_data(), &mut client_tx)
            }
        } else {
            let send = self
                .master
                .write_parameter(addr(31), param(223), value(442));
            master_tx.write_all(send.get_data())?;
            for node in &mut self.nodes {
                node.next(send.get_data(), &mut client_tx)
            }
        }
        self.read = !self.read;
        Ok(())
    }
}

struct Node(Option<node::ReceiveData>);

impl Node {
    fn new(address: usize) -> Self {
        Self(node::ReceiveData::new(address).ok())
    }

    fn next(&mut self, recv: &[u8], mut send: impl Write) {
        let mut state = self.0.take().unwrap().receive_data(recv);
        loop {
            state = match state {
                NodeState::ReceiveData(r) => {
                    self.0 = r.into();
                    return;
                }
                NodeState::SendData(mut s) => {
                    send.write_all(s.get_data()).expect("Write failed");
                    s.data_sent()
                }
                NodeState::ReadParameter(read) => read.send_reply_ok(value(33)),
                NodeState::WriteParameter(write) => write.write_ok(),
            };
        }
    }
}

#[test]
fn test_chatter() -> Result<()> {
    let mut pcap = SerialPacketWriter::new("test.pcap")?;
    let mut chat = Chat::new();

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
