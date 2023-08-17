use core::convert::Infallible;

use display_interface_spi::SPIInterface;
use embedded_graphics::pixelcolor::{Rgb888, RgbColor};
use embedded_hal::blocking::delay::DelayUs;
use embedded_hal::digital::v2::OutputPin;
use embedded_hal::spi::MODE_0;
use embedded_hal::PwmPin;
use fugit::RateExtU32;
use mipidsi::{models, Builder, Display};
use rp_pico::hal::gpio::bank0::{
    Gpio12, Gpio13, Gpio14, Gpio15, Gpio16, Gpio17, Gpio18, Gpio19, Gpio20, Gpio6, Gpio7, Gpio8,
};
use rp_pico::hal::gpio::{FunctionSpi, Pin, PullDownDisabled, PullUpInput, PushPullOutput};
use rp_pico::hal::{pwm, spi};
use rp_pico::pac;

pub struct DummyPin;

impl OutputPin for DummyPin {
    type Error = Infallible;
    fn set_low(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }
    fn set_high(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }
}

pub type Screen = Display<
    SPIInterface<spi::Spi<spi::Enabled, pac::SPI0, 8>, DC, CS>,
    models::ST7789,
    DummyPin, // Reset is connected to RUN on the Pi Pico
>;

pub type CS = Pin<Gpio17, PushPullOutput>;
pub type DC = Pin<Gpio16, PushPullOutput>;

pub type GpioPin<T> = Pin<T, PullDownDisabled>;

pub struct Buttons {
    pub a: Pin<Gpio12, PullUpInput>,
    pub b: Pin<Gpio13, PullUpInput>,
    pub x: Pin<Gpio14, PullUpInput>,
    pub y: Pin<Gpio15, PullUpInput>,
}

#[macro_export]
macro_rules! make_buttons {
    ($pins:expr) => {
        Buttons::new($pins.gpio12, $pins.gpio13, $pins.gpio14, $pins.gpio15)
    };
}

impl Buttons {
    pub fn new(
        a: GpioPin<Gpio12>,
        b: GpioPin<Gpio13>,
        x: GpioPin<Gpio14>,
        y: GpioPin<Gpio15>,
    ) -> Self {
        Self {
            a: a.into_mode(),
            b: b.into_mode(),
            x: x.into_mode(),
            y: y.into_mode(),
        }
    }
}

pub struct RGB {
    pwm_rg: pwm::Slice<pwm::Pwm3, pwm::FreeRunning>,
    pwm_b: pwm::Slice<pwm::Pwm4, pwm::FreeRunning>,
    brightness: u8,
    color: Rgb888,
}

const GAMMA_8BIT: [u8; 256] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 1, 1, 1, 2, 2, 2,
    2, 2, 2, 3, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 6, 6, 6, 7, 7, 7, 8, 8, 8, 9, 9, 9, 10, 10, 11,
    11, 11, 12, 12, 13, 13, 13, 14, 14, 15, 15, 16, 16, 17, 17, 18, 18, 19, 19, 20, 21, 21, 22, 22,
    23, 23, 24, 25, 25, 26, 27, 27, 28, 29, 29, 30, 31, 31, 32, 33, 34, 34, 35, 36, 37, 37, 38, 39,
    40, 40, 41, 42, 43, 44, 45, 46, 46, 47, 48, 49, 50, 51, 52, 53, 54, 55, 56, 57, 58, 59, 60, 61,
    62, 63, 64, 65, 66, 67, 68, 69, 70, 71, 72, 73, 74, 76, 77, 78, 79, 80, 81, 83, 84, 85, 86, 88,
    89, 90, 91, 93, 94, 95, 96, 98, 99, 100, 102, 103, 104, 106, 107, 109, 110, 111, 113, 114, 116,
    117, 119, 120, 121, 123, 124, 126, 128, 129, 131, 132, 134, 135, 137, 138, 140, 142, 143, 145,
    146, 148, 150, 151, 153, 155, 157, 158, 160, 162, 163, 165, 167, 169, 170, 172, 174, 176, 178,
    179, 181, 183, 185, 187, 189, 191, 193, 194, 196, 198, 200, 202, 204, 206, 208, 210, 212, 214,
    216, 218, 220, 222, 224, 227, 229, 231, 233, 235, 237, 239, 241, 244, 246, 248, 250, 252, 255,
];

impl RGB {
    pub fn new(
        pin_r: GpioPin<Gpio6>,
        pin_g: GpioPin<Gpio7>,
        pin_b: GpioPin<Gpio8>,
        mut pwm_rg: pwm::Slice<pwm::Pwm3, pwm::FreeRunning>,
        mut pwm_b: pwm::Slice<pwm::Pwm4, pwm::FreeRunning>,
    ) -> Self {
        pwm_rg.default_config();
        pwm_b.default_config();

        pwm_rg.channel_a.set_inverted();
        pwm_rg.channel_b.set_inverted();
        pwm_b.channel_a.set_inverted();

        pwm_rg.channel_a.output_to(pin_r);
        pwm_rg.channel_b.output_to(pin_g);
        pwm_b.channel_a.output_to(pin_b);

        pwm_rg.enable();
        pwm_b.enable();

        Self {
            pwm_rg,
            pwm_b,
            brightness: 0,
            color: Rgb888::WHITE,
        }
    }
    pub fn set_color(&mut self, color: impl Into<Rgb888>) {
        self.color = color.into();
        self.update_pwm();
    }

    pub fn set_brightness(&mut self, brightness: u8) {
        self.brightness = brightness;
        self.update_pwm();
    }

    fn update_pwm(&mut self) {
        let r: u16 = GAMMA_8BIT[self.color.r() as usize].into();
        let g: u16 = GAMMA_8BIT[self.color.g() as usize].into();
        let b: u16 = GAMMA_8BIT[self.color.b() as usize].into();

        let r = r.saturating_mul(self.brightness as _);
        let g = g.saturating_mul(self.brightness as _);
        let b = b.saturating_mul(self.brightness as _);

        self.pwm_rg.channel_a.set_duty(r);
        self.pwm_rg.channel_b.set_duty(g);
        self.pwm_b.channel_a.set_duty(b);
    }
}

#[macro_export]
macro_rules! create_picodisplay {
    ($pins:expr, $pac:expr, $delay:expr) => {
        PicoDisplay::new(
            $pins.gpio16,
            $pins.gpio17,
            $pins.gpio18,
            $pins.gpio19,
            $pins.gpio20,
            $pac.SPI0,
            &mut $pac.RESETS,
            $delay,
        )
    };
}

pub struct PicoDisplay {
    pub screen: Screen,
}
impl PicoDisplay {
    pub fn new(
        gpio16: GpioPin<Gpio16>, // Data / Control (MISO unused)
        gpio17: GpioPin<Gpio17>, // Chip Select
        gpio18: GpioPin<Gpio18>, // SPI0 clock
        gpio19: GpioPin<Gpio19>, // SPI0 MOSI
        gpio20: GpioPin<Gpio20>, // Backlight
        spi0: pac::SPI0,
        resets: &mut pac::RESETS,
        delay: &mut impl DelayUs<u32>,
    ) -> Self {
        let dc = gpio16.into_push_pull_output();
        let cs = gpio17.into_push_pull_output();
        let _spi_sclk = gpio18.into_mode::<FunctionSpi>();
        let _spi_mosi = gpio19.into_mode::<FunctionSpi>();
        let mut backlight = gpio20.into_mode::<PushPullOutput>();

        let spi_screen = spi::Spi::new(spi0).init(resets, 125u32.MHz(), 16u32.MHz(), &MODE_0);
        let spi_if = SPIInterface::new(spi_screen, dc, cs);

        let screen = Builder::st7789_pico1(spi_if).init(delay, None).unwrap();
        backlight.set_high().unwrap();

        Self { screen }
    }
}
