#![no_std]
#![no_main]

mod debounce;
mod dfplayer;

use dfplayer::DFPlayerCommand;

use debounce::{Debounced, Edge, Measurable};

use panic_halt as _;

const TIME_STEP_MS: u32 = 5;
const TIME_STEP_RING_BELL_MS: u32 = 30;
const WAIT_AFTER_LAST_DIGIT_MS: u32 = 3000;
const RETURN_CALL_DELAY_MS: u32 = 8500;
const CORRECT_NUMBER: u8 = 16;
const COIL_ON_MS: u32 = 1000;
const COIL_OFF_MS: u32 = 4000;

const VOLUME: u16 = 28;

use arduino_hal::port::{mode, Pin};

impl<PIN> Measurable for Pin<mode::Input<mode::PullUp>, PIN>
where
    PIN: arduino_hal::port::PinOps,
{
    fn is_up(&self) -> bool {
        // <Self>::is_high(self)
        self.is_high()
    }
}

impl<USART, RX, TX> dfplayer::WriteFrame for arduino_hal::Usart<USART, RX, TX>
where
    USART: arduino_hal::hal::usart::UsartOps<arduino_hal::hal::Atmega, RX, TX>,
{
    fn write_frame(&mut self, frame: [u8; 10]) {
        for byte in frame {
            self.write_byte(byte);
        }
    }
}

enum State {
    IdleWaitingForCallMs(Option<u32>),
    WaitingForDial,
    Dialing {
        number: u8,
        pulses: u8,
        wait_ms: u32,
    },
    Call,
    WrongNumber,
    AudioEnded,
    Ringing {
        coil: Option<Coil>,
        wait_ms: u32,
    },
}

enum Input {
    Hook(HookInput),
    Dial(DialState),
    PlaybackOver,
    Pulse,
}

enum DialState {
    Moving,
    Stopped,
}

enum HookInput {
    On,
    Off,
}

#[repr(u16)]
enum Track {
    VoiceOutgoing = 1u16,
    VoiceIncoming = 2u16,
    ShortBeep = 3u16,
    ContinuousBeep = 4u16,
}

#[derive(Clone, Copy)]
enum Coil {
    Left,
    Right,
}

impl Coil {
    fn flip(&self) -> Coil {
        use Coil::*;
        match self {
            Right => Left,
            Left => Right,
        }
    }
}

enum Effect {
    PlayTrack(Track),
    LoopTrack(Track),
    StopPlayback,
    PowerCoil(Option<Coil>),
    Debug(u32),
}

use Coil::*;
use DialState::*;
use Effect::*;
use HookInput::*;
use Input::*;
use State::*;
use Track::*;
use ufmt::uWrite;

impl State {
    fn next(&mut self, input: Option<Input>) -> Option<Effect> {
        let (maybe_new_state, effect) = apply_next_state(self, input);
        if let Some(new_state) = maybe_new_state {
            *self = new_state;
        }
        effect
    }
}

fn apply_next_state(state: &State, input: Option<Input>) -> (Option<State>, Option<Effect>) {
    match (state, input) {
        // Play a continuous beep when the phone is off the hook
        (IdleWaitingForCallMs(_), Some(Hook(Off))) => {
            (Some(WaitingForDial), Some(LoopTrack(ContinuousBeep)))
        }

        // Start ringing when the time comes for scheduled incoming call
        (IdleWaitingForCallMs(Some(0)), None) => (
            Some(Ringing {
                coil: Some(Left),
                wait_ms: COIL_ON_MS,
            }),
            Some(PowerCoil(Some(Left))),
        ),

        // Decrement time counter
        (IdleWaitingForCallMs(Some(time_ms)), None) => (
            Some(IdleWaitingForCallMs(Some(
                time_ms.saturating_sub(TIME_STEP_MS),
            ))),
            None,
        ),

        // Stop the ringing and play incoming call track
        // when the phone is off the hook
        (
            Ringing {
                coil: _,
                wait_ms: _,
            },
            Some(Hook(Off)),
        ) => (Some(Call), Some(PlayTrack(VoiceIncoming))),

        // Switch the ring bell on/off in a realistic manner
        (Ringing { coil, wait_ms: 0 }, _) => {
            let next_coil = coil.xor(Some(Left));
            (
                Some(Ringing {
                    coil: next_coil.clone(),
                    wait_ms: match next_coil {
                        None => COIL_OFF_MS,
                        Some(_) => COIL_ON_MS,
                    },
                }),
                Some(PowerCoil(next_coil)),
            )
        }

        // Count down time when ringing, alternate coils
        (Ringing { coil, wait_ms }, _) => {
            let next_coil = coil.as_ref().map(Coil::flip);
            (
                Some(Ringing {
                    coil: next_coil.clone(),
                    wait_ms: wait_ms.saturating_sub(TIME_STEP_MS + TIME_STEP_RING_BELL_MS),
                }),
                Some(PowerCoil(next_coil)),
            )
        }

        // Shut the continuous beep when the dial starts moving
        (WaitingForDial, Some(Dial(Moving))) => (
            Some(Dialing {
                number: 0,
                pulses: 0,
                wait_ms: WAIT_AFTER_LAST_DIGIT_MS,
            }),
            Some(StopPlayback),
        ),

        // Count the pulses when the rotary dial comes to rest
        (
            Dialing {
                number,
                pulses,
                wait_ms: _,
            },
            Some(Dial(Stopped)),
        ) => {
            let new_number = if *pulses > 0 {
                number.saturating_mul(10).saturating_add(*pulses)
            } else {
                *number
            };
            (
                Some(Dialing {
                    number: new_number,
                    pulses: 0,
                    wait_ms: WAIT_AFTER_LAST_DIGIT_MS,
                }),
                Some(Debug(new_number as u32)),
            )
        }

        // Increment the pulse counter
        (
            Dialing {
                number,
                pulses,
                wait_ms: _,
            },
            Some(Pulse),
        ) => (
            Some(Dialing {
                number: *number,
                pulses: pulses.wrapping_add(1),
                wait_ms: WAIT_AFTER_LAST_DIGIT_MS,
            }),
            None,
        ),

        // Win! Play the outgoing call audio if user has guessed the secret number!!!
        (
            Dialing {
                number: CORRECT_NUMBER,
                pulses: _,
                wait_ms: 0,
            },
            None,
        ) => (Some(Call), Some(PlayTrack(VoiceOutgoing))),

        // Play the busy signal if the user didn't guess the number.
        (
            Dialing {
                number: _,
                pulses: _,
                wait_ms: 0,
            },
            None,
        ) => (Some(WrongNumber), Some(LoopTrack(ShortBeep))),

        // Keep the countdown going
        (
            Dialing {
                number,
                pulses,
                wait_ms,
            },
            None,
        ) => (
            Some(Dialing {
                number: *number,
                pulses: *pulses,
                wait_ms: wait_ms.saturating_sub(TIME_STEP_MS),
            }),
            None,
        ),

        // User waited until the voice clip ended. Play line busy signal
        (Call, Some(PlaybackOver)) => (Some(AudioEnded), Some(LoopTrack(ShortBeep))),

        // User hanged the phone during audio playback (rude) or after the audio has ended, back to idle
        (Call | AudioEnded, Some(Hook(On))) => {
            (Some(IdleWaitingForCallMs(None)), Some(StopPlayback))
        }

        // Schedule the return call if the phone was put down
        // after any incorrect interaction
        (
            WaitingForDial
            | Dialing {
                number: _,
                pulses: _,
                wait_ms: _,
            }
            | WrongNumber,
            Some(Hook(On)),
        ) => (
            Some(IdleWaitingForCallMs(Some(RETURN_CALL_DELAY_MS))),
            Some(StopPlayback),
        ),

        // Catchall. Ignoring all other inputs
        (_, _) => (None, None),
    }
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

    let mut led = pins.d13.into_output();
    led.set_low();

    let mut coil1 = pins.d8.into_output();
    let mut coil2 = pins.d9.into_output();
    coil1.set_low();
    coil2.set_low();

    let mut dfplayer_busy = Debounced::new(pins.d4.into_pull_up_input());
    let mut phone_hook = Debounced::new(pins.d5.into_pull_up_input());
    let mut dial_moving = Debounced::new(pins.d6.into_pull_up_input());
    let mut dial_pulse = Debounced::new(pins.d7.into_pull_up_input());

    let mut state = State::IdleWaitingForCallMs(None);

    arduino_hal::delay_ms(1000);
    dfplayer::send(&mut serial, DFPlayerCommand::Stop);
    arduino_hal::delay_ms(150);
    dfplayer::send(&mut serial, DFPlayerCommand::SetVolume(VOLUME));
    arduino_hal::delay_ms(150);

    dfplayer::send(&mut serial, DFPlayerCommand::Stop);

    loop {
        let dfplayer_busy_edge = dfplayer_busy.poll();
        let phone_hook_edge = phone_hook.poll();
        let dial_moving_edge = dial_moving.poll();
        let dial_pulse_edge = dial_pulse.poll();

        let input = phone_hook_edge
            .map(|edge| match edge {
                Edge::Falling => Hook(Off),
                Edge::Rising => Hook(On),
            })
            .or_else(|| match dfplayer_busy_edge {
                Some(Edge::Rising) => Some(PlaybackOver),
                _ => None,
            })
            .or_else(|| match dial_moving_edge {
                Some(Edge::Rising) => Some(Dial(Stopped)),
                Some(Edge::Falling) => Some(Dial(Moving)),
                _ => None,
            })
            .or_else(|| match dial_pulse_edge {
                Some(Edge::Rising) => Some(Pulse),
                _ => None,
            });

        if let Some(effect) = state.next(input) {
            match effect {
                Debug(number) => {
                    if number == CORRECT_NUMBER as u32 {
                          led.toggle();
                          arduino_hal::delay_ms(400);
                          led.toggle();
                    }
                    // for byte in number.to_be_bytes() {
                    //     serial.write_byte(byte);
                    // }
                }
                PlayTrack(track) => {
                    dfplayer::send(&mut serial, DFPlayerCommand::Stop);
                    arduino_hal::delay_ms(50);
                    dfplayer::send(&mut serial, DFPlayerCommand::PlayTrack(track as u16))
                }
                LoopTrack(track) => {
                    dfplayer::send(&mut serial, DFPlayerCommand::Stop);
                    arduino_hal::delay_ms(50);
                    dfplayer::send(&mut serial, DFPlayerCommand::LoopTrack(track as u16))
                }
                StopPlayback => dfplayer::send(&mut serial, DFPlayerCommand::Stop),
                PowerCoil(coil) => match coil {
                    Some(Left) => {
                        coil1.set_low();
                        coil2.set_high();
                        arduino_hal::delay_ms(TIME_STEP_RING_BELL_MS);
                    }
                    Some(Right) => {
                        coil2.set_low();
                        coil1.set_high();
                        arduino_hal::delay_ms(TIME_STEP_RING_BELL_MS);
                    }
                    None => {
                        coil2.set_low();
                        coil1.set_low();
                    }
                },
            };
        }
        arduino_hal::delay_ms(TIME_STEP_MS);
    }
}
