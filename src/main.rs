#![no_std]
#![no_main]

mod dfplayer;

use dfplayer::DFPlayerCommand;

use panic_halt as _;

const TIME_STEP_MS: u32 = 5;
const WAIT_AFTER_LAST_DIGIT_MS: u32 = 1250;
const RETURN_CALL_DELAY_MS: u32 = 8500;
const CORRECT_NUMBER: u8 = 16;
const COIL_ON_MS: u32 = 1000;
const COIL_OFF_MS: u32 = 4000;

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

enum CallType {
    Outgoing,
    Incoming,
}

type UntilReturnCallMs = u32;

enum State {
    Idle {
        until_return_call_ms: Option<u32>,
    },
    WaitingForDial,
    Dialing {
        number: u8,
        pulses: u8,
        wait_ms: u32,
    },
    Call(CallType),
    WrongNumber,
    AudioEnded,
    Ringing {
        coil: Option<Coil>,
        wait_ms: u32,
    },
}

enum Input {
    Hook(HookInput),
    Dial(DialInput),
    PlaybackOver,
}

enum DialInput {
    Moving,
    Stopped,
    Pulse,
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
}

impl State {
    fn next(&mut self, input: Option<Input>) -> Option<Effect> {
        let (new_state, effect) = apply_next_state(self, input);
        *self = new_state;
        effect
    }
}

fn apply_next_state(state: &State, input: Option<Input>) -> (State, Option<Effect>) {
    use CallType::*;
    use Coil::*;
    use DialInput::*;
    use Effect::*;
    use HookInput::*;
    use Input::*;
    use State::*;
    use Track::*;

    match (state, input) {
        // Play a continuous beep when the phone is off the hook
        (
            Idle {
                until_return_call_ms: _,
            },
            Some(Hook(Off)),
        ) => (WaitingForDial, Some(LoopTrack(ContinuousBeep))),

        // Start ringing when the time comes for scheduled incoming call
        (
            Idle {
                until_return_call_ms: Some(0),
            },
            None,
        ) => (
            Ringing {
                coil: Some(Left),
                wait_ms: COIL_ON_MS,
            },
            Some(PowerCoil(Some(Left))),
        ),

        // Stop the ringing when the phone is off the hook
        (
            Ringing {
                coil: _,
                wait_ms: _,
            },
            Some(Hook(Off)),
        ) => (Call(Incoming), Some(PlayTrack(VoiceIncoming))),

        // Switch the ring bell on/off
        (Ringing { coil, wait_ms: 0 }, _) => match coil {
            None => (
                Ringing {
                    coil: Some(Left),
                    wait_ms: COIL_ON_MS,
                },
                Some(PowerCoil(Some(Left))),
            ),
            Some(_) => (
                Ringing {
                    coil: None,
                    wait_ms: COIL_OFF_MS,
                },
                Some(PowerCoil(None)),
            ),
        },

        // Count down time when ringing
        (Ringing { coil, wait_ms }, _) => {
            let next_coil = coil.as_ref().map(Coil::flip);
            (
                Ringing {
                    coil: next_coil.clone(),
                    wait_ms: wait_ms.saturating_sub(TIME_STEP_MS),
                },
                Some(PowerCoil(next_coil)),
            )
        }

        // Shut the continuous beep when the dial starts moving
        (WaitingForDial, Some(Dial(Moving))) => (
            Dialing {
                number: 0,
                pulses: 0,
                wait_ms: WAIT_AFTER_LAST_DIGIT_MS,
            },
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
        ) => (
            Dialing {
                number: number.saturating_mul(10).saturating_add(*pulses),
                pulses: 0,
                wait_ms: WAIT_AFTER_LAST_DIGIT_MS,
            },
            None,
        ),

        // Increment the pulse counter
        (
            Dialing {
                number,
                pulses,
                wait_ms: _,
            },
            Some(Dial(Pulse)),
        ) => (
            Dialing {
                number: *number,
                pulses: pulses.wrapping_add(1),
                wait_ms: WAIT_AFTER_LAST_DIGIT_MS,
            },
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
        ) => (Call(Outgoing), Some(PlayTrack(VoiceOutgoing))),

        // User waited until the voice clip ended. Play line busy signal
        (Call(Outgoing), Some(PlaybackOver)) => (AudioEnded, Some(LoopTrack(ShortBeep))),

        // User hanged the phone during audio playback (rude) or after the audio has ended
        (Call(Outgoing) | AudioEnded, Some(Hook(On))) => (
            Idle {
                until_return_call_ms: None,
            },
            Some(StopPlayback),
        ),

        // Play the busy signal if the user didn't guess the number.
        (
            Dialing {
                number: _,
                pulses: _,
                wait_ms: 0,
            },
            None,
        ) => (WrongNumber, Some(LoopTrack(ShortBeep))),

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
            Idle {
                until_return_call_ms: Some(RETURN_CALL_DELAY_MS),
            },
            Some(StopPlayback),
        ),

        // Catchall
        (_, _) => (
            Idle {
                until_return_call_ms: None,
            },
            None,
        ),
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

    let dfplayer_busy = pins.d4.into_pull_up_input();
    let handset = pins.d5.into_pull_up_input();
    let dial_moved = pins.d6.into_pull_up_input();
    let dial_pulse = pins.d7.into_pull_up_input();

    let mut led = pins.d13.into_output();
    led.set_low();

    let mut coil1 = pins.d8.into_output();
    let mut coil2 = pins.d9.into_output();
    coil1.set_low();
    coil2.set_low();

    let mut state = State::Idle {
        until_return_call_ms: None,
    };

    arduino_hal::delay_ms(1000);

    dfplayer::send(&mut serial, DFPlayerCommand::Stop);

    loop {
        match state.next(Some(Input::Hook(HookInput::Off))) {
            None => dfplayer::send(&mut serial, DFPlayerCommand::PlayTrack(1)),
            Some(_) => {}
        };
        arduino_hal::delay_ms(TIME_STEP_MS);
    }
}
