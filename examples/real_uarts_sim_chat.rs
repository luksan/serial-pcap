use anyhow::{Context, Result};
use tokio::io::AsyncWriteExt;
use tokio_serial::SerialStream;
use x328_proto::{addr, node, param, value, Master, NodeState};

use serial_pcap::open_async_uart;

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

    pub async fn next(
        &mut self,
        master_tx: &mut SerialStream,
        client_tx: &mut SerialStream,
    ) -> Result<()> {
        if self.read {
            let send = self.master.read_parameter(addr(21), param(23));
            master_tx.write_all(send.get_data()).await?;
            for node in &mut self.nodes {
                node.next(send.get_data(), client_tx).await;
            }
        } else {
            let send = self
                .master
                .write_parameter(addr(31), param(223), value(442));
            master_tx.write_all(send.get_data()).await?;
            for node in &mut self.nodes {
                node.next(send.get_data(), client_tx).await;
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

    async fn next(&mut self, recv: &[u8], send: &mut SerialStream) {
        let mut state = self.0.take().unwrap().receive_data(recv);
        loop {
            state = match state {
                NodeState::ReceiveData(r) => {
                    self.0 = r.into();
                    return;
                }
                NodeState::SendData(mut s) => {
                    send.write_all(s.get_data()).await.expect("Write failed");
                    s.data_sent()
                }
                NodeState::ReadParameter(read) => read.send_reply_ok(value(33)),
                NodeState::WriteParameter(write) => write.write_ok(),
            };
        }
    }
}

async fn chat(mut ctrl: SerialStream, mut node: SerialStream) -> Result<()> {
    let mut chat = Chat::new();

    let mut cnt = 0;

    while chat.next(&mut ctrl, &mut node).await.is_ok() {
        cnt += 1;
        if cnt > 10 {
            break;
        }
    }
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let ctrl_uart = open_async_uart("COM12")?;
    let node_uart = open_async_uart("COM13")?;

    chat(ctrl_uart, node_uart).await?;

    Ok(())
}
