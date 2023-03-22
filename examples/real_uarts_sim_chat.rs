use std::time::Duration;

use anyhow::{Context, Result};
use bytes::BytesMut;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::time::timeout;
use tokio_serial::SerialStream;
use x328_proto::master::{ReceiveDataProgress, Receiver};
use x328_proto::{addr, master, node, param, value, Master, NodeState, Value};

use serial_pcap::open_async_uart;

pub struct BusController {
    master: Master,
}

#[derive(Copy, Clone, Debug)]
pub enum Cmd {
    R(u8, i16),
    W(u8, i16, i32),
}

impl BusController {
    pub fn new() -> Self {
        BusController {
            master: Master::new(),
        }
    }

    pub async fn next(&mut self, cmd: Cmd, uart: &mut SerialStream) -> Result<Value> {
        match cmd {
            Cmd::R(a, p) => {
                let read = self.master.read_parameter(addr(a), param(p));
                match Self::master_trx(read, uart).await? {
                    Ok(r) => return Ok(r),
                    Err(e) => println!("Error in response: {e:?}"),
                }
            }
            Cmd::W(a, p, v) => {
                let write = self.master.write_parameter(addr(a), param(p), value(v));
                match Self::master_trx(write, uart).await? {
                    Ok(_) => return Ok(value(1)),
                    Err(e) => println!("Error in response: {e:?}"),
                }
            }
        }
        Ok(value(0))
    }

    // this doesn't take `self` since send is borrowed from self.master
    async fn master_trx<Rec: Receiver<R>, R>(
        send: master::SendData<'_, Rec, R>,
        uart: &mut SerialStream,
    ) -> Result<R> {
        uart.write_all(send.get_data())
            .await
            .context("Ctrl UART write failed")?;

        let mut sent = send.data_sent();
        let mut buf = BytesMut::with_capacity(40);
        loop {
            buf.clear();
            timeout(Duration::from_millis(500), uart.read_buf(&mut buf))
                .await
                .context("Ctrl UART read timeout")?
                .context("Ctrl UART read error")?;
            match sent.receive_data(buf.as_ref()) {
                ReceiveDataProgress::Done(r) => return Ok(r),
                ReceiveDataProgress::NeedData(s) => {
                    sent = s;
                }
            }
        }
    }
}

struct Node(Option<node::ReceiveData>);

impl Node {
    fn new(address: usize) -> Self {
        Self(node::ReceiveData::new(address).ok())
    }

    async fn next(&mut self, recv: &[u8], send: &mut SerialStream) -> Result<()> {
        let mut state = self.0.take().unwrap().receive_data(recv);
        loop {
            state = match state {
                NodeState::ReceiveData(r) => {
                    self.0 = r.into();
                    return Ok(());
                }
                NodeState::SendData(mut s) => {
                    send.write_all(s.get_data())
                        .await
                        .context("Node UART write failed")?;
                    s.data_sent()
                }
                NodeState::ReadParameter(read) => read.send_reply_ok(value(33)),
                NodeState::WriteParameter(write) => write.write_ok(),
            };
        }
    }
}

async fn nodes_chat(mut uart: SerialStream, mut nodes: Vec<Node>) -> Result<()> {
    let mut buf = BytesMut::with_capacity(40);
    loop {
        buf.clear();
        uart.read_buf(&mut buf)
            .await
            .context("Node UART read failed")?;

        for node in nodes.iter_mut() {
            node.next(buf.as_ref(), &mut uart).await?;
        }
    }
}

async fn chat(mut ctrl: SerialStream, node: SerialStream) -> Result<()> {
    let scenario = vec![Cmd::R(21, 23), Cmd::W(31, 223, 442)];
    let scenario = scenario.iter().cycle().take(10).copied();

    let mut chat = BusController::new();

    let nodes = vec![Node::new(21), Node::new(31)];
    let node_handle = tokio::spawn(nodes_chat(node, nodes));

    for cmd in scenario {
        let _value = chat.next(cmd, &mut ctrl).await?;
        tokio::time::sleep(Duration::from_millis(10)).await;
        if node_handle.is_finished() {
            return node_handle
                .await
                .context("Error in node task join handle.")?
                .context("Node task terminated unexpectedly");
        }
    }
    // Stop the node UART reader
    node_handle.abort();
    let _ = node_handle.await;
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let ctrl_uart = open_async_uart("COM12")?;
    let node_uart = open_async_uart("COM13")?;

    chat(ctrl_uart, node_uart).await?;

    Ok(())
}
