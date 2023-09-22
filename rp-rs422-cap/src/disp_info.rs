use core::fmt::Write;
use core::sync::atomic::Ordering;

use arrayvec::ArrayString;
use embedded_graphics::mono_font;
use embedded_graphics::mono_font::ascii::FONT_10X20;
use embedded_graphics::mono_font::{MonoFont, MonoTextStyle, MonoTextStyleBuilder};
use embedded_graphics::pixelcolor::Rgb565;
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::{PrimitiveStyle, Rectangle, StyledDrawable, Triangle};
use embedded_graphics::text::{Alignment, Text};
use enumflags2::BitFlags;

use rp_rs422_cap::picodisplay;
use rp_rs422_cap::picodisplay::PicoDisplay;
use rp_rs422_cap::x328_bus::iobox::{CommandBit, InputBit, OutputBit};

use crate::BTN_CTR;

#[repr(u8)]
#[derive(Copy, Clone, Default, PartialEq, Eq)]
pub enum Info {
    StowPressEast(u16) = 0,
    StowPressWest(u16),
    PolarSpeedCmd(u16),
    IoboxCmd(BitFlags<CommandBit>),
    IoboxInputs(BitFlags<InputBit>),
    IoboxOutputs(BitFlags<OutputBit>),
    #[default]
    END,
}

const INFO_CNT: usize = Info::END.discriminant();

impl Info {
    const fn discriminant(&self) -> usize {
        // https://doc.rust-lang.org/reference/items/enumerations.html#pointer-casting
        (unsafe { *(self as *const Self as *const u8) }) as usize
    }
}

pub struct DisplayUpdates {
    new: [Info; INFO_CNT],
    idx: usize,
}

impl DisplayUpdates {
    pub const fn new() -> Self {
        Self {
            new: [Info::END; INFO_CNT],
            idx: 0,
        }
    }
    pub fn set_info(&mut self, info: Info) {
        if matches!(info, Info::END) {
            return;
        }
        self.new[info.discriminant()] = info;
    }

    pub fn next_change(&mut self) -> Option<Info> {
        let mut idx = self.idx;
        for _ in 0..self.new.len() {
            idx += 1;
            if idx >= self.new.len() {
                idx = 0;
            }
            if self.new[idx] != Info::END {
                let r = self.new[idx];
                self.new[idx] = Info::END;
                self.idx = idx;
                return Some(r);
            }
        }
        None
    }
}

pub struct BusDisplay {
    screen: picodisplay::Screen,
    on_screen: [Info; INFO_CNT],
}

impl BusDisplay {
    const FONT: &'static MonoFont<'static> = &mono_font::ascii::FONT_7X14;

    const TEXT_STYLE: MonoTextStyle<'static, Rgb565> = MonoTextStyleBuilder::new()
        .font(Self::FONT)
        .text_color(Rgb565::GREEN)
        .background_color(Rgb565::BLACK)
        .build();

    pub fn new(screen: picodisplay::Screen) -> Self {
        Self {
            screen,
            on_screen: Default::default(),
        }
    }

    /// Redraw the entire screen
    pub fn redraw(&mut self) {
        self.screen.clear(RgbColor::BLUE).unwrap();
        for i in 0..self.on_screen.len() {
            self.draw_info(self.on_screen[i])
        }
    }

    pub fn draw_info(&mut self, info: Info) {
        let mut buf = ArrayString::<100>::new();
        let mut row;
        match info {
            Info::StowPressEast(p) => {
                row = 0;
                write!(&mut buf, "Stow east {p}");
            }
            Info::StowPressWest(p) => {
                row = 1;
                write!(&mut buf, "Stow west {p}");
            }
            Info::PolarSpeedCmd(s) => {
                row = 2;
                write!(&mut buf, "Pol speed cmd {s}");
            }
            Info::IoboxCmd(c) => {
                row = 4;
                for b in c.iter() {
                    writeln!(buf, "c {b:?}");
                }
            }
            Info::IoboxInputs(i) => {
                row = 9;
                for b in i.iter() {
                    writeln!(buf, "i {b:?}");
                }
            }
            Info::IoboxOutputs(o) => {
                row = 15;
                for b in o.iter() {
                    writeln!(buf, "o {b:?}");
                }
            }
            Info::END => return,
        }

        for line in buf.lines().filter(|l| !l.is_empty()) {
            self.write_row(row, line);
            row += 1;
        }

        self.on_screen[info.discriminant()] = info;
    }

    fn write_row(&mut self, row: i32, text: &str) {
        let font_height = Self::FONT.character_size.height as i32;
        let y = Self::FONT.baseline as i32 + row * font_height;

        let Ok(row_end) =
            Text::with_alignment(text, Point::new(0, y), Self::TEXT_STYLE, Alignment::Left)
                .draw(&mut self.screen)
        else {
            return;
        };
        // clear the row end
        let disp_width = self.screen.bounding_box().size.width;
        Rectangle::new(
            Point::new(row_end.x, row_end.y - (Self::FONT.baseline as i32)),
            Size::new(disp_width, font_height as u32),
        )
        .draw_styled(&PrimitiveStyle::with_fill(Rgb565::BLUE), &mut self.screen);
    }
}

pub fn r(disp: &mut PicoDisplay) {
    let screen = &mut disp.screen;
    // screen.clear(RgbColor::BLUE).unwrap();

    let style = MonoTextStyleBuilder::new()
        .font(&FONT_10X20)
        .text_color(Rgb565::GREEN)
        .background_color(Rgb565::BLACK)
        .build();
    let thin_stroke = PrimitiveStyle::with_stroke(Rgb565::GREEN, 1);
    screen.bounding_box().into_styled(thin_stroke).draw(screen);
    let x_off = 16;
    let y_off = 20;
    Triangle::new(
        Point::new(x_off + 8, y_off - 16),
        Point::new(x_off - 8, y_off - 16),
        Point::new(x_off, y_off),
    )
    .into_styled(thin_stroke)
    .draw(screen);

    let mut strbuf = ArrayString::<100>::new();
    let presses = BTN_CTR.load(Ordering::Relaxed);
    write!(&mut strbuf, "{}", presses);
    Text::with_alignment(
        strbuf.as_str(), // asd
        Point::new(15, 35),
        style,
        Alignment::Left,
    )
    .draw(screen);
}
