use crate::x328_bus::{NodeMirror, UpdateEvent};
use core::marker::PhantomData;
use x328_proto::{addr, Address, Parameter, Value};

pub struct Polar;
pub struct Declination;

pub struct Encoder<Pos> {
    value: i32, // 100-dels grader
    _pos: PhantomData<Pos>,
}

impl<Pos> Encoder<Pos> {
    pub const fn new() -> Self {
        Self {
            value: 0,
            _pos: PhantomData,
        }
    }
}
impl<Pos> Default for Encoder<Pos> {
    fn default() -> Self {
        Self::new()
    }
}

impl NodeMirror for Encoder<Polar> {
    const ADDR: Address = addr(12);

    fn update_parameter(&mut self, p: Parameter, v: Value) -> Option<UpdateEvent> {
        match *p {
            101 => {
                self.value = *v;
                UpdateEvent::PolarEncoder(self.value)
            }
            _ => return None,
        }
        .into()
    }
}

impl NodeMirror for Encoder<Declination> {
    const ADDR: Address = addr(22);

    fn update_parameter(&mut self, p: Parameter, v: Value) -> Option<UpdateEvent> {
        match *p {
            101 => {
                self.value = *v;
                UpdateEvent::DeclinationEncoder(self.value)
            }
            _ => return None,
        }
        .into()
    }
}
