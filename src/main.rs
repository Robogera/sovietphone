#![no_std]
#![no_main]

use debouncr::debounce_stateful_16;
use panic_halt as _;

const TIME_DELTA_MS: u32 = 10;

const START_BYTE: u8 = 0x7E;
const VERSION_BYTE: u8 = 0xFF;
const DATA_LENGTH: u8 = 0x06;
const END_BYTE: u8 = 0xEF;
const FEEDBACK: u8 = 0x00;

const VOLUME_MAX: u8 = 23;
const FADEOUT_DURATION_MS: u32 = 1000;

const GRACE_MS: u32 = 1000;

const CMD_SET_VOLUME: u8 = 0x06;
const CMD_PLAY_TRACK: u8 = 0x03;
const CMD_STOP: u8 = 0x16;

enum PlayerState {
    Stopped,
    Playing,
}

fn get_checksum(
    version: u8,
    length: u8,
    command: u8,
    feedback: u8,
    param_high: u8,
    param_low: u8,
) -> (u8, u8) {
    let sum = version as u16
        + length as u16
        + command as u16
        + feedback as u16
        + param_high as u16
        + param_low as u16;
    let checksum = 0u16.wrapping_sub(sum);
    ((checksum >> 8) as u8, (checksum & 0xFF) as u8)
}

pub trait WriteFrame {
    fn write_frame(&mut self, frame: [u8; 10]);
}

impl<USART, RX, TX> WriteFrame for arduino_hal::Usart<USART, RX, TX>
where
    USART: arduino_hal::hal::usart::UsartOps<arduino_hal::hal::Atmega, RX, TX>,
{
    fn write_frame(&mut self, frame: [u8; 10]) {
        for byte in frame {
            self.write_byte(byte);
        }
    }
}

fn dfplayer_command<W: WriteFrame>(serial: &mut W, command: u8, param_high: u8, param_low: u8) {
    let (checksum_high, checksum_low) = get_checksum(
        VERSION_BYTE,
        DATA_LENGTH,
        command,
        FEEDBACK,
        param_high,
        param_low,
    );

    let frame = [
        START_BYTE,
        VERSION_BYTE,
        DATA_LENGTH,
        command,
        FEEDBACK,
        param_high,
        param_low,
        checksum_high,
        checksum_low,
        END_BYTE,
    ];

    serial.write_frame(frame);
    arduino_hal::delay_ms(150);
}

#[arduino_hal::entry]
fn main() -> ! {
    let dp = arduino_hal::Peripherals::take().unwrap();
    let pins = arduino_hal::pins!(dp);

    // DFPlayer requires 9600baud
    // RX pin is not actually connected to the player because we
    // share the physical serial with the usb uart
    // and it would mess up the code upload process
    let mut serial = arduino_hal::Usart::new(
        dp.USART0,
        pins.d0,
        pins.d1.into_output(),
        arduino_hal::hal::usart::BaudrateArduinoExt::into_baudrate(9600),
    );

    let button = pins.d3.into_pull_up_input();
    let busy = pins.d6.into_floating_input();

    let mut led = pins.d13.into_output();
    led.set_low();

    let mut player_state = PlayerState::Stopped;

    arduino_hal::delay_ms(1000);

    let mut debounced_button = debounce_stateful_16(button.is_high());

    loop {
        // Maybe add another debouncer ring buffer? Seems to work ok tho
        if busy.is_high() {
            led.set_high()
        } else {
            led.set_low()
        }

        player_state = match player_state {
            // When the voice stops
            PlayerState::Playing if busy.is_high() => PlayerState::Stopped,
            // Never happens, but if in some bizzare case the arduino is
            // rebootet during the playback this should kill the audio
            PlayerState::Stopped if busy.is_low() => {
                dfplayer_command(&mut serial, CMD_STOP, 0, 0);
                PlayerState::Stopped
            }
            state => state,
        };

        if let Some(_) = debounced_button.update(button.is_high()) {
            player_state = match player_state {
                PlayerState::Stopped => {
                    dfplayer_command(&mut serial, CMD_SET_VOLUME, 0, VOLUME_MAX);
                    dfplayer_command(&mut serial, CMD_PLAY_TRACK, 0, 1);
                    while busy.is_high() {
                        arduino_hal::delay_ms(TIME_DELTA_MS);
                    }
                    PlayerState::Playing
                }
                PlayerState::Playing => {
                    for volume in (0..VOLUME_MAX).rev() {
                        dfplayer_command(&mut serial, CMD_SET_VOLUME, 0x00, volume);
                    }
                    dfplayer_command(&mut serial, CMD_STOP, 0, 0);
                    while busy.is_low() {
                        arduino_hal::delay_ms(TIME_DELTA_MS);
                    }
                    PlayerState::Stopped
                }
            };

            // Dropping previous button values to prevent weird things
            // from happening if button was messed with during the
            // fadeout grace period
            debounced_button = debounce_stateful_16(button.is_high());
        }

        arduino_hal::delay_ms(TIME_DELTA_MS);
    }
}
