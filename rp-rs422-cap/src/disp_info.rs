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
    on_screen: [(Info, Rectangle); INFO_CNT],
}
#[derive(Copy, Clone)]
struct Row(i32);

const DISP_WIDTH: i32 = 135;

impl Row {
    fn top_left(self, x: i32) -> Point {
        Point::new(x, self.0 * BusDisplay::ROW_HEIGHT)
    }
    fn bottom_right(self) -> Point {
        // y -1 since the point is inside the bounding box
        Point::new(DISP_WIDTH - 1, (self.0 + 1) * BusDisplay::ROW_HEIGHT - 1)
    }
    fn baseline(self) -> Point {
        let y = BusDisplay::FONT.baseline as i32 + self.0 * BusDisplay::ROW_HEIGHT;
        Point::new(0, y)
    }
}

impl BusDisplay {
    const FONT: &'static MonoFont<'static> = &mono_font::ascii::FONT_7X14;
    const ROW_HEIGHT: i32 = Self::FONT.character_size.height as i32;

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
            self.draw_info(self.on_screen[i].0)
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

        let top_left = Row(row).top_left(0);

        for line in buf.lines().filter(|l| !l.is_empty()) {
            self.write_row(Row(row), line);
            row += 1;
        }
        row -= 1;
        let bottom_right = Row(row).bottom_right();

        let info_idx = info.discriminant();
        self.on_screen[info_idx].0 = info;
        let prev_rect = core::mem::replace(
            &mut self.on_screen[info_idx].1,
            Rectangle::with_corners(top_left, bottom_right),
        );

        // Clear any old lines below, in case of multi-line info text
        let top_below = Row(row + 1).top_left(0);
        let bottom_right = prev_rect.bottom_right().unwrap();
        if top_below.y < bottom_right.y {
            let to_clear = Rectangle::with_corners(top_below, bottom_right);
            self.clear_area(to_clear);
        }
    }

    fn clear_area(&mut self, rect: Rectangle) {
        if !rect.is_zero_sized() {
            rect.draw_styled(&PrimitiveStyle::with_fill(Rgb565::BLUE), &mut self.screen);
        }
    }

    fn write_row(&mut self, row: Row, text: &str) {
        let Ok(row_end) =
            Text::with_alignment(text, row.baseline(), Self::TEXT_STYLE, Alignment::Left)
                .draw(&mut self.screen)
        else {
            return;
        };
        // clear the row end
        self.clear_area(Rectangle::with_corners(
            row.top_left(row_end.x),
            row.bottom_right(),
        ));
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
