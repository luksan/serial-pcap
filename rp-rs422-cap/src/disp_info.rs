use core::fmt::Write;

use arrayvec::ArrayString;
use embedded_graphics::mono_font;
use embedded_graphics::mono_font::{MonoFont, MonoTextStyle, MonoTextStyleBuilder};
use embedded_graphics::pixelcolor::Rgb565;
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::{PrimitiveStyle, Rectangle, StyledDrawable};
use embedded_graphics::text::{Alignment, Text};
use enumflags2::BitFlags;

use rp_rs422_cap::picodisplay;
use rp_rs422_cap::x328_bus::iobox::{CommandBit, InputBit, OutputBit};

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
    on_screen: [ScreenItem; INFO_CNT],
}

pub type Age = i32;

#[derive(Default)]
struct ScreenItem {
    info: Info,
    area: Rectangle,
    info_age: Age,
    style: ItemStyle,
}

#[derive(Default, Copy, Clone, Eq, PartialEq)]
enum ItemStyle {
    Current,
    Aging,
    #[default]
    Old,
}
type TextStyle = MonoTextStyle<'static, Rgb565>;

impl ItemStyle {
    const CURR_STYLE: TextStyle = MonoTextStyleBuilder::new()
        .font(BusDisplay::FONT)
        .text_color(Rgb565::GREEN)
        .background_color(Rgb565::BLACK)
        .build();
    const AGING_STYLE: TextStyle = MonoTextStyleBuilder::new()
        .font(BusDisplay::FONT)
        .text_color(Rgb565::GREEN)
        .background_color(Rgb565::YELLOW)
        .build();
    const OLD_STYLE: TextStyle = MonoTextStyleBuilder::new()
        .font(BusDisplay::FONT)
        .text_color(Rgb565::GREEN)
        .background_color(Rgb565::RED)
        .build();
    fn get_text_style(self) -> TextStyle {
        match self {
            ItemStyle::Current => Self::CURR_STYLE,
            ItemStyle::Aging => Self::AGING_STYLE,
            ItemStyle::Old => Self::OLD_STYLE,
        }
    }
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
            self.draw_info(i)
        }
    }

    pub fn check_age(&mut self, current_age: i32) {
        for idx in 0..self.on_screen.len() {
            let i = &mut self.on_screen[idx];
            match (current_age - i.info_age, i.style) {
                (-1, _) => continue,
                (0, ItemStyle::Current) | (1, ItemStyle::Aging) | (_, ItemStyle::Old) => continue,
                (0, _) => i.style = ItemStyle::Current,
                (1, _) => i.style = ItemStyle::Aging,
                (_, _) => i.style = ItemStyle::Old,
            }
            self.draw_info(idx);
        }
    }

    pub fn update_info(&mut self, info: Info, age: Age) {
        let info_idx = info.discriminant();
        self.on_screen[info_idx].info = info;
        self.on_screen[info_idx].style = ItemStyle::Current;
        self.on_screen[info_idx].info_age = age;
        self.draw_info(info_idx)
    }

    fn draw_info(&mut self, info_idx: usize) {
        let mut buf = ArrayString::<100>::new();
        let mut row;
        let info = &self.on_screen[info_idx].info;

        let _write_res = match info {
            Info::StowPressEast(p) => {
                row = 0;
                write!(&mut buf, "Stow east {p}")
            }
            Info::StowPressWest(p) => {
                row = 1;
                write!(&mut buf, "Stow west {p}")
            }
            Info::PolarSpeedCmd(s) => {
                row = 2;
                write!(&mut buf, "Pol speed cmd {s}")
            }
            Info::IoboxCmd(c) => {
                row = 4;
                c.iter().try_for_each(|b| writeln!(buf, "c {b:?}"))
            }
            Info::IoboxInputs(i) => {
                row = 9;
                i.iter().try_for_each(|b| writeln!(buf, "i {b:?}"))
            }
            Info::IoboxOutputs(o) => {
                row = 15;
                o.iter().try_for_each(|b| writeln!(buf, "o {b:?}"))
            }
            Info::END => return,
        };

        let top_left = Row(row).top_left(0);

        for line in buf.lines().filter(|l| !l.is_empty()) {
            self.write_row(
                Row(row),
                line,
                self.on_screen[info_idx].style.get_text_style(),
            );
            row += 1;
        }
        row -= 1;
        let bottom_right = Row(row).bottom_right();

        let prev_rect = core::mem::replace(
            &mut self.on_screen[info_idx].area,
            Rectangle::with_corners(top_left, bottom_right),
        );

        // Clear any old lines below, in case of multi-line info text
        let top_below = Row(row + 1).top_left(0);
        if let Some(bottom_right) = prev_rect.bottom_right() {
            if top_below.y < bottom_right.y {
                let to_clear = Rectangle::with_corners(top_below, bottom_right);
                self.clear_area(to_clear);
            }
        }
    }

    fn clear_area(&mut self, rect: Rectangle) {
        if !rect.is_zero_sized() {
            let _ = rect.draw_styled(&PrimitiveStyle::with_fill(Rgb565::BLUE), &mut self.screen);
        }
    }

    fn write_row(&mut self, row: Row, text: &str, style: TextStyle) {
        let Ok(row_end) = Text::with_alignment(text, row.baseline(), style, Alignment::Left)
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
