use anyhow::Result;
use std::convert::TryInto;
use std::io::Write;
use x328_proto::{node, Master, NodeState};

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
            let send = self.master.read_parameter(21.try_into()?, 23.try_into()?);
            master_tx.write_all(send.get_data())?;
            for node in &mut self.nodes {
                node.next(send.get_data(), &mut client_tx)
            }
        } else {
            let send = self
                .master
                .write_parameter(31.try_into()?, 223.try_into()?, 442u16.into());
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
                NodeState::ReadParameter(read) => read.send_reply_ok(33u16.into()),
                NodeState::WriteParameter(write) => write.write_ok(),
            };
        }
    }
}
