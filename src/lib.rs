//! Implements the Tockloader protocol.
//!
//! TockOS applications are loaded with `tockloader`.
//! This speaks to the TockOS bootloader using a specific
//! protocol. This crate implements that protocol so
//! that you can write future tockloader compatible bootloaders
//! in Rust!

#![no_std]

// ****************************************************************************
//
// Imports
//
// ****************************************************************************

extern crate byteorder;

use byteorder::{LittleEndian, ByteOrder};

// ****************************************************************************
//
// Public Types
//
// ****************************************************************************

/// Commands supported by the protocol. A bootloader will decode these and a
/// flash tool will encode them.
#[derive(Debug, PartialEq)]
pub enum Command<'a> {
    /// Send a PING to the bootloader. It will drop its hp buffer and send
    /// back a PONG.
    Ping,
    /// Get info about the bootloader. The result is one byte of length, plus
    /// length bytes of string, followed by 192-length zeroes.
    Info,
    /// Get the Unique ID. Result is 8 bytes of unique ID (but I'm not sure
    /// what the result code should be).
    Id,
    /// Reset all TX and RX buffers.
    Reset,
    /// Erase a page. The RX buffer should contain the address of the start of
    /// the 512 byte page. Any non-page-aligned addresses will result in
    /// RES_BADADDR. This command is not required before writing a page, it is
    /// just an optimisation. It is particularly quick for already empty pages.
    ErasePage { address: u32 },
    /// Write a page in internal flash. The RX buffer should contain the 4
    /// byte address of the start of the page, followed by 512 bytes of page.
    WritePage { address: u32, data: &'a [u8] },
    /// Erase a block of pages in ex flash. The RX buffer should contain the
    /// address of the start of the block. Each block is 8 pages, so 2048
    /// bytes.
    EraseExBlock { address: u32 },
    /// Write a page to ex flash. The RX buffer should contain the address of
    /// the start of the 256 byte page, followed by 256 bytes of page.
    WriteExPage { address: u32, data: &'a [u8] },
    /// Get the length and CRC of the RX buffer. The response is two bytes of
    /// little endian length, followed by 4 bytes of crc32.
    CrcRxBuffer,
    /// Read a range from internal flash. The RX buffer should contain a 4
    /// byte address followed by 2 bytes of length. The response will be
    /// length bytes long.
    ReadRange { address: u32, length: u16 },
    /// Read a range from external flash. The RX buffer should contain a 4
    /// byte address followed by 2 bytes of length. The response will be
    /// length bytes long.
    ExReadRange { address: u32, length: u16 },
    /// Write a payload attribute. The RX buffer should contain a one byte
    /// index, 8 bytes of key (null padded), one byte of value length, and
    /// valuelength value bytes. valuelength must be less than or equal to 55.
    /// The value may contain nulls.
    ///
    /// The attribute index must be less than 16.
    SetAttr {
        index: u8,
        key: &'a [u8],
        value: &'a [u8],
    },
    /// Get a payload attribute. The RX buffer should contain a 1 byte index.
    /// The result is 8 bytes of key, 1 byte of value length, and 55 bytes of
    /// potential value. You must discard 55-valuelength bytes from the end
    /// yourself.
    GetAttr { index: u8 },
    /// Get the CRC of a range of internal flash. The RX buffer should contain
    /// a four byte address and a four byte range. The result will be a four
    /// byte crc32.
    CrcIntFlash { address: u32, length: u32 },
    /// Get the CRC of a range of external flash. The RX buffer should contain
    /// a four byte address and a four byte range. The result will be a four
    /// byte crc32.
    CrcExtFlash { address: u32, length: u32 },
    /// Erase a page in external flash. The RX buffer should contain a 4 byte
    /// address pointing to the start of the 256 byte page.
    EraseExPage { address: u32 },
    /// Initialise the external flash chip. This sets the page size to 256b.
    ExtFlashInit,
    /// Go into an infinite loop with the 32khz clock present on pin PA19
    /// (GP6) this is used for clock calibration.
    ClockOut,
    /// Write the flash user pages (first 4 bytes is first page, second 4
    /// bytes is second page, little endian).
    WriteFlashUserPages { page1: u32, page2: u32 },
    /// Change the baud rate of the bootloader. The first byte is 0x01 to set
    /// a new baud rate. The next 4 bytes are the new baud rate. To allow the
    /// bootloader to verify that the new baud rate works, the host must call
    /// this command again with the first byte of 0x02 and the next 4 bytes of
    /// the new baud rate. If the next command does not match this, the
    /// bootloader will revert to the old baud rate.
    ChangeBaud { mode: BaudMode, baud: u32 },
}

/// Reponses supported by the protocol. A bootloader will encode these
/// and a flash tool will decode them.
#[derive(Debug, PartialEq)]
pub enum Response<'a> {
    Overflow, // RES_OVERFLOW
    Pong, // RES_PONG
    BadAddress, // RES_BADADDR
    InternalError, // RES_INTERROR
    BadArguments, // RES_BADARGS
    Ok, // RES_OK
    Unknown, // RES_UNKNOWN
    ExtFlashTimeout, // RES_XFTIMEOUT
    ExtFlashPageError, // RES_XFEPE ??
    CrcRxBuffer { length: u16, crc: u32 }, // RES_CRCRX
    ReadRange { data: &'a [u8] }, // RES_RRANGE
    ExReadRange { data: &'a [u8] }, // RES_XRRANGE
    GetAttr { key: &'a [u8], value: &'a [u8] }, // RES_GATTR
    CrcIntFlash { crc: u32 }, // RES_CRCIF
    CrcExtFlash { crc: u32 }, // RES_CRCXF
    Info { info: &'a [u8] }, // RES_INFO
    ChangeBaudFail, // RES_CHANGE_BAUD_FAIL
}

#[derive(Debug, PartialEq)]
pub enum Error {
    /// We got a command we didn't understand.
    UnknownCommand,
    /// We didn't like the arguments given with a command.
    BadArguments,
    /// The user didn't call `set_payload_len` yet we
    /// got a response of unbounded length.
    UnsetLength,
    /// The user called `set_payload_len` yet we
    /// got a response of bounded length.
    SetLength,
}

/// The `ComandDecoder` takes bytes and gives you `Command`s.
pub struct CommandDecoder {
    state: DecoderState,
    buffer: [u8; 520],
    count: usize,
}

/// The `ResponseDecoder` takes bytes and gives you `Responses`s.
pub struct ResponseDecoder {
    state: DecoderState,
    buffer: [u8; 520],
    count: usize,
    needed: Option<usize>,
}

/// The `CommandEncoder` takes a `Command` and gives you bytes.
pub struct CommandEncoder<'a> {
    command: &'a Command<'a>,
    count: usize,
    sent_escape: bool,
}

/// The `ResponseEncoder` takes a `Response` and gives you bytes.
pub struct ResponseEncoder<'a> {
    response: &'a Response<'a>,
    count: usize,
    sent_escape: bool,
}

#[derive(Debug, PartialEq, Clone, Copy)]
pub enum BaudMode {
    Set, // 0x01
    Verify, // 0x02
}

// ****************************************************************************
//
// Public Data
//
// ****************************************************************************

// None

// ****************************************************************************
//
// Private Types
//
// ****************************************************************************

enum DecoderState {
    Loading,
    Escape,
}

// ****************************************************************************
//
// Private Data
//
// ****************************************************************************

const ESCAPE_CHAR: u8 = 0xFC;

const CMD_PING: u8 = 0x01;
const CMD_INFO: u8 = 0x03;
const CMD_ID: u8 = 0x04;
const CMD_RESET: u8 = 0x05;
const CMD_EPAGE: u8 = 0x06;
const CMD_WPAGE: u8 = 0x07;
const CMD_XEBLOCK: u8 = 0x08;
const CMD_XWPAGE: u8 = 0x09;
const CMD_CRCRX: u8 = 0x10;
const CMD_RRANGE: u8 = 0x11;
const CMD_XRRANGE: u8 = 0x12;
const CMD_SATTR: u8 = 0x13;
const CMD_GATTR: u8 = 0x14;
const CMD_CRCIF: u8 = 0x15;
const CMD_CRCEF: u8 = 0x16;
const CMD_XEPAGE: u8 = 0x17;
const CMD_XFINIT: u8 = 0x18;
const CMD_CLKOUT: u8 = 0x19;
const CMD_WUSER: u8 = 0x20;
const CMD_CHANGE_BAUD: u8 = 0x21;

const RES_OVERFLOW: u8 = 0x10;
const RES_PONG: u8 = 0x11;
const RES_BADADDR: u8 = 0x12;
const RES_INTERROR: u8 = 0x13;
const RES_BADARGS: u8 = 0x14;
const RES_OK: u8 = 0x15;
const RES_UNKNOWN: u8 = 0x16;
const RES_XFTIMEOUT: u8 = 0x17;
const RES_XFEPE: u8 = 0x18;
const RES_CRCRX: u8 = 0x19;
const RES_RRANGE: u8 = 0x20;
const RES_XRRANGE: u8 = 0x21;
const RES_GATTR: u8 = 0x22;
const RES_CRCIF: u8 = 0x23;
const RES_CRCXF: u8 = 0x24;
const RES_INFO: u8 = 0x25;
const RES_CHANGE_BAUD_FAIL: u8 = 0x26;

const MAX_INDEX: u8 = 16;
const KEY_LEN: usize = 8;
const MAX_ATTR_LEN: usize = 55;
const INT_PAGE_SIZE: usize = 512;
const EXT_PAGE_SIZE: usize = 256;
const MAX_INFO_LEN: usize = 192;

// ****************************************************************************
//
// Public Impl/Functions/Modules
//
// ****************************************************************************

impl CommandDecoder {
    /// Create a new `CommandDecoder`.
    ///
    /// The decoder is fed bytes with the `receive` method.
    pub fn new() -> CommandDecoder {
        CommandDecoder {
            state: DecoderState::Loading,
            buffer: [0u8; 520],
            count: 0,
        }
    }

    /// Empty the RX buffer.
    pub fn reset(&mut self) {
        self.count = 0;
    }

    /// Process incoming bytes.
    ///
    /// The decoder is fed bytes with the `receive` method. If not enough
    /// bytes have been seen, this function returns `None`. Once enough bytes
    /// have been seen, it returns `Ok(Some(Command))` containing the decoded
    /// Command. It returns `Err` if it doesn't like the byte received.
    pub fn receive(&mut self, ch: u8) -> Result<Option<Command>, Error> {
        match self.state {
            DecoderState::Loading => self.handle_loading(ch),
            DecoderState::Escape => self.handle_escape(ch),
        }
    }

    fn load_char(&mut self, ch: u8) {
        if self.count < self.buffer.len() {
            self.buffer[self.count] = ch;
            self.count = self.count + 1;
        }
    }

    fn handle_loading(&mut self, ch: u8) -> Result<Option<Command>, Error> {
        if ch == ESCAPE_CHAR {
            self.state = DecoderState::Escape;
        } else {
            self.load_char(ch);
        }
        Ok(None)
    }

    fn handle_escape(&mut self, ch: u8) -> Result<Option<Command>, Error> {
        self.state = DecoderState::Loading;
        let result: Result<Option<Command>, Error> = match ch {
            ESCAPE_CHAR => {
                // Double escape means just load an escape
                self.load_char(ch);
                Ok(None)
            }
            CMD_PING => Ok(Some(Command::Ping)),
            CMD_INFO => Ok(Some(Command::Info)),
            CMD_ID => Ok(Some(Command::Id)),
            CMD_RESET => Ok(Some(Command::Reset)),
            CMD_EPAGE => {
                let num_expected_bytes: usize = 4;
                if self.count == num_expected_bytes {
                    let address = LittleEndian::read_u32(&self.buffer[0..4]);
                    Ok(Some(Command::ErasePage { address }))
                } else {
                    Err(Error::BadArguments)
                }
            }
            CMD_WPAGE => {
                let num_expected_bytes: usize = INT_PAGE_SIZE + 4;
                if self.count == num_expected_bytes {
                    let payload = &self.buffer[0..num_expected_bytes];
                    let address = LittleEndian::read_u32(&payload[0..4]);
                    Ok(Some(Command::WritePage {
                        address,
                        data: &payload[4..num_expected_bytes],
                    }))
                } else {
                    Err(Error::BadArguments)
                }
            }
            CMD_XEBLOCK => {
                let num_expected_bytes: usize = 4;
                if self.count == num_expected_bytes {
                    let address = LittleEndian::read_u32(&self.buffer[0..4]);
                    Ok(Some(Command::EraseExBlock { address }))
                } else {
                    Err(Error::BadArguments)
                }
            }
            CMD_XWPAGE => {
                let num_expected_bytes: usize = EXT_PAGE_SIZE + 4;
                if self.count == num_expected_bytes {
                    let payload = &self.buffer[0..num_expected_bytes];
                    let address = LittleEndian::read_u32(&payload[0..4]);
                    Ok(Some(Command::WriteExPage {
                        address,
                        data: &payload[4..num_expected_bytes],
                    }))
                } else {
                    Err(Error::BadArguments)
                }
            }
            CMD_CRCRX => Ok(Some(Command::CrcRxBuffer)),
            CMD_RRANGE => {
                let num_expected_bytes: usize = 6;
                if self.count == num_expected_bytes {
                    let address = LittleEndian::read_u32(&self.buffer[0..4]);
                    let length = LittleEndian::read_u16(&self.buffer[4..6]);
                    Ok(Some(Command::ReadRange { address, length }))
                } else {
                    Err(Error::BadArguments)
                }
            }
            CMD_XRRANGE => {
                let num_expected_bytes: usize = 6;
                if self.count == num_expected_bytes {
                    let address = LittleEndian::read_u32(&self.buffer[0..4]);
                    let length = LittleEndian::read_u16(&self.buffer[4..6]);
                    Ok(Some(Command::ExReadRange { address, length }))
                } else {
                    Err(Error::BadArguments)
                }
            }
            CMD_SATTR => {
                let num_expected_bytes: usize = 10;
                if self.count >= num_expected_bytes {
                    let index = self.buffer[0];
                    let key = &self.buffer[1..9];
                    let length = self.buffer[9] as usize;
                    if self.count > (num_expected_bytes + length) {
                        let value = &self.buffer[10..10 + length];
                        Ok(Some(Command::SetAttr { index, key, value }))
                    } else {
                        Err(Error::BadArguments)
                    }
                } else {
                    Err(Error::BadArguments)
                }
            }
            CMD_GATTR => {
                let num_expected_bytes: usize = 1;
                if self.count == num_expected_bytes {
                    let index = self.buffer[0];
                    Ok(Some(Command::GetAttr { index }))
                } else {
                    Err(Error::BadArguments)
                }
            }
            CMD_CRCIF => {
                let num_expected_bytes: usize = 8;
                if self.count == num_expected_bytes {
                    let address = LittleEndian::read_u32(&self.buffer[0..4]);
                    let length = LittleEndian::read_u32(&self.buffer[4..8]);
                    Ok(Some(Command::CrcIntFlash { address, length }))
                } else {
                    Err(Error::BadArguments)
                }
            }
            CMD_CRCEF => {
                let num_expected_bytes: usize = 8;
                if self.count == num_expected_bytes {
                    let address = LittleEndian::read_u32(&self.buffer[0..4]);
                    let length = LittleEndian::read_u32(&self.buffer[4..8]);
                    Ok(Some(Command::CrcExtFlash { address, length }))
                } else {
                    Err(Error::BadArguments)
                }
            }
            CMD_XEPAGE => {
                let num_expected_bytes: usize = 4;
                if self.count == num_expected_bytes {
                    let address = LittleEndian::read_u32(&self.buffer[0..4]);
                    Ok(Some(Command::EraseExPage { address }))
                } else {
                    Err(Error::BadArguments)
                }
            }
            CMD_XFINIT => Ok(Some(Command::ExtFlashInit)),
            CMD_CLKOUT => Ok(Some(Command::ClockOut)),
            CMD_WUSER => {
                let num_expected_bytes: usize = 8;
                if self.count == num_expected_bytes {
                    let page1 = LittleEndian::read_u32(&self.buffer[0..4]);
                    let page2 = LittleEndian::read_u32(&self.buffer[4..8]);
                    Ok(Some(Command::WriteFlashUserPages { page1, page2 }))
                } else {
                    Err(Error::BadArguments)
                }
            }
            CMD_CHANGE_BAUD => {
                let num_expected_bytes: usize = 5;
                if self.count == num_expected_bytes {
                    let mode = self.buffer[0];
                    let baud = LittleEndian::read_u32(&self.buffer[1..5]);
                    match mode {
                        0x01 => Ok(Some(Command::ChangeBaud {
                            mode: BaudMode::Set,
                            baud,
                        })),
                        0x02 => Ok(Some(Command::ChangeBaud {
                            mode: BaudMode::Verify,
                            baud,
                        })),
                        _ => Err(Error::BadArguments),

                    }
                } else {
                    Err(Error::BadArguments)
                }
            }
            _ => Ok(None),
        };
        // A command or error signifies the end of the buffer
        if let Ok(Some(_)) = result {
            self.count = 0;
        } else if let Err(_) = result {
            self.count = 0;
        }
        result
    }
}

impl ResponseDecoder {
    /// Create a new `ResponseDecoder`.
    ///
    /// The decoder is fed bytes with the `receive` method.
    pub fn new() -> ResponseDecoder {
        ResponseDecoder {
            state: DecoderState::Loading,
            buffer: [0u8; 520],
            count: 0,
            needed: None,
        }
    }

    /// Empty the RX buffer.
    pub fn reset(&mut self) {
        self.count = 0;
    }

    /// Process incoming bytes.
    ///
    /// The decoder is fed bytes with the `receive` method. If not enough
    /// bytes have been seen, this function returns `None`. Once enough bytes
    /// have been seen, it returns `Some(Response)` containing the
    /// decoded Response.
    pub fn receive(&mut self, ch: u8) -> Result<Option<Response>, Error> {
        match self.state {
            DecoderState::Loading => self.handle_loading(ch),
            DecoderState::Escape => self.handle_escape(ch),
        }
    }

    /// Set the expected length of an unbounded message. This
    /// depends entirely on the last command you sent.
    pub fn set_payload_len(&mut self, length: usize) -> Result<(), Error> {
        match self.needed {
            Some(_) => Err(Error::SetLength),
            None => {
                self.needed = Some(length + 1);
                Ok(())
            }
        }
    }

    fn load_char(&mut self, ch: u8) -> Result<Option<Response>, Error> {
        if self.count < self.buffer.len() {
            self.buffer[self.count] = ch;
            self.count = self.count + 1;
        }
        if self.needed == Some(self.count) {
            let result = match self.buffer[0] {
                RES_CRCRX => {
                    let length = LittleEndian::read_u16(&self.buffer[1..3]);
                    let crc = LittleEndian::read_u32(&self.buffer[3..7]);
                    Ok(Some(Response::CrcRxBuffer { length, crc }))
                }
                RES_RRANGE => {
                    let data = &self.buffer[1..self.count];
                    Ok(Some(Response::ReadRange { data }))
                }
                RES_XRRANGE => {
                    let data = &self.buffer[1..self.count];
                    Ok(Some(Response::ExReadRange { data }))
                }
                RES_GATTR => {
                    let key = &self.buffer[1..9];
                    let length = self.buffer[9] as usize;
                    if (9 + length) <= self.count {
                        let value = &self.buffer[10..(10 + length)];
                        Ok(Some(Response::GetAttr { key, value }))
                    } else {
                        Err(Error::BadArguments)
                    }
                }
                RES_CRCIF => {
                    let crc = LittleEndian::read_u32(&self.buffer[1..5]);
                    Ok(Some(Response::CrcIntFlash { crc }))
                }
                RES_CRCXF => {
                    let crc = LittleEndian::read_u32(&self.buffer[1..5]);
                    Ok(Some(Response::CrcExtFlash { crc }))
                }
                RES_INFO => {
                    let info = &self.buffer[1..self.count];
                    Ok(Some(Response::Info { info }))
                }
                _ => Err(Error::UnknownCommand),
            };
            self.needed = None;
            self.count = 0;
            result
        } else {
            Ok(None)
        }
    }

    fn handle_loading(&mut self, ch: u8) -> Result<Option<Response>, Error> {
        if ch == ESCAPE_CHAR {
            self.state = DecoderState::Escape;
            Ok(None)
        } else {
            self.load_char(ch)
        }
    }

    fn handle_escape(&mut self, ch: u8) -> Result<Option<Response>, Error> {
        self.state = DecoderState::Loading;
        match ch {
            ESCAPE_CHAR => {
                // Double escape means just load an escape
                self.load_char(ch)
            }
            RES_PONG => {
                self.count = 0;
                self.needed = None;
                Ok(Some(Response::Pong))
            }
            RES_OVERFLOW => {
                self.count = 0;
                self.needed = None;
                Ok(Some(Response::Overflow))
            }
            RES_BADADDR => {
                self.count = 0;
                self.needed = None;
                Ok(Some(Response::BadAddress))
            }
            RES_INTERROR => {
                self.count = 0;
                self.needed = None;
                Ok(Some(Response::InternalError))
            }
            RES_BADARGS => {
                self.count = 0;
                self.needed = None;
                Ok(Some(Response::BadArguments))
            }
            RES_OK => {
                self.count = 0;
                self.needed = None;
                Ok(Some(Response::Ok))
            }
            RES_UNKNOWN => {
                self.count = 0;
                self.needed = None;
                Ok(Some(Response::Unknown))
            }
            RES_XFTIMEOUT => {
                self.count = 0;
                self.needed = None;
                Ok(Some(Response::ExtFlashTimeout))
            }
            RES_XFEPE => {
                self.count = 0;
                self.needed = None;
                Ok(Some(Response::ExtFlashPageError))
            }
            RES_CHANGE_BAUD_FAIL => {
                self.count = 0;
                self.needed = None;
                Ok(Some(Response::ChangeBaudFail))
            }
            RES_CRCRX => {
                self.set_payload_len(6)?;
                self.load_char(ch)?;
                Ok(None)
            }
            RES_RRANGE => {
                if self.needed.is_none() {
                    Err(Error::UnsetLength)
                } else {
                    self.load_char(ch)?;
                    Ok(None)
                }
            }
            RES_XRRANGE => {
                if self.needed.is_none() {
                    Err(Error::UnsetLength)
                } else {
                    self.load_char(ch)?;
                    Ok(None)
                }
            }
            RES_GATTR => {
                self.set_payload_len(1 + 8 + 55)?;
                self.load_char(ch)?;
                Ok(None)
            }
            RES_CRCIF => {
                self.set_payload_len(4)?;
                self.load_char(ch)?;
                Ok(None)
            }
            RES_CRCXF => {
                self.set_payload_len(4)?;
                self.load_char(ch)?;
                Ok(None)
            }
            RES_INFO => {
                self.set_payload_len(8)?;
                self.load_char(ch)?;
                Ok(None)
            }
            _ => Ok(None),
        }
    }
}

impl<'a> CommandEncoder<'a> {
    /// Create a new `CommandEncoder`.
    ///
    /// The encoder takes a reference to a `Command` to encode. The `next` method
    /// will then supply the encoded bytes one at a time.
    pub fn new(command: &'a Command) -> Result<CommandEncoder<'a>, Error> {
        // We have to accept slices rather than arrays, so bounds check them
        // all now to save surprises later.
        match command {
            &Command::WritePage { address: _, data } => {
                if data.len() != INT_PAGE_SIZE {
                    return Err(Error::BadArguments);
                }
            }
            &Command::WriteExPage { address: _, data } => {
                if data.len() != EXT_PAGE_SIZE {
                    return Err(Error::BadArguments);
                }
            }
            &Command::SetAttr { index, key, value } => {
                if index > MAX_INDEX {
                    return Err(Error::BadArguments);
                }
                if key.len() != KEY_LEN {
                    return Err(Error::BadArguments);
                }
                if value.len() > MAX_ATTR_LEN {
                    return Err(Error::BadArguments);
                }
            }
            _ => {}
        };
        Ok(CommandEncoder {
            command: command,
            count: 0,
            sent_escape: false,
        })
    }

    fn render_byte(&mut self, byte: u8) -> (usize, Option<u8>) {
        if byte == ESCAPE_CHAR {
            if self.sent_escape {
                self.sent_escape = false;
                (1, Some(ESCAPE_CHAR))
            } else {
                self.sent_escape = true;
                (0, Some(ESCAPE_CHAR))
            }
        } else {
            self.sent_escape = false;
            (1, Some(byte))
        }
    }

    fn render_u16(&mut self, idx: usize, value: u16) -> (usize, Option<u8>) {
        match idx {
            0 => self.render_byte(value as u8),
            1 => self.render_byte((value >> 8) as u8),
            _ => (0, None),
        }
    }

    fn render_u32(&mut self, idx: usize, value: u32) -> (usize, Option<u8>) {
        match idx {
            0 => self.render_byte(value as u8),
            1 => self.render_byte((value >> 8) as u8),
            2 => self.render_byte((value >> 16) as u8),
            3 => self.render_byte((value >> 24) as u8),
            _ => (0, None),
        }
    }

    fn render_buffer(&mut self, idx: usize, page_size: usize, data: &[u8]) -> (usize, Option<u8>) {
        if (idx < data.len()) && (idx < page_size) {
            self.render_byte(data[idx])
        } else if idx < page_size {
            self.render_byte(0xFF) // pad short data with 0xFFs
        } else {
            (0, None)
        }
    }

    fn render_basic_cmd(&mut self, count: usize, cmd: u8) -> (usize, Option<u8>) {
        match count {
            0 => (1, Some(ESCAPE_CHAR)), // Escape
            1 => (1, Some(cmd)), // Command
            _ => (0, None),
        }
    }

    fn render_erasepage_cmd(&mut self, address: u32) -> (usize, Option<u8>) {
        let count = self.count;
        match count {
            0...3 => self.render_u32(count, address),
            _ => self.render_basic_cmd(count - 4, CMD_EPAGE),
        }
    }

    fn render_writepage_cmd(&mut self, address: u32, data: &[u8]) -> (usize, Option<u8>) {
        let count = self.count;
        match count {
            0...3 => self.render_u32(count, address),
            4...515 => self.render_buffer(count - 4, INT_PAGE_SIZE, data),
            _ => self.render_basic_cmd(count - 516, CMD_WPAGE),
        }
    }

    fn render_eraseexblock(&mut self, address: u32) -> (usize, Option<u8>) {
        let count = self.count;
        match count {
            0...3 => self.render_u32(count, address),
            _ => self.render_basic_cmd(count - 4, CMD_XEBLOCK),
        }
    }

    fn render_writeexpage(&mut self, address: u32, data: &[u8]) -> (usize, Option<u8>) {
        let count = self.count;
        match count {
            0...3 => self.render_u32(count, address),
            4...259 => self.render_buffer(count - 4, EXT_PAGE_SIZE, data),
            _ => self.render_basic_cmd(count - (EXT_PAGE_SIZE + 4), CMD_XWPAGE),
        }
    }

    fn render_readrange(&mut self, address: u32, length: u16) -> (usize, Option<u8>) {
        let count = self.count;
        match count {
            0...3 => self.render_u32(count, address),
            4...5 => self.render_u16(count - 4, length),
            _ => self.render_basic_cmd(count - 6, CMD_RRANGE),
        }
    }

    fn render_exreadrange(&mut self, address: u32, length: u16) -> (usize, Option<u8>) {
        let count = self.count;
        match count {
            0...3 => self.render_u32(count, address),
            4...5 => self.render_u16(count - 4, length),
            _ => self.render_basic_cmd(count - 6, CMD_XRRANGE),
        }
    }

    fn render_setattr(&mut self, index: u8, key: &[u8], value: &[u8]) -> (usize, Option<u8>) {
        let count = self.count;
        let max_len = if value.len() > MAX_ATTR_LEN {
            MAX_ATTR_LEN
        } else {
            value.len()
        };
        match count {
            0 => self.render_byte(index),
            1...9 => self.render_buffer(count - 1, KEY_LEN, key),
            10 => self.render_byte(max_len as u8),
            x if (max_len > 0) && (x < max_len + 11) => {
                self.render_buffer(count - 11, max_len, value)
            }
            _ => self.render_basic_cmd(count - (11 + max_len), CMD_SATTR),
        }
    }

    fn render_getattr(&mut self, index: u8) -> (usize, Option<u8>) {
        let count = self.count;
        match count {
            0 => self.render_byte(index),
            _ => self.render_basic_cmd(count - 1, CMD_GATTR),
        }
    }

    fn render_crcintflash(&mut self, address: u32, length: u32) -> (usize, Option<u8>) {
        let count = self.count;
        match count {
            0...3 => self.render_u32(count, address),
            4...7 => self.render_u32(count - 4, length),
            _ => self.render_basic_cmd(count - 8, CMD_CRCIF),
        }
    }

    fn render_crcextflash(&mut self, address: u32, length: u32) -> (usize, Option<u8>) {
        let count = self.count;
        match count {
            0...3 => self.render_u32(count, address),
            4...7 => self.render_u32(count - 4, length),
            _ => self.render_basic_cmd(count - 8, CMD_CRCEF),
        }
    }

    fn render_eraseexpage(&mut self, address: u32) -> (usize, Option<u8>) {
        let count = self.count;
        match count {
            0...3 => self.render_u32(count, address),
            _ => self.render_basic_cmd(count - 4, CMD_XEPAGE),
        }
    }

    fn render_writeflashuserpages(&mut self, page1: u32, page2: u32) -> (usize, Option<u8>) {
        let count = self.count;
        match count {
            0...3 => self.render_u32(count, page1),
            4...7 => self.render_u32(count - 4, page2),
            _ => self.render_basic_cmd(count - 8, CMD_WUSER),
        }
    }

    fn render_changebaud(&mut self, mode: BaudMode, baud: u32) -> (usize, Option<u8>) {
        let count = self.count;
        match count {
            0 => {
                self.render_byte(match mode {
                    BaudMode::Set => 0x01,
                    BaudMode::Verify => 0x02,
                })
            }
            1...3 => self.render_u32(count - 1, baud),
            _ => self.render_basic_cmd(count - 8, CMD_WUSER),
        }
    }
}

impl<'a> Iterator for CommandEncoder<'a> {
    type Item = u8;

    /// Supply the next encoded byte. Once all the bytes have been emitted, it
    /// returns `None` forevermore.
    fn next(&mut self) -> Option<u8> {
        let count = self.count;
        let (inc, result) = match self.command {
            &Command::Ping => self.render_basic_cmd(count, CMD_PING),
            &Command::Info => self.render_basic_cmd(count, CMD_INFO),
            &Command::Id => self.render_basic_cmd(count, CMD_ID),
            &Command::Reset => self.render_basic_cmd(count, CMD_RESET),
            &Command::ErasePage { address } => self.render_erasepage_cmd(address),
            &Command::WritePage { address, data } => self.render_writepage_cmd(address, data),
            &Command::EraseExBlock { address } => self.render_eraseexblock(address),
            &Command::WriteExPage { address, data } => self.render_writeexpage(address, data),
            &Command::CrcRxBuffer => self.render_basic_cmd(count, CMD_CRCRX),
            &Command::ReadRange { address, length } => self.render_readrange(address, length),
            &Command::ExReadRange { address, length } => self.render_exreadrange(address, length),
            &Command::SetAttr { index, key, value } => self.render_setattr(index, key, value),
            &Command::GetAttr { index } => self.render_getattr(index),
            &Command::CrcIntFlash { address, length } => self.render_crcintflash(address, length),
            &Command::CrcExtFlash { address, length } => self.render_crcextflash(address, length),
            &Command::EraseExPage { address } => self.render_eraseexpage(address),
            &Command::ExtFlashInit => self.render_basic_cmd(count, CMD_XFINIT),
            &Command::ClockOut => self.render_basic_cmd(count, CMD_CLKOUT),
            &Command::WriteFlashUserPages { page1, page2 } => {
                self.render_writeflashuserpages(page1, page2)
            }
            &Command::ChangeBaud { mode, baud } => self.render_changebaud(mode, baud),
        };
        self.count = self.count + inc;
        result
    }
}

impl<'a> ResponseEncoder<'a> {
    /// Create a new `ResponseEncoder`.
    ///
    /// The encoder takes a reference to a `Command` to encode. The `next` method
    /// will then supply the encoded bytes one at a time.
    pub fn new(response: &'a Response) -> Result<ResponseEncoder<'a>, Error> {
        match response {
            &Response::GetAttr { key, value } => {
                if key.len() != KEY_LEN {
                    return Err(Error::BadArguments);
                }
                if value.len() > MAX_ATTR_LEN {
                    return Err(Error::BadArguments);
                }
            }
            &Response::Info { info } => {
                if info.len() > MAX_INFO_LEN {
                    return Err(Error::BadArguments);
                }
            }
            _ => {}
        }
        Ok(ResponseEncoder {
            response: response,
            count: 0,
            sent_escape: false,
        })
    }

    fn render_byte(&mut self, byte: u8) -> (usize, Option<u8>) {
        if byte == ESCAPE_CHAR {
            if self.sent_escape {
                self.sent_escape = false;
                (1, Some(ESCAPE_CHAR))
            } else {
                self.sent_escape = true;
                (0, Some(ESCAPE_CHAR))
            }
        } else {
            (1, Some(byte))
        }
    }

    fn render_crc_rx_buffer(&mut self, length: u16, crc: u32) -> (usize, Option<u8>) {
        let count = self.count;
        match count {
            0...1 => self.render_header(count, RES_CRCRX),
            2...3 => self.render_u16(count - 2, length),
            4...7 => self.render_u32(count - 4, crc),
            _ => (0, None),
        }
    }

    fn render_read_range(&mut self, data: &[u8]) -> (usize, Option<u8>) {
        let count = self.count;
        match count {
            0...1 => self.render_header(count, RES_RRANGE),
            x if x < data.len() + 2 => self.render_byte(data[x - 2]),
            _ => (0, None),
        }
    }

    fn render_ex_read_range(&mut self, data: &[u8]) -> (usize, Option<u8>) {
        let count = self.count;
        match count {
            0...1 => self.render_header(count, RES_XRRANGE),
            x if x - 2 < data.len() => self.render_byte(data[x - 2]),
            _ => (0, None),
        }
    }

    fn render_get_attr(&mut self, key: &[u8], value: &[u8]) -> (usize, Option<u8>) {
        let count = self.count;
        match count {
            0...1 => self.render_header(count, RES_GATTR),
            2...9 => self.render_buffer(count - 2, 8, key),
            10 => self.render_byte(value.len() as u8),
            _ => self.render_buffer(count - 11, MAX_ATTR_LEN, value),
        }
    }

    fn render_crc_int_flash(&mut self, crc: u32) -> (usize, Option<u8>) {
        let count = self.count;
        match count {
            0...1 => self.render_header(count, RES_CRCIF),
            _ => self.render_u32(count - 2, crc),
        }
    }

    fn render_crc_ex_flash(&mut self, crc: u32) -> (usize, Option<u8>) {
        let count = self.count;
        match count {
            0...1 => self.render_header(count, RES_CRCXF),
            _ => self.render_u32(count - 2, crc),
        }
    }

    fn render_info(&mut self, info: &[u8]) -> (usize, Option<u8>) {
        let count = self.count;
        match count {
            0...1 => self.render_header(count, RES_INFO),
            _ => self.render_buffer(count - 2, info.len(), info),
        }
    }

    fn render_u16(&mut self, idx: usize, value: u16) -> (usize, Option<u8>) {
        match idx {
            0 => self.render_byte(value as u8),
            1 => self.render_byte((value >> 8) as u8),
            _ => (0, None),
        }
    }

    fn render_u32(&mut self, idx: usize, value: u32) -> (usize, Option<u8>) {
        match idx {
            0 => self.render_byte(value as u8),
            1 => self.render_byte((value >> 8) as u8),
            2 => self.render_byte((value >> 16) as u8),
            3 => self.render_byte((value >> 24) as u8),
            _ => (0, None),
        }
    }

    fn render_buffer(&mut self, idx: usize, page_size: usize, data: &[u8]) -> (usize, Option<u8>) {
        if (idx < data.len()) && (idx < page_size) {
            self.render_byte(data[idx])
        } else if idx < page_size {
            self.render_byte(0xFF) // pad short data with 0xFFs
        } else {
            (0, None)
        }
    }

    fn render_header(&mut self, count: usize, cmd: u8) -> (usize, Option<u8>) {
        match count {
            0 => (1, Some(ESCAPE_CHAR)), // Escape
            1 => (1, Some(cmd)), // Command
            _ => (0, None),
        }
    }
}

impl<'a> Iterator for ResponseEncoder<'a> {
    type Item = u8;

    /// Supply the next encoded byte. Once all the bytes have been emitted, it
    /// returns `None` forevermore.
    fn next(&mut self) -> Option<u8> {
        let count = self.count;
        let (inc, result) = match self.response {
            &Response::Overflow => self.render_header(count, RES_OVERFLOW),
            &Response::Pong => self.render_header(count, RES_PONG),
            &Response::BadAddress => self.render_header(count, RES_BADADDR),
            &Response::InternalError => self.render_header(count, RES_INTERROR),
            &Response::BadArguments => self.render_header(count, RES_BADARGS),
            &Response::Ok => self.render_header(count, RES_OK),
            &Response::Unknown => self.render_header(count, RES_UNKNOWN),
            &Response::ExtFlashTimeout => self.render_header(count, RES_XFTIMEOUT),
            &Response::ExtFlashPageError => self.render_header(count, RES_XFEPE),
            &Response::CrcRxBuffer { length, crc } => self.render_crc_rx_buffer(length, crc),
            &Response::ReadRange { data } => self.render_read_range(data),
            &Response::ExReadRange { data } => self.render_ex_read_range(data),
            &Response::GetAttr { key, value } => self.render_get_attr(key, value),
            &Response::CrcIntFlash { crc } => self.render_crc_int_flash(crc),
            &Response::CrcExtFlash { crc } => self.render_crc_ex_flash(crc),
            &Response::Info { info } => self.render_info(info),
            &Response::ChangeBaudFail => self.render_header(count, RES_CHANGE_BAUD_FAIL),
        };
        self.count = self.count + inc;
        result
    }
}

// ****************************************************************************
//
// Private Impl/Functions/Modules
//
// ****************************************************************************

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_cmd_ping_decode() {
        let mut p = CommandDecoder::new();
        assert_eq!(p.receive(ESCAPE_CHAR), Ok(None));
        match p.receive(CMD_PING) {
            Ok(Some(Command::Ping)) => {}
            e => panic!("Did not expect: {:?}", e),
        }
    }

    #[test]
    fn check_cmd_ping_encode() {
        let cmd = Command::Ping;
        let mut e = CommandEncoder::new(&cmd).unwrap();
        assert_eq!(e.next(), Some(ESCAPE_CHAR));
        assert_eq!(e.next(), Some(CMD_PING));
        assert_eq!(e.next(), None);
        assert_eq!(e.next(), None);
    }

    #[test]
    fn check_cmd_info_decode() {
        let mut p = CommandDecoder::new();
        assert_eq!(p.receive(ESCAPE_CHAR), Ok(None));
        match p.receive(CMD_INFO) {
            Ok(Some(Command::Info)) => {}
            e => panic!("Did not expect: {:?}", e),
        }
    }

    #[test]
    fn check_cmd_info_encode() {
        let cmd = Command::Info;
        let mut e = CommandEncoder::new(&cmd).unwrap();
        assert_eq!(e.next(), Some(ESCAPE_CHAR));
        assert_eq!(e.next(), Some(CMD_INFO));
        assert_eq!(e.next(), None);
        assert_eq!(e.next(), None);
    }

    #[test]
    fn check_cmd_id_decode() {
        let mut p = CommandDecoder::new();
        assert_eq!(p.receive(ESCAPE_CHAR), Ok(None));
        match p.receive(CMD_ID) {
            Ok(Some(Command::Id)) => {}
            e => panic!("Did not expect: {:?}", e),
        }
    }

    #[test]
    fn check_cmd_id_encode() {
        let cmd = Command::Id;
        let mut e = CommandEncoder::new(&cmd).unwrap();
        assert_eq!(e.next(), Some(ESCAPE_CHAR));
        assert_eq!(e.next(), Some(CMD_ID));
        assert_eq!(e.next(), None);
        assert_eq!(e.next(), None);
    }

    #[test]
    fn check_cmd_reset_decode() {
        let mut p = CommandDecoder::new();
        assert_eq!(p.receive(ESCAPE_CHAR), Ok(None));
        match p.receive(CMD_RESET) {
            Ok(Some(Command::Reset)) => {}
            e => panic!("Did not expect: {:?}", e),
        }
    }

    #[test]
    fn check_cmd_reset_encode() {
        let cmd = Command::Reset;
        let mut e = CommandEncoder::new(&cmd).unwrap();
        assert_eq!(e.next(), Some(ESCAPE_CHAR));
        assert_eq!(e.next(), Some(CMD_RESET));
        assert_eq!(e.next(), None);
        assert_eq!(e.next(), None);
    }

    #[test]
    fn check_cmd_erase_page_decode() {
        let mut p = CommandDecoder::new();
        assert_eq!(p.receive(0xEF), Ok(None));
        assert_eq!(p.receive(0xBE), Ok(None));
        assert_eq!(p.receive(0xAD), Ok(None));
        assert_eq!(p.receive(0xDE), Ok(None));
        assert_eq!(p.receive(ESCAPE_CHAR), Ok(None)); // Escape
        match p.receive(CMD_EPAGE) {
            Ok(Some(Command::ErasePage { address })) => {
                assert_eq!(address, 0xDEADBEEF);
            }
            e => panic!("Did not expect: {:?}", e),
        }
    }

    #[test]
    fn check_cmd_erase_page_encode() {
        let cmd = Command::ErasePage { address: 0xDEADBEEF };
        let mut e = CommandEncoder::new(&cmd).unwrap();
        // 4 byte address, little-endian
        assert_eq!(e.next(), Some(0xEF));
        assert_eq!(e.next(), Some(0xBE));
        assert_eq!(e.next(), Some(0xAD));
        assert_eq!(e.next(), Some(0xDE));
        assert_eq!(e.next(), Some(ESCAPE_CHAR));
        assert_eq!(e.next(), Some(CMD_EPAGE));
        assert_eq!(e.next(), None);
        assert_eq!(e.next(), None);
    }

    #[test]
    fn check_cmd_write_page_decode() {
        let mut p = CommandDecoder::new();
        assert_eq!(p.receive(0xEF), Ok(None));
        assert_eq!(p.receive(0xBE), Ok(None));
        assert_eq!(p.receive(0xAD), Ok(None));
        assert_eq!(p.receive(0xDE), Ok(None));
        for i in 0..INT_PAGE_SIZE {
            let datum = i as u8;
            assert_eq!(p.receive(datum), Ok(None));
            if datum == ESCAPE_CHAR {
                assert_eq!(p.receive(datum), Ok(None));
            }
        }
        assert_eq!(p.receive(ESCAPE_CHAR), Ok(None)); // Escape
        match p.receive(CMD_WPAGE) {
            Ok(Some(Command::WritePage {
                        address,
                        data: ref page,
                    })) => {
                assert_eq!(address, 0xDEADBEEF);
                assert_eq!(page.len(), INT_PAGE_SIZE);
                for i in 0..INT_PAGE_SIZE {
                    let datum = i as u8;
                    assert_eq!(datum, page[i as usize]);
                }
            }
            e => panic!("Did not expect: {:?}", e),
        }
    }

    #[test]
    fn check_cmd_write_page_encode() {
        let mut buffer = [0xBBu8; INT_PAGE_SIZE];
        buffer[0] = 0xAA;
        buffer[INT_PAGE_SIZE - 1] = 0xCC;
        let cmd = Command::WritePage {
            address: 0xDEADBEEF,
            data: &buffer,
        };
        let mut e = CommandEncoder::new(&cmd).unwrap();
        // 4 byte address, little-endian
        assert_eq!(e.next(), Some(0xEF));
        assert_eq!(e.next(), Some(0xBE));
        assert_eq!(e.next(), Some(0xAD));
        assert_eq!(e.next(), Some(0xDE));
        // first byte of data
        assert_eq!(e.next(), Some(0xAA));
        for _ in 1..(INT_PAGE_SIZE - 1) {
            assert_eq!(e.next(), Some(0xBB));
        }
        // last byte of data
        assert_eq!(e.next(), Some(0xCC));
        assert_eq!(e.next(), Some(ESCAPE_CHAR));
        assert_eq!(e.next(), Some(CMD_WPAGE));
        assert_eq!(e.next(), None);
        assert_eq!(e.next(), None);
    }

    #[test]
    fn check_cmd_erase_block_decode() {
        let mut p = CommandDecoder::new();
        assert_eq!(p.receive(0xEF), Ok(None));
        assert_eq!(p.receive(0xBE), Ok(None));
        assert_eq!(p.receive(0xAD), Ok(None));
        assert_eq!(p.receive(0xDE), Ok(None));
        assert_eq!(p.receive(ESCAPE_CHAR), Ok(None)); // Escape
        match p.receive(CMD_XEBLOCK) {
            Ok(Some(Command::EraseExBlock { address })) => {
                assert_eq!(address, 0xDEADBEEF);
            }
            e => panic!("Did not expect: {:?}", e),
        }
    }

    #[test]
    fn check_cmd_erase_block_encode() {
        let cmd = Command::EraseExBlock { address: 0xDEADBEEF };
        let mut e = CommandEncoder::new(&cmd).unwrap();
        // 4 byte address, little-endian
        assert_eq!(e.next(), Some(0xEF));
        assert_eq!(e.next(), Some(0xBE));
        assert_eq!(e.next(), Some(0xAD));
        assert_eq!(e.next(), Some(0xDE));
        assert_eq!(e.next(), Some(ESCAPE_CHAR));
        assert_eq!(e.next(), Some(CMD_XEBLOCK));
        assert_eq!(e.next(), None);
        assert_eq!(e.next(), None);
    }

    #[test]
    fn check_cmd_write_ex_page_decode() {
        let mut p = CommandDecoder::new();
        assert_eq!(p.receive(0xEF), Ok(None));
        assert_eq!(p.receive(0xBE), Ok(None));
        assert_eq!(p.receive(0xAD), Ok(None));
        assert_eq!(p.receive(0xDE), Ok(None));
        for i in 0..EXT_PAGE_SIZE {
            let datum = i as u8;
            assert_eq!(p.receive(datum), Ok(None));
            if datum == ESCAPE_CHAR {
                assert_eq!(p.receive(datum), Ok(None));
            }
        }
        assert_eq!(p.receive(ESCAPE_CHAR), Ok(None)); // Escape
        match p.receive(CMD_XWPAGE) {
            Ok(Some(Command::WriteExPage {
                        address,
                        data: ref page,
                    })) => {
                assert_eq!(address, 0xDEADBEEF);
                assert_eq!(page.len(), EXT_PAGE_SIZE);
                for i in 0..EXT_PAGE_SIZE {
                    let datum = i as u8;
                    assert_eq!(datum, page[i as usize]);
                }
            }
            e => panic!("Did not expect: {:?}", e),
        }
    }

    #[test]
    fn check_cmd_write_ex_page_encode() {
        let mut buffer = [0xBBu8; EXT_PAGE_SIZE];
        buffer[0] = 0xAA;
        buffer[EXT_PAGE_SIZE - 1] = 0xCC;
        let cmd = Command::WriteExPage {
            address: 0xDEADBEEF,
            data: &buffer,
        };
        let mut e = CommandEncoder::new(&cmd).unwrap();
        // 4 byte address, little-endian
        assert_eq!(e.next(), Some(0xEF));
        assert_eq!(e.next(), Some(0xBE));
        assert_eq!(e.next(), Some(0xAD));
        assert_eq!(e.next(), Some(0xDE));
        // first byte of data
        assert_eq!(e.next(), Some(0xAA));
        for _ in 1..(EXT_PAGE_SIZE - 1) {
            assert_eq!(e.next(), Some(0xBB));
        }
        // last byte of data
        assert_eq!(e.next(), Some(0xCC));
        assert_eq!(e.next(), Some(ESCAPE_CHAR));
        assert_eq!(e.next(), Some(CMD_XWPAGE));
        assert_eq!(e.next(), None);
        assert_eq!(e.next(), None);
    }

    // Test CMD_CRCRX here
    // Test CMD_RRANGE here
    // Test CMD_XRRANGE here
    // Test CMD_SATTR here
    // Test CMD_GATTR here
    // Test CMD_CRCIF here
    // Test CMD_CRCEF here
    // Test CMD_XEPAGE here
    // Test CMD_XFINIT here
    // Test CMD_CLKOUT here
    // Test CMD_WUSER here
    // Test CMD_CHANGE_BAUD here

    // Responses

    fn check_rsp_generic(response: Response, cmd: u8) {
        let mut p = ResponseDecoder::new();
        assert_eq!(p.receive(ESCAPE_CHAR), Ok(None));
        match p.receive(cmd) {
            Ok(Some(ref x)) if x == &response => {}
            e => panic!("Did not expect: {:?}", e),
        }

        let mut e = ResponseEncoder::new(&response).unwrap();
        assert_eq!(e.next(), Some(ESCAPE_CHAR));
        assert_eq!(e.next(), Some(cmd));
        assert_eq!(e.next(), None);
        assert_eq!(e.next(), None);
    }

    #[test]
    fn check_rsp_overflow() {
        check_rsp_generic(Response::Overflow, RES_OVERFLOW);
    }

    #[test]
    fn check_rsp_pong() {
        check_rsp_generic(Response::Pong, RES_PONG);
    }

    #[test]
    fn check_rsp_badaddress() {
        check_rsp_generic(Response::BadAddress, RES_BADADDR);
    }

    #[test]
    fn check_rsp_internalerror() {
        check_rsp_generic(Response::InternalError, RES_INTERROR);
    }

    #[test]
    fn check_rsp_badarguments() {
        check_rsp_generic(Response::BadArguments, RES_BADARGS);
    }

    #[test]
    fn check_rsp_ok() {
        check_rsp_generic(Response::Ok, RES_OK);
    }

    #[test]
    fn check_rsp_unknown() {
        check_rsp_generic(Response::Unknown, RES_UNKNOWN);
    }

    #[test]
    fn check_rsp_exflashtimeout() {
        check_rsp_generic(Response::ExtFlashTimeout, RES_XFTIMEOUT);
    }

    #[test]
    fn check_rsp_exflashpageerror() {
        check_rsp_generic(Response::ExtFlashPageError, RES_XFEPE);
    }

    #[test]
    fn check_rsp_changebaudfail() {
        check_rsp_generic(Response::ChangeBaudFail, RES_CHANGE_BAUD_FAIL);
    }

    #[test]
    fn check_rsp_crc_rx() {
        let mut p = ResponseDecoder::new();
        assert_eq!(p.receive(ESCAPE_CHAR), Ok(None));
        assert_eq!(p.receive(RES_CRCRX), Ok(None));
        // Length
        assert_eq!(p.receive(0x34), Ok(None));
        assert_eq!(p.receive(0x12), Ok(None));
        // CRC
        assert_eq!(p.receive(0xEF), Ok(None));
        assert_eq!(p.receive(0xBE), Ok(None));
        assert_eq!(p.receive(0xAD), Ok(None));
        assert_eq!(
            p.receive(0xDE),
            Ok(Some(Response::CrcRxBuffer {
                length: 0x1234,
                crc: 0xDEADBEEF,
            }))
        );

        let r = Response::CrcRxBuffer {
            length: 0x1234,
            crc: 0xDEADBEEF,
        };
        let mut e = ResponseEncoder::new(&r).unwrap();
        assert_eq!(e.next(), Some(ESCAPE_CHAR));
        assert_eq!(e.next(), Some(RES_CRCRX));
        assert_eq!(e.next(), Some(0x34));
        assert_eq!(e.next(), Some(0x12));
        assert_eq!(e.next(), Some(0xEF));
        assert_eq!(e.next(), Some(0xBE));
        assert_eq!(e.next(), Some(0xAD));
        assert_eq!(e.next(), Some(0xDE));
        assert_eq!(e.next(), None);
        assert_eq!(e.next(), None);
    }

    #[test]
    fn check_rsp_rrange() {
        let mut p = ResponseDecoder::new();
        p.set_payload_len(4).unwrap();
        assert_eq!(p.receive(ESCAPE_CHAR), Ok(None));
        assert_eq!(p.receive(RES_RRANGE), Ok(None));
        // four bytes of data
        assert_eq!(p.receive(0x00), Ok(None));
        assert_eq!(p.receive(0x11), Ok(None));
        assert_eq!(p.receive(0x22), Ok(None));
        assert_eq!(
            p.receive(0x33),
            Ok(Some(
                Response::ReadRange { data: &[0x00, 0x11, 0x22, 0x33] },
            ))
        );

        let r = Response::ReadRange { data: &[0x00, 0x11, 0x22, 0x33] };
        let mut e = ResponseEncoder::new(&r).unwrap();
        assert_eq!(e.next(), Some(ESCAPE_CHAR));
        assert_eq!(e.next(), Some(RES_RRANGE));
        assert_eq!(e.next(), Some(0x00));
        assert_eq!(e.next(), Some(0x11));
        assert_eq!(e.next(), Some(0x22));
        assert_eq!(e.next(), Some(0x33));
        assert_eq!(e.next(), None);
        assert_eq!(e.next(), None);
    }

    #[test]
    fn check_rsp_xrrange() {
        let mut p = ResponseDecoder::new();
        p.set_payload_len(4).unwrap();
        assert_eq!(p.receive(ESCAPE_CHAR), Ok(None));
        assert_eq!(p.receive(RES_XRRANGE), Ok(None));
        // four bytes of data
        assert_eq!(p.receive(0x00), Ok(None));
        assert_eq!(p.receive(0x11), Ok(None));
        assert_eq!(p.receive(0x22), Ok(None));
        assert_eq!(
            p.receive(0x33),
            Ok(Some(
                Response::ExReadRange { data: &[0x00, 0x11, 0x22, 0x33] },
            ))
        );

        let r = Response::ExReadRange { data: &[0x00, 0x11, 0x22, 0x33] };
        let mut e = ResponseEncoder::new(&r).unwrap();
        assert_eq!(e.next(), Some(ESCAPE_CHAR));
        assert_eq!(e.next(), Some(RES_XRRANGE));
        assert_eq!(e.next(), Some(0x00));
        assert_eq!(e.next(), Some(0x11));
        assert_eq!(e.next(), Some(0x22));
        assert_eq!(e.next(), Some(0x33));
        assert_eq!(e.next(), None);
        assert_eq!(e.next(), None);
    }

    #[test]
    fn check_rsp_get_attr() {
        let mut p = ResponseDecoder::new();
        assert_eq!(p.receive(ESCAPE_CHAR), Ok(None));
        assert_eq!(p.receive(RES_GATTR), Ok(None));
        // eight bytes of key
        assert_eq!(p.receive(0x00), Ok(None));
        assert_eq!(p.receive(0x11), Ok(None));
        assert_eq!(p.receive(0x22), Ok(None));
        assert_eq!(p.receive(0x33), Ok(None));
        assert_eq!(p.receive(0x44), Ok(None));
        assert_eq!(p.receive(0x55), Ok(None));
        assert_eq!(p.receive(0x66), Ok(None));
        assert_eq!(p.receive(0x77), Ok(None));
        // Length byte
        assert_eq!(p.receive(0x04), Ok(None));
        // length bytes of data
        assert_eq!(p.receive(0xAA), Ok(None));
        assert_eq!(p.receive(0xBB), Ok(None));
        assert_eq!(p.receive(0xCC), Ok(None));
        assert_eq!(p.receive(0xDD), Ok(None));
        // 55 - length bytes of padding
        for _ in 4..MAX_ATTR_LEN - 1 {
            assert_eq!(p.receive(0xFF), Ok(None));
        }
        assert_eq!(
            p.receive(0xFF),
            Ok(Some(Response::GetAttr {
                key: &[0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77],
                value: &[0xAA, 0xBB, 0xCC, 0xDD],
            }))
        );

        let r = Response::GetAttr {
            key: &[0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77],
            value: &[0xAA, 0xBB, 0xCC, 0xDD],
        };
        let mut e = ResponseEncoder::new(&r).unwrap();
        assert_eq!(e.next(), Some(ESCAPE_CHAR));
        assert_eq!(e.next(), Some(RES_GATTR));
        assert_eq!(e.next(), Some(0x00));
        assert_eq!(e.next(), Some(0x11));
        assert_eq!(e.next(), Some(0x22));
        assert_eq!(e.next(), Some(0x33));
        assert_eq!(e.next(), Some(0x44));
        assert_eq!(e.next(), Some(0x55));
        assert_eq!(e.next(), Some(0x66));
        assert_eq!(e.next(), Some(0x77));
        assert_eq!(e.next(), Some(0x04));
        assert_eq!(e.next(), Some(0xAA));
        assert_eq!(e.next(), Some(0xBB));
        assert_eq!(e.next(), Some(0xCC));
        assert_eq!(e.next(), Some(0xDD));
        for _ in 4..MAX_ATTR_LEN {
            assert!(e.next().is_some());
        }
        assert_eq!(e.next(), None);
        assert_eq!(e.next(), None);
    }

    #[test]
    fn check_rsp_crc_int_flash() {
        let mut p = ResponseDecoder::new();
        assert_eq!(p.receive(ESCAPE_CHAR), Ok(None));
        assert_eq!(p.receive(RES_CRCIF), Ok(None));
        // CRC
        assert_eq!(p.receive(0xEF), Ok(None));
        assert_eq!(p.receive(0xBE), Ok(None));
        assert_eq!(p.receive(0xAD), Ok(None));
        assert_eq!(
            p.receive(0xDE),
            Ok(Some(Response::CrcIntFlash { crc: 0xDEADBEEF }))
        );

        let r = Response::CrcIntFlash { crc: 0xDEADBEEF };
        let mut e = ResponseEncoder::new(&r).unwrap();
        assert_eq!(e.next(), Some(ESCAPE_CHAR));
        assert_eq!(e.next(), Some(RES_CRCIF));
        assert_eq!(e.next(), Some(0xEF));
        assert_eq!(e.next(), Some(0xBE));
        assert_eq!(e.next(), Some(0xAD));
        assert_eq!(e.next(), Some(0xDE));
        assert_eq!(e.next(), None);
        assert_eq!(e.next(), None);
    }

    #[test]
    fn check_rsp_crc_ext_flash() {
        let mut p = ResponseDecoder::new();
        assert_eq!(p.receive(ESCAPE_CHAR), Ok(None));
        assert_eq!(p.receive(RES_CRCXF), Ok(None));
        // CRC
        assert_eq!(p.receive(0xEF), Ok(None));
        assert_eq!(p.receive(0xBE), Ok(None));
        assert_eq!(p.receive(0xAD), Ok(None));
        assert_eq!(
            p.receive(0xDE),
            Ok(Some(Response::CrcExtFlash { crc: 0xDEADBEEF }))
        );

        let r = Response::CrcExtFlash { crc: 0xDEADBEEF };
        let mut e = ResponseEncoder::new(&r).unwrap();
        assert_eq!(e.next(), Some(ESCAPE_CHAR));
        assert_eq!(e.next(), Some(RES_CRCXF));
        assert_eq!(e.next(), Some(0xEF));
        assert_eq!(e.next(), Some(0xBE));
        assert_eq!(e.next(), Some(0xAD));
        assert_eq!(e.next(), Some(0xDE));
        assert_eq!(e.next(), None);
        assert_eq!(e.next(), None);
    }

    #[test]
    fn check_rsp_info() {
        let mut p = ResponseDecoder::new();
        assert_eq!(p.receive(ESCAPE_CHAR), Ok(None));
        assert_eq!(p.receive(RES_INFO), Ok(None));
        // eight bytes of data
        assert_eq!(p.receive(0x00), Ok(None));
        assert_eq!(p.receive(0x11), Ok(None));
        assert_eq!(p.receive(0x22), Ok(None));
        assert_eq!(p.receive(0x33), Ok(None));
        assert_eq!(p.receive(0x44), Ok(None));
        assert_eq!(p.receive(0x55), Ok(None));
        assert_eq!(p.receive(0x66), Ok(None));
        assert_eq!(
            p.receive(0x77),
            Ok(Some(Response::Info {
                info: &[0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77],
            }))
        );

        // Check follow-on command
        assert_eq!(p.receive(ESCAPE_CHAR), Ok(None));
        assert_eq!(p.receive(RES_PONG), Ok(Some(Response::Pong)));

        let r = Response::Info { info: &[0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77] };
        let mut e = ResponseEncoder::new(&r).unwrap();
        assert_eq!(e.next(), Some(ESCAPE_CHAR));
        assert_eq!(e.next(), Some(RES_INFO));
        assert_eq!(e.next(), Some(0x00));
        assert_eq!(e.next(), Some(0x11));
        assert_eq!(e.next(), Some(0x22));
        assert_eq!(e.next(), Some(0x33));
        assert_eq!(e.next(), Some(0x44));
        assert_eq!(e.next(), Some(0x55));
        assert_eq!(e.next(), Some(0x66));
        assert_eq!(e.next(), Some(0x77));
        assert_eq!(e.next(), None);
        assert_eq!(e.next(), None);
    }

}

// ****************************************************************************
//
// End Of File
//
// ****************************************************************************
