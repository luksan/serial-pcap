use core::ops::Deref;

use iobox::IoBox;
use x328_proto::{Address, Parameter, Value};

mod iobox;

#[derive(Default)]
pub struct UartBuf {
    len: usize,
    read_pos: usize,
    data: [u8; 20],
}

impl Deref for UartBuf {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        &self.data[self.read_pos..self.read_pos + self.len]
    }
}

impl UartBuf {
    pub const fn new() -> Self {
        Self {
            len: 0,
            read_pos: 0,
            data: [0; 20],
        }
    }

    pub fn tail_slice(&mut self, min_cap: usize) -> &mut [u8] {
        if self.is_empty() {
            self.read_pos = 0;
        }
        let tail_cap = self.data.len() - (self.read_pos + self.len);
        let tot_spare_cap = tail_cap + self.read_pos;
        if tot_spare_cap < min_cap {
            self.consume(min_cap - tot_spare_cap);
        }
        if tail_cap < min_cap {
            self.data.copy_within(self.read_pos..self.len, 0);
        }

        let wr_pos = self.read_pos + self.len;
        &mut self.data[wr_pos..]
    }

    pub fn incr_len(&mut self, new: usize) {
        self.len += new;
    }

    pub fn write(&mut self, data: &[u8]) {
        let data = if data.len() > self.data.len() {
            let excess = data.len() - self.data.len();
            &data[excess..]
        } else {
            data
        };
        let x = &mut self.tail_slice(data.len())[0..data.len()];
        x.copy_from_slice(data);
        self.incr_len(data.len());
    }

    pub fn consume(&mut self, len: usize) {
        let len = len.min(self.len);
        self.read_pos += len;
        self.len -= len;
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn len(&self) -> usize {
        self.len
    }
}

// Tracks all the nodes on the bus in the 25m
pub struct FieldBus {
    pub iobox: IoBox,
}

impl FieldBus {
    pub const fn new() -> Self {
        Self {
            iobox: IoBox::new(),
        }
    }
    pub fn update_parameter(&mut self, a: Address, p: Parameter, v: Value) {
        match a {
            IoBox::ADDR => self.iobox.update_parameter(p, v),
            _ => {}
        }
    }
}

pub trait NodeMirror {
    const ADDR: Address;
    fn update_parameter(&mut self, p: Parameter, v: Value);
}
