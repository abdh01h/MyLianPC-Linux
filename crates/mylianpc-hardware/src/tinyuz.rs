//! TinyUZ stream encoder/decoder used by the HydroShift II ring-lighting
//! upload (command 0xFC).
//!
//! Ported from `MyLianPC.Hardware/LianLi/TinyUz.cs`, which derives from
//! `hydroshift2-rs` (MIT) and the TinyUZ format by sisong (MIT).

use std::fmt;

const MIN_LITERAL_LINE: usize = 15;
const MAX_DICT_DISTANCE: usize = 127;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TinyUzError(&'static str);

impl fmt::Display for TinyUzError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

impl std::error::Error for TinyUzError {}

pub fn compress(data: &[u8]) -> Vec<u8> {
    let mut writer = Writer::new();
    let mut pos = 0;
    while pos < data.len() {
        let (length, distance) = longest_match(data, pos);
        if length >= 3 {
            writer.push_match(length, distance);
            pos += length;
            continue;
        }
        let start = pos;
        pos += 1;
        while pos < data.len() && longest_match(data, pos).0 < 3 {
            pos += 1;
        }
        writer.push_data(&data[start..pos]);
    }
    writer.finish()
}

fn longest_match(data: &[u8], pos: usize) -> (usize, usize) {
    let mut best = (0usize, 0usize);
    for distance in 1..=pos.min(MAX_DICT_DISTANCE) {
        let mut length = 0;
        while pos + length < data.len() && data[pos + length] == data[pos + length - distance] {
            length += 1;
        }
        if length > best.0 {
            best = (length, distance);
        }
    }
    best
}

struct Writer {
    code: Vec<u8>,
    type_count: usize,
    types_index: usize,
    dict_pos_back: usize,
    max_dict_size: usize,
    have_data_back: bool,
}

impl Writer {
    fn new() -> Self {
        Self {
            code: Vec::new(),
            type_count: 0,
            types_index: 0,
            dict_pos_back: 1,
            max_dict_size: 1,
            have_data_back: false,
        }
    }

    fn type_bit(&mut self, bit: u8) {
        if self.type_count == 0 {
            self.types_index = self.code.len();
            self.code.push(0);
        }
        self.code[self.types_index] |= (bit & 1) << self.type_count;
        self.type_count = (self.type_count + 1) % 8;
    }

    fn write_len(&mut self, value: usize, pack_bits: usize) {
        let mut count = 1;
        let mut reduced = value;
        while reduced >= 1usize << (count * pack_bits) {
            reduced -= 1usize << (count * pack_bits);
            count += 1;
        }
        while count > 0 {
            count -= 1;
            for bit in 0..pack_bits {
                self.type_bit(((reduced >> (count * pack_bits + bit)) & 1) as u8);
            }
            self.type_bit(if count > 0 { 1 } else { 0 });
        }
    }

    fn dict_pos(&mut self, pos: usize) {
        let extended = pos >= 128;
        let pos = if extended { pos - 128 } else { pos };
        self.code
            .push(((pos & 0x7F) | if extended { 0x80 } else { 0 }) as u8);
        if extended {
            self.write_len(pos >> 7, 2);
        }
    }

    fn control(&mut self, control: usize) {
        self.type_bit(0);
        self.write_len(control, 1);
        if self.have_data_back {
            self.type_bit(0);
        }
        self.dict_pos(0);
    }

    fn push_data(&mut self, data: &[u8]) {
        if data.len() >= MIN_LITERAL_LINE {
            self.control(1);
            self.write_len(data.len() - MIN_LITERAL_LINE, 2);
            self.code.extend_from_slice(data);
        } else {
            for byte in data {
                self.type_bit(1);
                self.code.push(*byte);
            }
        }
        self.have_data_back = true;
    }

    fn push_match(&mut self, length: usize, distance: usize) {
        self.type_bit(0);
        self.max_dict_size = self.max_dict_size.max(distance);
        let saved = self.dict_pos_back == distance && self.have_data_back;
        self.write_len(length - 2, 1);
        if self.have_data_back {
            self.type_bit(if saved { 1 } else { 0 });
        }
        if !saved {
            self.dict_pos(distance);
        }
        self.have_data_back = false;
        self.dict_pos_back = distance;
    }

    fn finish(mut self) -> Vec<u8> {
        self.control(3);
        let mut out = Vec::with_capacity(4 + self.code.len());
        out.extend_from_slice(&(self.max_dict_size as u32).to_le_bytes());
        out.extend_from_slice(&self.code);
        out
    }
}

/// Reference decoder, used to verify every stream before it is sent to the
/// device.
pub fn decompress(code: &[u8], max_output: usize) -> Result<Vec<u8>, TinyUzError> {
    if code.len() < 5 {
        return Err(TinyUzError("TinyUZ stream is too short"));
    }
    let dict_size =
        u32::from_le_bytes([code[0], code[1], code[2], code[3]]) as usize;
    if dict_size == 0 {
        return Err(TinyUzError("invalid TinyUZ dictionary size"));
    }

    let mut reader = Reader::new(&code[4..]);
    let mut output = Vec::new();
    let mut dict_pos_back = 1usize;
    let mut have_data_back = false;

    loop {
        if reader.bit()? == 1 {
            output.push(reader.byte()?);
            have_data_back = true;
        } else {
            let mut saved_length = reader.unpack_len(1)?;
            let saved_pos;
            if have_data_back && reader.bit()? == 1 {
                saved_pos = dict_pos_back;
            } else {
                saved_pos = reader.dict_pos()?;
                if saved_pos > 2687 {
                    saved_length += 1;
                }
            }
            have_data_back = false;

            if saved_pos != 0 {
                let length = saved_length + 2;
                if saved_pos > output.len() || saved_pos > dict_size {
                    return Err(TinyUzError("invalid TinyUZ match"));
                }
                if output.len() + length > max_output {
                    return Err(TinyUzError("TinyUZ output exceeds limit"));
                }
                dict_pos_back = saved_pos;
                for _ in 0..length {
                    let byte = output[output.len() - saved_pos];
                    output.push(byte);
                }
            } else if saved_length == 1 {
                let length = reader.unpack_len(2)? + MIN_LITERAL_LINE;
                if output.len() + length > max_output {
                    return Err(TinyUzError("truncated TinyUZ literal"));
                }
                output.extend_from_slice(reader.take(length)?);
                have_data_back = true;
            } else {
                dict_pos_back = 1;
                reader.reset_types();
                if saved_length == 2 {
                    continue;
                }
                if saved_length == 3 {
                    return Ok(output);
                }
                return Err(TinyUzError("unknown TinyUZ control code"));
            }
        }

        if output.len() > max_output {
            return Err(TinyUzError("TinyUZ output exceeds limit"));
        }
    }
}

struct Reader<'a> {
    code: &'a [u8],
    pos: usize,
    type_count: usize,
    types: u8,
}

impl<'a> Reader<'a> {
    fn new(code: &'a [u8]) -> Self {
        Self {
            code,
            pos: 0,
            type_count: 0,
            types: 0,
        }
    }

    fn byte(&mut self) -> Result<u8, TinyUzError> {
        let byte = *self
            .code
            .get(self.pos)
            .ok_or(TinyUzError("truncated TinyUZ stream"))?;
        self.pos += 1;
        Ok(byte)
    }

    fn bit(&mut self) -> Result<u8, TinyUzError> {
        if self.type_count == 0 {
            self.types = self.byte()?;
            self.type_count = 8;
        }
        let bit = self.types & 1;
        self.types >>= 1;
        self.type_count -= 1;
        Ok(bit)
    }

    fn unpack_len(&mut self, pack_bits: usize) -> Result<usize, TinyUzError> {
        let mut value = 0usize;
        loop {
            let mut low = 0usize;
            for index in 0..pack_bits {
                low |= (self.bit()? as usize) << index;
            }
            let cont = self.bit()?;
            value = (value << pack_bits) + low;
            if cont == 0 {
                return Ok(value);
            }
            value += 1;
        }
    }

    fn dict_pos(&mut self) -> Result<usize, TinyUzError> {
        let value = self.byte()? as usize;
        if value >= 128 {
            Ok(((value & 0x7F) | (self.unpack_len(2)? << 7)) + 128)
        } else {
            Ok(value)
        }
    }

    fn take(&mut self, count: usize) -> Result<&'a [u8], TinyUzError> {
        if self.pos + count > self.code.len() {
            return Err(TinyUzError("truncated TinyUZ literal"));
        }
        let slice = &self.code[self.pos..self.pos + count];
        self.pos += count;
        Ok(slice)
    }

    fn reset_types(&mut self) {
        self.type_count = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_text() {
        for text in [
            b"abcabcabcabcabc---hydroshift---hydroshift".as_slice(),
            b"x".as_slice(),
            b"0123456789abcdefghijklmnopqrstuvwxyz".as_slice(),
        ] {
            let compressed = compress(text);
            let decompressed = decompress(&compressed, text.len()).expect("decodes");
            assert_eq!(decompressed, text);
        }
    }

    #[test]
    fn round_trips_a_full_rainbow_animation() {
        let mut input = vec![0u8; 48 * 72];
        for (index, byte) in input.iter_mut().enumerate() {
            *byte = if index % 7 == 0 {
                ((index * 37 + 11) % 256) as u8
            } else {
                ((index * 31) % 251) as u8
            };
        }
        let compressed = compress(&input);
        let decompressed = decompress(&compressed, input.len()).expect("decodes");
        assert_eq!(decompressed, input);
    }

    #[test]
    fn round_trips_empty_input() {
        assert!(decompress(&compress(&[]), 0).unwrap().is_empty());
    }

    #[test]
    fn rejects_a_short_stream() {
        assert!(decompress(&[0, 0, 0, 1], 10).is_err());
    }
}
