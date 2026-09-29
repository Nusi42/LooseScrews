//! Hand-written bit writer and reader.
//!
//! Bits are packed LSB-first: the first bit written ends up in bit 0 of the
//! first byte. A packet is padded with zero bits to a whole byte. Multi-bit
//! values are written least significant bit first, so a 16-bit value written
//! at a byte boundary is stored little-endian.
//!
//! The reader never panics on malformed input; it returns
//! [`DecodeError::UnexpectedEnd`] when it runs past the end.

use crate::error::DecodeError;

/// Appends bit fields to a growable byte buffer.
#[derive(Debug, Default, Clone)]
pub struct BitWriter {
    buf: Vec<u8>,
    /// Pending bits not yet flushed to `buf` (low `pending` bits are valid).
    acc: u64,
    pending: u32,
}

impl BitWriter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_capacity(bytes: usize) -> Self {
        BitWriter {
            buf: Vec::with_capacity(bytes),
            acc: 0,
            pending: 0,
        }
    }

    /// Total number of bits written so far.
    #[inline]
    pub fn bit_len(&self) -> usize {
        self.buf.len() * 8 + self.pending as usize
    }

    /// Writes the low `n` bits of `value` (`n <= 64`). Higher bits of `value`
    /// are ignored.
    pub fn write_bits(&mut self, value: u64, n: u32) {
        debug_assert!(n <= 64);
        if n == 0 {
            return;
        }
        // Split into at most 32-bit chunks so the accumulator never overflows
        // (pending < 8 after every flush, so pending + 32 < 64).
        if n > 32 {
            self.write_bits(value & 0xFFFF_FFFF, 32);
            self.write_bits(value >> 32, n - 32);
            return;
        }
        let masked = value & ((1u64 << n) - 1);
        self.acc |= masked << self.pending;
        self.pending += n;
        while self.pending >= 8 {
            self.buf.push(self.acc as u8);
            self.acc >>= 8;
            self.pending -= 8;
        }
    }

    #[inline]
    pub fn write_bool(&mut self, b: bool) {
        self.write_bits(b as u64, 1);
    }

    #[inline]
    pub fn write_u8(&mut self, v: u8) {
        self.write_bits(v as u64, 8);
    }

    #[inline]
    pub fn write_u16(&mut self, v: u16) {
        self.write_bits(v as u64, 16);
    }

    #[inline]
    pub fn write_u32(&mut self, v: u32) {
        self.write_bits(v as u64, 32);
    }

    /// Raw IEEE-754 bits, so the value round-trips exactly (used for terrain
    /// edits, which must be bit-identical on every peer).
    #[inline]
    pub fn write_f32(&mut self, v: f32) {
        self.write_u32(v.to_bits());
    }

    /// LEB128-style variable-length unsigned integer: 7 value bits plus a
    /// continuation bit per group. Small numbers take 8 bits.
    pub fn write_varint(&mut self, mut v: u64) {
        loop {
            let group = v & 0x7F;
            v >>= 7;
            let more = v != 0;
            self.write_bits(group | ((more as u64) << 7), 8);
            if !more {
                break;
            }
        }
    }

    /// Length-prefixed byte string.
    pub fn write_bytes(&mut self, bytes: &[u8]) {
        self.write_varint(bytes.len() as u64);
        for &b in bytes {
            self.write_u8(b);
        }
    }

    /// Length-prefixed UTF-8 string.
    pub fn write_str(&mut self, s: &str) {
        self.write_bytes(s.as_bytes());
    }

    /// Flushes the partial byte (zero padded) and returns the packet.
    pub fn finish(mut self) -> Vec<u8> {
        if self.pending > 0 {
            self.buf.push(self.acc as u8);
        }
        self.buf
    }
}

/// Reads bit fields written by [`BitWriter`].
#[derive(Debug, Clone)]
pub struct BitReader<'a> {
    data: &'a [u8],
    /// Absolute bit position.
    pos: usize,
}

impl<'a> BitReader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        BitReader { data, pos: 0 }
    }

    /// Bits left, including the zero padding of the last byte.
    #[inline]
    pub fn remaining_bits(&self) -> usize {
        self.data.len() * 8 - self.pos
    }

    /// Reads `n <= 64` bits.
    pub fn read_bits(&mut self, n: u32) -> Result<u64, DecodeError> {
        debug_assert!(n <= 64);
        if n as usize > self.remaining_bits() {
            return Err(DecodeError::UnexpectedEnd);
        }
        let mut out = 0u64;
        let mut got = 0u32;
        while got < n {
            let byte = self.data[self.pos / 8] as u64;
            let bit_off = (self.pos % 8) as u32;
            let take = (8 - bit_off).min(n - got);
            let chunk = (byte >> bit_off) & ((1u64 << take) - 1);
            out |= chunk << got;
            got += take;
            self.pos += take as usize;
        }
        Ok(out)
    }

    #[inline]
    pub fn read_bool(&mut self) -> Result<bool, DecodeError> {
        Ok(self.read_bits(1)? != 0)
    }

    #[inline]
    pub fn read_u8(&mut self) -> Result<u8, DecodeError> {
        Ok(self.read_bits(8)? as u8)
    }

    #[inline]
    pub fn read_u16(&mut self) -> Result<u16, DecodeError> {
        Ok(self.read_bits(16)? as u16)
    }

    #[inline]
    pub fn read_u32(&mut self) -> Result<u32, DecodeError> {
        Ok(self.read_bits(32)? as u32)
    }

    #[inline]
    pub fn read_f32(&mut self) -> Result<f32, DecodeError> {
        Ok(f32::from_bits(self.read_u32()?))
    }

    /// A raw `f32` that must be finite (rejects NaN and infinities).
    pub fn read_finite_f32(&mut self) -> Result<f32, DecodeError> {
        let v = self.read_f32()?;
        if v.is_finite() {
            Ok(v)
        } else {
            Err(DecodeError::Invalid("non-finite float"))
        }
    }

    pub fn read_varint(&mut self) -> Result<u64, DecodeError> {
        let mut out = 0u64;
        for i in 0..10 {
            let byte = self.read_bits(8)?;
            let group = byte & 0x7F;
            let shift = 7 * i;
            if shift == 63 && group > 1 {
                return Err(DecodeError::Invalid("varint overflow"));
            }
            out |= group << shift;
            if byte & 0x80 == 0 {
                return Ok(out);
            }
        }
        Err(DecodeError::Invalid("varint overflow"))
    }

    /// Varint that must fit in `u32`.
    pub fn read_varint_u32(&mut self) -> Result<u32, DecodeError> {
        let v = self.read_varint()?;
        u32::try_from(v).map_err(|_| DecodeError::Invalid("varint exceeds u32"))
    }

    /// Reads a length prefix and checks it against `max` and against the
    /// data actually left (each element needs at least `min_bits_each` bits),
    /// so a forged length can never trigger a huge allocation.
    pub fn read_len(&mut self, max: usize, min_bits_each: usize) -> Result<usize, DecodeError> {
        let len = self.read_varint()?;
        if len > max as u64 {
            return Err(DecodeError::TooLong {
                len,
                max: max as u64,
            });
        }
        let len = len as usize;
        if len.saturating_mul(min_bits_each) > self.remaining_bits() {
            return Err(DecodeError::UnexpectedEnd);
        }
        Ok(len)
    }

    pub fn read_bytes(&mut self, max: usize) -> Result<Vec<u8>, DecodeError> {
        let len = self.read_len(max, 8)?;
        let mut out = Vec::with_capacity(len);
        for _ in 0..len {
            out.push(self.read_u8()?);
        }
        Ok(out)
    }

    pub fn read_string(&mut self, max: usize) -> Result<String, DecodeError> {
        let bytes = self.read_bytes(max)?;
        String::from_utf8(bytes).map_err(|_| DecodeError::Invalid("string is not UTF-8"))
    }

    /// Checks that only zero padding (fewer than 8 bits) is left.
    pub fn finish(self) -> Result<(), DecodeError> {
        let rem = self.remaining_bits();
        if rem >= 8 {
            return Err(DecodeError::TrailingBytes(rem / 8));
        }
        if rem > 0 {
            let last = self.data[self.data.len() - 1] as u32;
            if last >> (8 - rem) != 0 {
                return Err(DecodeError::Invalid("non-zero padding bits"));
            }
        }
        Ok(())
    }
}

/// ZigZag-maps a signed value so small magnitudes become small unsigned ones.
#[inline]
pub fn zigzag16(v: i16) -> u16 {
    ((v << 1) ^ (v >> 15)) as u16
}

#[inline]
pub fn unzigzag16(v: u16) -> i16 {
    ((v >> 1) as i16) ^ -((v & 1) as i16)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ls_math::Pcg32;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    #[test]
    fn mixed_fields_round_trip() {
        let mut rng = Pcg32::new(1, 1);
        for _ in 0..200 {
            let fields: Vec<(u64, u32)> = (0..50)
                .map(|_| {
                    let n = rng.below(65);
                    let v = rng.next_u64();
                    let v = if n == 64 { v } else { v & ((1u64 << n) - 1) };
                    (v, n)
                })
                .collect();
            let mut w = BitWriter::new();
            for &(v, n) in &fields {
                w.write_bits(v, n);
            }
            let bits = w.bit_len();
            let bytes = w.finish();
            assert_eq!(bytes.len(), bits.div_ceil(8));
            let mut r = BitReader::new(&bytes);
            for &(v, n) in &fields {
                assert_eq!(r.read_bits(n).unwrap(), v);
            }
            r.finish().unwrap();
        }
    }

    #[test]
    fn varints_strings_floats() {
        let values = [
            0u64,
            1,
            127,
            128,
            300,
            16_383,
            16_384,
            u32::MAX as u64,
            u64::MAX,
        ];
        let mut w = BitWriter::new();
        w.write_bool(true); // misalign on purpose
        for &v in &values {
            w.write_varint(v);
        }
        w.write_str("Loose Screws ✓");
        w.write_f32(-0.0);
        w.write_f32(1.5e-40);
        let bytes = w.finish();
        let mut r = BitReader::new(&bytes);
        assert!(r.read_bool().unwrap());
        for &v in &values {
            assert_eq!(r.read_varint().unwrap(), v);
        }
        assert_eq!(r.read_string(64).unwrap(), "Loose Screws ✓");
        assert_eq!(r.read_f32().unwrap().to_bits(), (-0.0f32).to_bits());
        assert_eq!(r.read_f32().unwrap().to_bits(), 1.5e-40f32.to_bits());
        r.finish().unwrap();
    }

    #[test]
    fn overrun_and_hostile_lengths_fail_cleanly() {
        let mut r = BitReader::new(&[0xFF]);
        assert_eq!(r.read_bits(9), Err(DecodeError::UnexpectedEnd));
        // Length prefix claims 1 GB of data.
        let mut w = BitWriter::new();
        w.write_varint(1 << 30);
        let bytes = w.finish();
        let mut r = BitReader::new(&bytes);
        assert!(matches!(
            r.read_bytes(usize::MAX),
            Err(DecodeError::UnexpectedEnd)
        ));
        let mut r = BitReader::new(&bytes);
        assert!(matches!(r.read_bytes(16), Err(DecodeError::TooLong { .. })));
        // Unterminated varint.
        let mut r = BitReader::new(&[0x80; 12]);
        assert!(r.read_varint().is_err());
        // Trailing data is detected.
        assert_eq!(
            BitReader::new(&[0, 0]).finish(),
            Err(DecodeError::TrailingBytes(2))
        );
    }

    #[test]
    fn zigzag_round_trip() {
        for v in i16::MIN..=i16::MAX {
            assert_eq!(unzigzag16(zigzag16(v)), v);
        }
        assert_eq!(zigzag16(0), 0);
        assert_eq!(zigzag16(-1), 1);
        assert_eq!(zigzag16(1), 2);
    }
}
