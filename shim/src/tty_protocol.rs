//! Wire protocol for TTY communication between host and guest.
//!
//! This implements a simple binary protocol compatible with muvm for forwarding
//! stdin/stdout/stderr and terminal resize events over vsock.
//!
//! Message format:
//! - Command word: 2 bytes (little-endian)
//!   - Bits 0-1: opcode
//!   - Bits 2-15: payload (data length for writes, exit code for exit)
//! - For write commands: followed by `payload` bytes of data
//! - For resize commands: followed by 4 bytes (cols: u16 LE, rows: u16 LE)

pub const CMD_MASK: u16 = 0x3;
pub const CMD_SHIFT: u32 = 2;

// Guest → Host commands
pub const CMD_WRITE_STDOUT: u16 = 0;
pub const CMD_WRITE_STDERR: u16 = 1;
pub const CMD_EXIT: u16 = 2;

// Host → Guest commands
pub const CMD_WRITE_STDIN: u16 = 0;
pub const CMD_UPDATE_SIZE: u16 = 1;

/// Maximum data length that can be encoded (14 bits = 16383 bytes)
pub const MAX_DATA_LEN: usize = (1 << 14) - 1;

/// Encode a write command (stdout/stderr from guest, stdin from host)
#[inline]
pub fn encode_write_cmd(opcode: u16, data_len: usize) -> u16 {
    debug_assert!(data_len <= MAX_DATA_LEN);
    opcode | ((data_len as u16) << CMD_SHIFT)
}

/// Encode an exit command with exit code
#[inline]
pub fn encode_exit_cmd(exit_code: u8) -> u16 {
    CMD_EXIT | ((exit_code as u16) << CMD_SHIFT)
}

/// Decode a command word into (opcode, payload_value)
/// For write commands, payload_value is the data length.
/// For exit commands, payload_value is the exit code.
#[inline]
pub fn decode_cmd(cmd: u16) -> (u16, usize) {
    let opcode = cmd & CMD_MASK;
    let value = (cmd >> CMD_SHIFT) as usize;
    (opcode, value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encode_decode_write() {
        let cmd = encode_write_cmd(CMD_WRITE_STDOUT, 100);
        let (opcode, len) = decode_cmd(cmd);
        assert_eq!(opcode, CMD_WRITE_STDOUT);
        assert_eq!(len, 100);
    }

    #[test]
    fn test_encode_decode_stderr() {
        let cmd = encode_write_cmd(CMD_WRITE_STDERR, 256);
        let (opcode, len) = decode_cmd(cmd);
        assert_eq!(opcode, CMD_WRITE_STDERR);
        assert_eq!(len, 256);
    }

    #[test]
    fn test_encode_decode_exit() {
        let cmd = encode_exit_cmd(42);
        let (opcode, code) = decode_cmd(cmd);
        assert_eq!(opcode, CMD_EXIT);
        assert_eq!(code, 42);
    }

    #[test]
    fn test_encode_decode_stdin() {
        let cmd = encode_write_cmd(CMD_WRITE_STDIN, 1024);
        let (opcode, len) = decode_cmd(cmd);
        assert_eq!(opcode, CMD_WRITE_STDIN);
        assert_eq!(len, 1024);
    }

    #[test]
    fn test_max_data_length() {
        let cmd = encode_write_cmd(CMD_WRITE_STDOUT, MAX_DATA_LEN);
        let (opcode, len) = decode_cmd(cmd);
        assert_eq!(opcode, CMD_WRITE_STDOUT);
        assert_eq!(len, MAX_DATA_LEN);
    }

    #[test]
    fn test_zero_length() {
        let cmd = encode_write_cmd(CMD_WRITE_STDIN, 0);
        let (opcode, len) = decode_cmd(cmd);
        assert_eq!(opcode, CMD_WRITE_STDIN);
        assert_eq!(len, 0);
    }
}
