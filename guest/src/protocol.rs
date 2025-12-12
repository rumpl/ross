//! Wire protocol for TTY communication between host and guest.
//!
//! This is a copy of the protocol from ross-shim, kept separate since
//! ross-guest targets Linux while ross-shim targets macOS.

pub const CMD_MASK: u16 = 0x3;
pub const CMD_SHIFT: u32 = 2;

// Guest → Host commands
pub const CMD_WRITE_STDOUT: u16 = 0;
pub const CMD_WRITE_STDERR: u16 = 1;
pub const CMD_EXIT: u16 = 2;

// Host → Guest commands
pub const CMD_WRITE_STDIN: u16 = 0;
pub const CMD_UPDATE_SIZE: u16 = 1;

#[inline]
pub fn encode_write_cmd(opcode: u16, data_len: usize) -> u16 {
    opcode | ((data_len as u16) << CMD_SHIFT)
}

#[inline]
pub fn encode_exit_cmd(exit_code: u8) -> u16 {
    CMD_EXIT | ((exit_code as u16) << CMD_SHIFT)
}

#[inline]
pub fn decode_cmd(cmd: u16) -> (u16, usize) {
    let opcode = cmd & CMD_MASK;
    let value = (cmd >> CMD_SHIFT) as usize;
    (opcode, value)
}
