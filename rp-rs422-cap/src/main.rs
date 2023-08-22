#![no_std]
#![no_main]
#![allow(unused)]

use core::fmt::Write;
use core::sync::atomic::{AtomicU32, Ordering};

use arrayvec::ArrayString;
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::PrimitiveStyle;
use rp_pico::hal::{self, gpio, uart};
use rp_pico::pac;
// USB Device support
use usb_device::{class_prelude::*, prelude::*};
// USB Communications Class Device support
use usbd_serial::SerialPort;

use rp_rs422_cap::picodisplay::{self, Buttons, PicoDisplay};

type UartRxPin<P> = gpio::Pin<P, gpio::FunctionUart>;
type UartDev<D, P> = uart::UartPeripheral<uart::Enabled, D, uart::Pins<(), UartRxPin<P>, (), ()>>;

type Uart0 = UartDev<pac::UART0, gpio::bank0::Gpio1>;
type Uart1 = UartDev<pac::UART1, gpio::bank0::Gpio5>;

#[rtic::app(
    device = rp_pico::hal::pac,
    dispatchers = [TIMER_IRQ_1]
)]
mod app {
    use core::mem::MaybeUninit;

    use embedded_graphics::pixelcolor::Rgb888;
    use embedded_hal::digital::v2::{OutputPin, ToggleableOutputPin};
    use embedded_hal::PwmPin;
    use gpio::FunctionSpi;
    use hal::clocks::ClockSource;
    use panic_probe as _;
    use rp2040_monotonic::{
        fugit::Duration,
        fugit::RateExtU32, // For .kHz() conversion funcs
        Rp2040Monotonic,
    };
    use rp_pico::hal::{
        gpio::pin::bank0::{Gpio2, Gpio25, Gpio3},
        gpio::pin::PushPullOutput,
        pac, pwm,
        sio::Sio,
        Clock, I2C,
    };
    use rp_pico::XOSC_CRYSTAL_FREQ;

    use rp_rs422_cap::{create_picodisplay, make_buttons};

    use super::*;

    const MONO_NUM: u32 = 1;
    const MONO_DENOM: u32 = 1000000;
    const ONE_SEC_TICKS: u64 = 1000000;

    #[monotonic(binds = TIMER_IRQ_0, default = true)]
    type Rp2040Mono = Rp2040Monotonic;

    #[shared]
    struct Shared {
        usb_serial: SerialPort<'static, hal::usb::UsbBus>,
        usb_serial2: SerialPort<'static, hal::usb::UsbBus>,
    }

    #[local]
    struct Local {
        buttons: Buttons,
        led: gpio::Pin<Gpio25, PushPullOutput>,
        picodisplay: PicoDisplay,
        usb_device: UsbDevice<'static, hal::usb::UsbBus>,
        uart0: Uart0,
        uart1: Uart1,
    }

    #[init(local=[
        usb_bus_uninit: MaybeUninit<usb_device::bus::UsbBusAllocator<hal::usb::UsbBus>> = MaybeUninit::uninit(),
    ])]
    fn init(mut ctx: init::Context) -> (Shared, Local, init::Monotonics) {
        let mut pac = ctx.device;

        // Configure the clocks, watchdog - The default is to generate a 125 MHz system clock
        let mut watchdog = hal::watchdog::Watchdog::new(pac.WATCHDOG);

        let clocks: hal::clocks::ClocksManager = hal::clocks::init_clocks_and_plls(
            XOSC_CRYSTAL_FREQ,
            pac.XOSC,
            pac.CLOCKS,
            pac.PLL_SYS,
            pac.PLL_USB,
            &mut pac.RESETS,
            &mut watchdog,
        )
        .ok()
        .unwrap();

        let delay =
            &mut cortex_m::delay::Delay::new(ctx.core.SYST, clocks.system_clock.get_freq().to_Hz());
        // Init LED pin
        let sio = Sio::new(pac.SIO);
        let rp_pins = rp_pico::Pins::new(
            pac.IO_BANK0,
            pac.PADS_BANK0,
            sio.gpio_bank0,
            &mut pac.RESETS,
        );
        let mut led = rp_pins.led.into_push_pull_output();
        led.set_low().unwrap();

        // Configure the PWM
        let pwm_slices = pwm::Slices::new(pac.PWM, &mut pac.RESETS);
        let pwm_rg = pwm_slices.pwm3;
        let pwm_b = pwm_slices.pwm4;

        let mut rgb =
            picodisplay::RGB::new(rp_pins.gpio6, rp_pins.gpio7, rp_pins.gpio8, pwm_rg, pwm_b);
        rgb.set_brightness(50);
        rgb.set_color(Rgb888::GREEN);

        let picodisplay = create_picodisplay!(rp_pins, pac, delay);

        let mut buttons = make_buttons!(rp_pins);
        buttons.enable_interrupts(gpio::Interrupt::EdgeLow, true);

        // Configure the serial UARTs
        let uart0 = uart_setup(
            rp_pins.gpio1.into_mode(),
            pac.UART0,
            &clocks.peripheral_clock,
            &mut pac.RESETS,
        );
        let uart1 = uart_setup(
            rp_pins.gpio5.into_mode(),
            pac.UART1,
            &clocks.peripheral_clock,
            &mut pac.RESETS,
        );

        // Set up the USB driver
        let mut usb_bus_uninit = ctx.local.usb_bus_uninit;
        usb_bus_uninit.write(UsbBusAllocator::new(hal::usb::UsbBus::new(
            pac.USBCTRL_REGS,
            pac.USBCTRL_DPRAM,
            clocks.usb_clock,
            true,
            &mut pac.RESETS,
        )));
        // SAFETY: This is ok because we just wrote a valid value above.
        let usb_bus = unsafe { usb_bus_uninit.assume_init_ref() };

        // Set up the USB Communications Class Device driver
        let usb_serial = SerialPort::new(usb_bus);
        let usb_serial2 = SerialPort::new(usb_bus);

        // Create a USB device with a fake VID and PID
        let usb_device = UsbDeviceBuilder::new(usb_bus, UsbVidPid(0x16c0, 0x27dd))
            .manufacturer("Fake company")
            .product("Serial port")
            .serial_number("TEST")
            .device_class(2) // from: https://www.usb.org/defined-class-codes
            .build();

        let monotonic = Rp2040Mono::new(pac.TIMER);

        // Spawn heartbeat task
        heartbeat::spawn().unwrap();

        // Return resources and timer
        (
            Shared {
                usb_serial,
                usb_serial2,
            },
            Local {
                buttons,
                led,
                usb_device,
                picodisplay,
                uart0,
                uart1,
            },
            init::Monotonics(monotonic),
        )
    }

    fn uart_setup<D, P>(
        pin: gpio::Pin<P, gpio::FunctionUart>,
        dev: D,
        peripheral_clock: &hal::clocks::PeripheralClock,
        resets: &mut pac::RESETS,
    ) -> UartDev<D, P>
    where
        D: uart::UartDevice,
        P: gpio::PinId + gpio::bank0::BankPinId,
        gpio::Pin<P, gpio::FunctionUart>: uart::Rx<D>,
    {
        // TODO: ValidPinMode should imply PinMode and BankPinId should imply PinId
        let rx_pin = pin.into_mode::<gpio::FunctionUart>();
        let uart_config = uart::UartConfig::new(
            9600.Hz(),
            uart::DataBits::Seven,
            Some(uart::Parity::Even),
            uart::StopBits::One,
        );
        // TODO: uart config should be Clone, and new() should take it by reference
        let mut uart = uart::UartPeripheral::new(dev, uart::Pins::default().rx(rx_pin), resets)
            .enable(uart_config, peripheral_clock.freq())
            .unwrap();
        uart.enable_rx_interrupt();
        uart
    }

    #[task(local = [led, picodisplay])]
    fn heartbeat(mut ctx: heartbeat::Context) {
        // Flicker the built-in LED
        _ = ctx.local.led.toggle();
        let screen = &mut ctx.local.picodisplay;
        r(screen);

        // Re-spawn this task after 1 second
        let one_second = Duration::<u64, MONO_NUM, MONO_DENOM>::from_ticks(ONE_SEC_TICKS);
        heartbeat::spawn_after(one_second).unwrap();
    }

    #[task(binds = UART0_IRQ, local = [uart0], shared = [usb_serial])]
    fn uart0_irq(mut ctx: uart0_irq::Context) {
        let uart: &mut Uart0 = ctx.local.uart0;
        let mut buf = [0; 32];
        let data = match uart.read_raw(&mut buf) {
            Ok(len) => &buf[0..len],
            Err(nb::Error::WouldBlock) => b"",
            Err(nb::Error::Other(uart::ReadError { discarded, .. })) => discarded,
        };
        ctx.shared.usb_serial.lock(|serial: &mut SerialPort<_>| {
            serial.write(data);
            serial.flush();
        })
    }

    #[task(binds = UART1_IRQ, local = [uart1], shared = [usb_serial])]
    fn uart1_irq(mut ctx: uart1_irq::Context) {
        let uart: &mut Uart1 = ctx.local.uart1;
        let mut buf = [0; 32];
        let len = match uart.read_raw(&mut buf) {
            Ok(len) => len,
            Err(nb::Error::WouldBlock) => 0,
            Err(nb::Error::Other(uart::ReadError { discarded, .. })) => discarded.len(),
        };
        let data = &mut buf[0..len];
        for b in data.iter_mut() {
            *b |= 0x80; // set bit 8 high to indicate uart 1
        }

        ctx.shared.usb_serial.lock(|serial: &mut SerialPort<_>| {
            serial.write(data);
            serial.flush();
        })
    }

    #[task(
    binds = USBCTRL_IRQ,
    priority=2,
    local = [usb_device],
    shared = [usb_serial, usb_serial2],
    )]
    fn usb_irq(ctx: usb_irq::Context) {
        let usb_device: &mut UsbDevice<_> = ctx.local.usb_device;

        let serial = ctx.shared.usb_serial;
        let usb_serial2 = ctx.shared.usb_serial2;
        // Poll the USB driver with all of our supported USB Classes
        let mut ready = false;
        (serial, usb_serial2).lock(|serial: &mut SerialPort<_>, usb_serial2| {
            ready = usb_device.poll(&mut [serial, usb_serial2]);
            if ready {
                let mut buf = [0u8; 0];
                serial.read(&mut buf);
                usb_serial2.read(&mut buf);
            }
        });
    }

    #[task(binds = IO_IRQ_BANK0, priority = 1, local = [buttons])]
    fn button_irq(ctx: button_irq::Context) {
        use core::sync::atomic::Ordering;
        ctx.local
            .buttons
            .a
            .clear_interrupt(gpio::Interrupt::EdgeLow);
        let x = BTN_CTR.load(Ordering::Relaxed);
        BTN_CTR.store(x + 1, Ordering::Relaxed);
    }
}

static BTN_CTR: AtomicU32 = AtomicU32::new(0);

fn r(disp: &mut PicoDisplay) {
    use embedded_graphics::mono_font::ascii::FONT_10X20;
    use embedded_graphics::mono_font::MonoTextStyleBuilder;
    use embedded_graphics::pixelcolor::Rgb565;
    use embedded_graphics::prelude::*;
    use embedded_graphics::primitives::{PrimitiveStyle, Rectangle, Triangle};
    use embedded_graphics::text::{Alignment, Text};

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
