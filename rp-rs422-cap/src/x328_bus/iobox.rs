use crate::x328_bus::{NodeMirror, UpdateEvent};
use enumflags2::{bitflags, BitFlags};
use x328_proto::{addr, Address, Parameter, Value};

#[bitflags]
#[derive(Copy, Clone, Debug, PartialEq)]
#[repr(u16)]
pub enum CommandBit {
    RunDec = 1 << 15,
    InchUp = 1 << 14,
    InchDown = 1 << 13,
    Track = 1 << 12,
    RunPol = 1 << 11,
    InchWest = 1 << 10,
    InchEast = 1 << 9,

    SerialOK = 1 << 8,
    ComBit7 = 1 << 7,
    ComBit6 = 1 << 6,

    FlapsIn = 1 << 5,
    FlapsOut = 1 << 4,
    WestStowRelease = 1 << 3,
    EastStowRelease = 1 << 2,
    WestStowLock = 1 << 1,
    EastStowLock = 1 << 0,
}

#[bitflags]
#[derive(Copy, Clone, Debug, PartialEq)]
#[repr(u16)]
pub enum OutputBit {
    RunDec = 1 << 15,
    InchUp = 1 << 14,
    InchDown = 1 << 13,
    Track = 1 << 12,
    RunPol = 1 << 11,
    InchWest = 1 << 10,
    InchEast = 1 << 9,

    SerialOK = 1 << 8,
    ComBit7 = 1 << 7,
    ComBit6 = 1 << 6,

    FlapsIn = 1 << 5,
    FlapsOut = 1 << 4,
    WestStowRelease = 1 << 3,
    EastStowRelease = 1 << 2,
    WestStowLock = 1 << 1,
    EastStowLock = 1 << 0,
}

#[bitflags]
#[derive(Copy, Clone, Debug, PartialEq)]
#[repr(u16)]
pub enum InputBit {
    DecRun = 1 << 15,
    DecDriveOK = 1 << 14,
    UnderHorizon = 1 << 13,
    Track = 1 << 12,
    PolRun = 1 << 11,
    PolDriveOK = 1 << 10,
    WestOfSouth = 1 << 9,
    EastOfSouth = 1 << 8,
    NotStowpos = 1 << 7,
    Home = 1 << 6,
    FlapsIn = 1 << 5,
    FlapsOut = 1 << 4,
    WestStowReleased = 1 << 3,
    EastStowReleased = 1 << 2,
    WestStowLocked = 1 << 1,
    EastStowLocked = 1 << 0,
}

#[derive(Debug)]
pub struct IoBox {
    pub inputs: BitFlags<InputBit>,
    pub outputs: BitFlags<OutputBit>,
    pub cmd_reg: BitFlags<CommandBit>,
    pub stow_press_east: u16,
    pub stow_press_west: u16,
}

impl IoBox {
    pub const fn new() -> Self {
        Self {
            inputs: BitFlags::EMPTY,
            outputs: BitFlags::EMPTY,
            cmd_reg: BitFlags::EMPTY,
            stow_press_east: 0,
            stow_press_west: 0,
        }
    }
}

impl NodeMirror for IoBox {
    const ADDR: Address = addr(31);

    fn update_parameter(&mut self, p: Parameter, v: Value) -> Option<UpdateEvent> {
        match *p {
            101..=116 => {
                let bit = BitFlags::from_bits_truncate(1u16 << (*p - 101));
                self.cmd_reg.set(bit, *v != 0);
                UpdateEvent::IoboxCmd(self.cmd_reg)
            }
            117 => {
                self.cmd_reg = BitFlags::from_bits_truncate((*v & 0xffff) as u16);
                UpdateEvent::IoboxCmd(self.cmd_reg)
            }
            201..=216 => {
                let bit = BitFlags::from_bits_truncate(1u16 << (*p - 201));
                self.inputs.set(bit, *v != 0);
                UpdateEvent::IoboxInputs(self.inputs)
            }
            217 => {
                self.inputs = BitFlags::from_bits_truncate((*v & 0xffff) as u16);
                UpdateEvent::IoboxInputs(self.inputs)
            }
            301..=316 => {
                let bit = BitFlags::from_bits_truncate(1u16 << (*p - 301));
                self.outputs.set(bit, *v != 0);
                UpdateEvent::IoboxOutputs(self.outputs)
            }
            317 => {
                self.outputs = BitFlags::from_bits_truncate((*v & 0xffff) as u16);
                UpdateEvent::IoboxOutputs(self.outputs)
            }
            401 => {
                self.stow_press_east = *v as u16;
                UpdateEvent::StowPress(self.stow_press_east, self.stow_press_west)
            }
            402 => {
                self.stow_press_west = *v as u16;
                UpdateEvent::StowPress(self.stow_press_east, self.stow_press_west)
            }
            _ => return None,
        }
        .into()
    }
}
