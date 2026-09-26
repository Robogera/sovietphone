use panic_halt as _;

const START_BYTE: u8 = 0x7E;
const VERSION_BYTE: u8 = 0xFF;
const DATA_LENGTH: u8 = 0x06;
const END_BYTE: u8 = 0xEF;
const FEEDBACK: u8 = 0x00;

const CMD_PLAY_TRACK: u8 = 0x03;
const CMD_SET_VOLUME: u8 = 0x06;
const CMD_LOOP_TRACK: u8 = 0x08;
const CMD_STOP: u8 = 0x16;

pub enum DFPlayerCommand {
    Stop,
    PlayTrack(u16),
    LoopTrack(u16),
    SetVolume(u16),
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

pub fn send<W: WriteFrame>(serial: &mut W, command: DFPlayerCommand) {
    let (command_code, [param_high, param_low]) = match command {
        DFPlayerCommand::Stop => (CMD_STOP, 0u16.to_be_bytes()),
        DFPlayerCommand::PlayTrack(track) => (CMD_PLAY_TRACK, track.to_be_bytes()),
        DFPlayerCommand::LoopTrack(track) => (CMD_LOOP_TRACK, track.to_be_bytes()),
        DFPlayerCommand::SetVolume(volume) => (CMD_LOOP_TRACK, volume.min(30).to_be_bytes()),
    };

    let (checksum_high, checksum_low) = get_checksum(
        VERSION_BYTE,
        DATA_LENGTH,
        command_code,
        FEEDBACK,
        param_high,
        param_low,
    );

    let frame = [
        START_BYTE,
        VERSION_BYTE,
        DATA_LENGTH,
        command_code,
        FEEDBACK,
        param_high,
        param_low,
        checksum_high,
        checksum_low,
        END_BYTE,
    ];

    serial.write_frame(frame);
    arduino_hal::delay_ms(50);
}
