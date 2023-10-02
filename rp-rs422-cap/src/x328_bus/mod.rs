use core::ops::Deref;
use enumflags2::BitFlags;

use crate::x328_bus::iobox::{CommandBit, InputBit, OutputBit};
use iobox::IoBox;
use x328_proto::{addr, Address, Parameter, Value};

pub mod iobox;

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
        let tail_cap = self.tail_capacity();
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

    fn tail_capacity(&self) -> usize {
        self.data.len() - (self.read_pos + self.len)
    }

    pub fn incr_len(&mut self, new: usize) {
        let new = self.tail_capacity().min(new);
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

pub enum UpdateEvent {
    StowPress(u16, u16),
    IoboxInputs(BitFlags<InputBit>),
    IoboxCmd(BitFlags<CommandBit>),
    IoboxOutputs(BitFlags<OutputBit>),
    PolarSpeedCmd(u16),
}

impl FieldBus {
    pub const fn new() -> Self {
        Self {
            iobox: IoBox::new(),
        }
    }
    pub fn update_parameter(&mut self, a: Address, p: Parameter, v: Value) -> Option<UpdateEvent> {
        const POL_DRV: Address = addr(11);
        match a {
            IoBox::ADDR => self.iobox.update_parameter(p, v),
            POL_DRV => match *p {
                118 => Some(UpdateEvent::PolarSpeedCmd(*v as u16)),
                _ => None,
            },
            _ => None,
        }
    }
}

pub trait NodeMirror {
    const ADDR: Address;
    fn update_parameter(&mut self, p: Parameter, v: Value) -> Option<UpdateEvent>;
}
