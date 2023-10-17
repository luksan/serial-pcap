#![no_std]
#![no_main]
#![allow(unused_must_use)]

use core::fmt::Write;
use core::sync::atomic::AtomicU32;
use core::sync::atomic::Ordering;

use arrayvec::ArrayString;
use embedded_graphics::prelude::*;
use rp2040_hal::gpio::PullNone;
use rp2040_hal::typelevel::{OptionTNone, OptionTSome};
use rp_pico::hal::{self, gpio, uart};
use rp_pico::pac;
// USB Device support
use usb_device::{class_prelude::*, prelude::*};
// USB Communications Class Device support
use usbd_serial::SerialPort;

use rp_rs422_cap::picodisplay::{self, Buttons};

type UartRxPin<P> = gpio::Pin<P, gpio::FunctionUart, PullNone>;

type UartDev<D, P> = uart::UartPeripheral<
    uart::Enabled,
    D,
    uart::Pins<OptionTNone, OptionTSome<UartRxPin<P>>, OptionTNone, OptionTNone>,
>;

type Uart0 = UartDev<pac::UART0, gpio::bank0::Gpio1>;
type Uart1 = UartDev<pac::UART1, gpio::bank0::Gpio5>;

mod disp_info;

#[rtic::app(device = pac, dispatchers = [TIMER_IRQ_1, TIMER_IRQ_2])]
mod app {
    use core::mem::MaybeUninit;
    use core::sync::atomic::AtomicI32;

    use embedded_graphics::pixelcolor::Rgb888;
    use embedded_hal::digital::v2::{OutputPin, ToggleableOutputPin};
    use hal::clocks::ClockSource;
    use panic_probe as _;
    use rp2040_hal::gpio::FunctionSioOutput;
    use rp2040_monotonic::{
        fugit::Duration,
        fugit::RateExtU32, // For .kHz() conversion funcs
        Rp2040Monotonic,
    };
    use rp_pico::hal::{gpio::bank0::Gpio25, pac, pwm, sio::Sio, Clock};
    use rp_pico::XOSC_CRYSTAL_FREQ;
    use x328_proto::scanner;
    use x328_proto::scanner::ControllerEvent;

    use rp_rs422_cap::x328_bus::{FieldBus, UartBuf, UpdateEvent};
    use rp_rs422_cap::{create_picodisplay, make_buttons, picodisplay::PicoDisplay};

    use crate::disp_info::{DisplayUpdates, Info};

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
        x328_scanner: scanner::Scanner,
        display_updates: DisplayUpdates,
    }

    #[local]
    struct Local {
        buttons: Buttons,
        picodisplay: disp_info::BusDisplay,
        led: gpio::Pin<Gpio25, FunctionSioOutput, gpio::PullDown>,
        usb_device: UsbDevice<'static, hal::usb::UsbBus>,
        uart0: Uart0,
        uart1: Uart1,
    }

    #[init(local=[
        usb_bus_uninit: MaybeUninit<UsbBusAllocator<hal::usb::UsbBus>> = MaybeUninit::uninit(),
        display_updates: DisplayUpdates = DisplayUpdates::new(),
    ])]
    fn init(ctx: init::Context) -> (Shared, Local, init::Monotonics) {
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
        let mut picodisplay = disp_info::BusDisplay::new(picodisplay.screen);

        let buttons = make_buttons!(rp_pins);
        buttons.enable_interrupts(gpio::Interrupt::EdgeLow, true);

        // Configure the serial UARTs
        let uart0 = uart_setup(
            rp_pins.gpio1,
            pac.UART0,
            &clocks.peripheral_clock,
            &mut pac.RESETS,
        );
        let uart1 = uart_setup(
            rp_pins.gpio5,
            pac.UART1,
            &clocks.peripheral_clock,
            &mut pac.RESETS,
        );

        // Set up the USB driver
        let usb_bus_uninit = ctx.local.usb_bus_uninit;
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
        let usb_serial2 = SerialPort::new(usb_bus);
        let usb_serial = SerialPort::new(usb_bus);

        // Create a USB device with a fake VID and PID
        let usb_device = UsbDeviceBuilder::new(usb_bus, UsbVidPid(0x16c0, 0x27dd))
            .manufacturer("Fake company")
            .product("Serial port")
            .serial_number("TEST")
            .device_class(usbd_serial::USB_CLASS_CDC) // from: https://www.usb.org/defined-class-codes
            .build();

        let monotonic = Rp2040Mono::new(pac.TIMER);

        // Spawn heartbeat task
        heartbeat::spawn().unwrap();

        picodisplay.redraw();

        // Return resources and timer
        (
            Shared {
                usb_serial,
                usb_serial2,
                x328_scanner: Default::default(),
                display_updates: DisplayUpdates::new(),
            },
            Local {
                buttons,
                picodisplay,
                led,
                usb_device,
                uart0,
                uart1,
            },
            init::Monotonics(monotonic),
        )
    }

    fn uart_setup<D, P>(
        pin: gpio::Pin<P, gpio::FunctionNull, gpio::PullDown>,
        dev: D,
        peripheral_clock: &hal::clocks::PeripheralClock,
        resets: &mut pac::RESETS,
    ) -> UartDev<D, P>
    where
        D: uart::UartDevice,
        P: gpio::PinId + uart::ValidPinIdRx<D> + gpio::ValidFunction<gpio::FunctionUart>,
    {
        let rx_pin = pin.into_pull_type().into_function::<gpio::FunctionUart>();
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
        uart.set_fifos(false);
        uart.enable_rx_interrupt();
        uart
    }

    #[idle(local = [picodisplay], shared = [display_updates])]
    fn idle(mut ctx: idle::Context) -> ! {
        let disp = ctx.local.picodisplay;
        loop {
            let age = DISP_AGE.load(Ordering::SeqCst);
            let info = ctx.shared.display_updates.lock(|u| u.next_change());
            if let Some(update) = info {
                disp.update_info(update, age + 1);
            }
            disp.check_age(age);
        }
    }
    static DISP_AGE: AtomicI32 = AtomicI32::new(0);

    #[task(local = [led])]
    fn heartbeat(ctx: heartbeat::Context) {
        // Flicker the built-in LED
        _ = ctx.local.led.toggle();
        let age = DISP_AGE.load(Ordering::SeqCst);
        DISP_AGE.store(age + 1, Ordering::SeqCst);

        // Re-spawn this task after 1 second
        let one_second = Duration::<u64, MONO_NUM, MONO_DENOM>::from_ticks(ONE_SEC_TICKS);
        heartbeat::spawn_after(one_second).unwrap();
    }

    #[task(
        capacity = 1,
        priority = 2,
        shared = [ usb_serial2, display_updates ],
        local = [
            ctrl_ev: ControllerEvent = ControllerEvent::NodeTimeout,
            fb: FieldBus = FieldBus::new(),
        ])]
    fn x328_event_handler(mut ctx: x328_event_handler::Context, ev: scanner::Event) {
        use scanner::{ControllerEvent, Event, NodeEvent};
        let mut msg = ArrayString::<100>::new();
        let fb = ctx.local.fb;
        let ctrl_ev = ctx.local.ctrl_ev;
        let mut update_event = None;
        match ev {
            Event::Ctrl(ev) => {
                if matches!(ev, ControllerEvent::NodeTimeout) {
                    match ctrl_ev {
                        ControllerEvent::Write(a, p, v) => {
                            write!(msg, "Timeout node {} write param {} = {}", **a, **p, **v);
                            update_event = fb.update_parameter(*a, *p, *v);
                        }
                        ControllerEvent::Read(a, p) => {
                            write!(msg, "Timeout node {} read param {}", **a, **p);
                        }
                        _ => {}
                    }
                }
                *ctrl_ev = ev;
            }
            Event::Node(ev) => match (ev, ctrl_ev) {
                (NodeEvent::Write(Ok(_)), ControllerEvent::Write(a, p, v)) => {
                    update_event = fb.update_parameter(*a, *p, *v);
                    write!(msg, "Node {} write ok {} = {}", **a, **p, **v);
                }
                (NodeEvent::Read(Ok(v)), ControllerEvent::Read(a, p)) => {
                    update_event = fb.update_parameter(*a, *p, v);
                    write!(msg, "Node {} read ok {} == {}", **a, **p, *v);
                }
                (NodeEvent::UnexpectedTransmission, _) => {}
                _ => {}
            },
        }
        if !msg.is_empty() {
            msg.push_str("\r\n");

            ctx.shared.usb_serial2.lock(|serial| {
                serial.write(msg.as_bytes());
                serial.flush();
            });
        }
        if let Some(event) = update_event {
            ctx.shared.display_updates.lock(|disp| match event {
                UpdateEvent::StowPress(e, w) => {
                    disp.set_info(Info::StowPressEast(e));
                    disp.set_info(Info::StowPressWest(w));
                }
                UpdateEvent::IoboxInputs(i) => disp.set_info(Info::IoboxInputs(i)),
                UpdateEvent::IoboxCmd(c) => disp.set_info(Info::IoboxCmd(c)),
                UpdateEvent::IoboxOutputs(o) => disp.set_info(Info::IoboxOutputs(o)),
                UpdateEvent::PolarSpeedCmd(s) => disp.set_info(Info::PolarSpeedCmd(s)),
                UpdateEvent::PolarEncoder(v) => disp.set_info(Info::PolEncVal(v)),
                UpdateEvent::DeclinationEncoder(v) => disp.set_info(Info::DeclEncVal(v)),
            });
        }
    }

    // Received from x3.28 node
    #[task(binds = UART0_IRQ, priority = 2, local = [uart0, buf: UartBuf = UartBuf::new()], shared = [usb_serial, x328_scanner])]
    fn uart0_irq(mut ctx: uart0_irq::Context) {
        let uart: &mut Uart0 = ctx.local.uart0;
        let buf = ctx.local.buf;
        ctx.shared.usb_serial.lock(|serial: &mut SerialPort<_>| {
            let tail = buf.tail_slice(1);
            let len = match uart.read_raw(tail) {
                Ok(len) => len,
                Err(nb::Error::WouldBlock) => 0,
                Err(nb::Error::Other(uart::ReadError { discarded, .. })) => discarded.len(),
            };
            let _ = serial.write(&tail[0..len]);
            let _ = serial.flush();
            buf.incr_len(len);
        });
        ctx.shared.x328_scanner.lock(|s| {
            let (consumed, event) = s.recv_from_node(buf);
            buf.consume(consumed);
            if let Some(event) = event {
                let _ = x328_event_handler::spawn(event.into());
            }
        });
    }

    // Received from bus controller
    #[task(binds = UART1_IRQ, priority = 1, local = [uart1, buf: UartBuf = UartBuf::new()], shared = [usb_serial, x328_scanner])]
    fn uart1_irq(mut ctx: uart1_irq::Context) {
        let uart: &mut Uart1 = ctx.local.uart1;
        let buf = ctx.local.buf;
        let tail = buf.tail_slice(1);
        let len = match uart.read_raw(tail) {
            Ok(len) => len,
            Err(nb::Error::WouldBlock) => 0,
            Err(nb::Error::Other(uart::ReadError { discarded, .. })) => discarded.len(),
        };
        let tail = &mut tail[0..len];
        for b in tail.iter_mut() {
            *b |= 0x80; // set bit 8 high to indicate uart 1
        }

        ctx.shared.usb_serial.lock(|serial: &mut SerialPort<_>| {
            let _ = serial.write(tail);
            let _ = serial.flush();
        });
        for b in tail.iter_mut() {
            *b &= 0x7f; // clear bit 8 again
        }
        buf.incr_len(len);

        ctx.shared.x328_scanner.lock(|s| {
            let (consumed, event) = s.recv_from_ctrl(buf);
            buf.consume(consumed);
            if let Some(event) = event {
                let _ = x328_event_handler::spawn(event.into());
            }
        });
    }

    #[task(
    binds = USBCTRL_IRQ,
    priority=3,
    local = [usb_device],
    shared = [usb_serial, usb_serial2],
    )]
    fn usb_irq(ctx: usb_irq::Context) {
        let usb_device: &mut UsbDevice<_> = ctx.local.usb_device;

        let serial = ctx.shared.usb_serial;
        let usb_serial2 = ctx.shared.usb_serial2;
        // Poll the USB driver with all of our supported USB Classes
        let mut ready = false;
        (serial, usb_serial2).lock(|ser1: &mut SerialPort<_>, ser2| {
            ready = usb_device.poll(&mut [ser2, ser1]);
            if ready {
                let mut buf = [0u8; 0];
                ser1.read(&mut buf);
                ser2.read(&mut buf);
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
