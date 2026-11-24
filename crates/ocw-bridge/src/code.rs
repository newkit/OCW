//! The pairing code (spec 14 Phase 10, 10D.2 *Built (local bridge)*): what
//! the desktop shows and the rider types into the browser, e.g.
//! `7K2M-Q9XP-4DA`. It carries everything the browser needs, since a page
//! served over HTTPS from GitHub Pages cannot ask the desktop anything
//! before it has connected:
//!
//! - the desktop's **IPv4 address on the local network** (the common
//!   private ranges in fewer bits: 192.168.x.y in 16, 172.16–31.x.y in 20,
//!   10.x.y.z in 24; any other address in 32),
//! - which of the bridge's four **ports** it listens on ([`port`]),
//! - a 30-bit **secret**: the browser derives the desktop's certificate from
//!   it ([`crate::cert`]) and proves it knows it in its hello,
//! - a **check symbol**, so a typo is reported at once.
//!
//! The symbols are Crockford's base32 (digits and letters without I, L, O,
//! U; typing is case-insensitive, O reads as 0, I and L as 1), in groups of
//! four. A code on 192.168.x.y has 11 symbols, on 10.x.y.z 13.

use std::fmt;
use std::net::Ipv4Addr;

use sha2::{Digest, Sha256};

/// The first of the bridge's UDP ports; a code names one of
/// [`PORT_SLOTS`] from here (the desktop takes the first free one).
pub const BASE_PORT: u16 = 47810;
/// Ports the code can name.
pub const PORT_SLOTS: u16 = 4;
/// Bits of the secret.
pub const SECRET_BITS: u32 = 30;
const SECRET_MASK: u32 = (1 << SECRET_BITS) - 1;

/// Crockford's base32 alphabet.
const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/// The bridge port of a slot.
pub fn port(slot: u8) -> u16 {
    BASE_PORT + u16::from(slot)
}

/// What a code says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PairingCode {
    pub ip: Ipv4Addr,
    /// The port slot (0 – 3): the port is [`port`]`(slot)`.
    pub slot: u8,
    /// The secret (30 bits).
    pub secret: u32,
}

/// Why a typed code is not one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodeError {
    Empty,
    /// A character that is not in the alphabet.
    BadChar(char),
    /// Too short or too long for its address form.
    Length,
    /// The check symbol does not match: a typo.
    Check,
}

impl fmt::Display for CodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(f, "no code typed"),
            Self::BadChar(c) => write!(f, "{c:?} is not in a pairing code (digits and letters)"),
            Self::Length => write!(f, "the code is too short or too long – check every character"),
            Self::Check => write!(f, "the code has a typo – check every character"),
        }
    }
}

impl std::error::Error for CodeError {}

/// Bit writer / reader over a big-endian bit string.
#[derive(Default)]
struct Bits {
    bits: Vec<bool>,
    pos: usize,
}

impl Bits {
    fn push(&mut self, value: u64, n: u32) {
        for i in (0..n).rev() {
            self.bits.push((value >> i) & 1 == 1);
        }
    }

    fn take(&mut self, n: u32) -> Option<u64> {
        let mut v = 0u64;
        for _ in 0..n {
            v = (v << 1) | u64::from(*self.bits.get(self.pos)?);
            self.pos += 1;
        }
        Some(v)
    }

    fn symbols(&self) -> Vec<u8> {
        self.bits
            .chunks(5)
            .map(|c| {
                let mut v = 0u8;
                for i in 0..5 {
                    v = (v << 1) | u8::from(c.get(i).copied().unwrap_or(false));
                }
                v
            })
            .collect()
    }
}

/// The address forms (2 bits) and their address bits.
fn form_of(ip: Ipv4Addr) -> (u64, u64, u32) {
    let [a, b, c, d] = ip.octets();
    let (c, d) = (u64::from(c), u64::from(d));
    match (a, b) {
        (192, 168) => (0, (c << 8) | d, 16),
        (10, _) => (1, (u64::from(b) << 16) | (c << 8) | d, 24),
        (172, 16..=31) => (2, (u64::from(b - 16) << 16) | (c << 8) | d, 20),
        _ => (3, u64::from(u32::from(ip)), 32),
    }
}

fn address_bits(form: u64) -> u32 {
    [16, 24, 20, 32][form as usize]
}

/// Symbols of a form's data (without the check symbol).
fn data_symbols(form: u64) -> usize {
    (2 + address_bits(form) + 2 + SECRET_BITS).div_ceil(5) as usize
}

fn check_symbol(data: &[u8]) -> u8 {
    let h = Sha256::new()
        .chain_update(b"ocw-bridge-code")
        .chain_update(data)
        .finalize();
    h[0] >> 3
}

impl PairingCode {
    pub fn new(ip: Ipv4Addr, slot: u8, secret: u32) -> Self {
        Self {
            ip,
            slot: slot % PORT_SLOTS as u8,
            secret: secret & SECRET_MASK,
        }
    }

    pub fn port(&self) -> u16 {
        port(self.slot)
    }

    /// The symbols without separators.
    pub fn compact(&self) -> String {
        let (form, addr, n) = form_of(self.ip);
        let mut bits = Bits::default();
        bits.push(form, 2);
        bits.push(addr, n);
        bits.push(u64::from(self.slot), 2);
        bits.push(u64::from(self.secret), SECRET_BITS);
        let mut symbols = bits.symbols();
        symbols.push(check_symbol(&symbols));
        symbols
            .iter()
            .map(|&s| ALPHABET[s as usize] as char)
            .collect()
    }

    /// Parses a typed code: case and separators (`-`, spaces, `.`) do not
    /// matter, O is 0, I and L are 1.
    pub fn parse(typed: &str) -> Result<Self, CodeError> {
        let mut symbols = Vec::new();
        for c in typed.chars() {
            if matches!(c, '-' | ' ' | '.' | '_' | '\t') {
                continue;
            }
            let c = match c.to_ascii_uppercase() {
                'O' => '0',
                'I' | 'L' => '1',
                c => c,
            };
            let v = ALPHABET
                .iter()
                .position(|&a| a as char == c)
                .ok_or(CodeError::BadChar(c))?;
            symbols.push(v as u8);
        }
        let Some((&check, data)) = symbols.split_last() else {
            return Err(CodeError::Empty);
        };
        let mut bits = Bits::default();
        for &s in data {
            bits.push(u64::from(s), 5);
        }
        let form = bits.take(2).ok_or(CodeError::Length)?;
        if data.len() != data_symbols(form) {
            return Err(CodeError::Length);
        }
        if check_symbol(data) != check {
            return Err(CodeError::Check);
        }
        let n = address_bits(form);
        let addr = bits.take(n).ok_or(CodeError::Length)?;
        let ip = match form {
            0 => Ipv4Addr::new(192, 168, (addr >> 8) as u8, addr as u8),
            1 => Ipv4Addr::new(10, (addr >> 16) as u8, (addr >> 8) as u8, addr as u8),
            2 => Ipv4Addr::new(
                172,
                16 + (addr >> 16) as u8,
                (addr >> 8) as u8,
                addr as u8,
            ),
            _ => Ipv4Addr::from(addr as u32),
        };
        let slot = bits.take(2).ok_or(CodeError::Length)? as u8;
        let secret = bits.take(SECRET_BITS).ok_or(CodeError::Length)? as u32;
        Ok(Self { ip, slot, secret })
    }

    /// The WebTransport URL of the desktop's bridge.
    pub fn url(&self) -> String {
        format!("https://{}:{}{}", self.ip, self.port(), crate::PATH)
    }
}

/// Shown in groups of four: `7K2M-Q9XP-4DA`.
impl fmt::Display for PairingCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let c = self.compact();
        let groups: Vec<&str> = c
            .as_bytes()
            .chunks(4)
            .map(|g| std::str::from_utf8(g).expect("ascii"))
            .collect();
        write!(f, "{}", groups.join("-"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_round_trip_for_every_address_form() {
        for (ip, len) in [
            (Ipv4Addr::new(192, 168, 1, 23), 11),
            (Ipv4Addr::new(192, 168, 255, 254), 11),
            (Ipv4Addr::new(10, 0, 7, 99), 13),
            (Ipv4Addr::new(172, 31, 200, 1), 12),
            (Ipv4Addr::new(172, 16, 0, 1), 12),
            (Ipv4Addr::new(100, 64, 3, 4), 15),
            (Ipv4Addr::new(127, 0, 0, 1), 15),
        ] {
            for (slot, secret) in [(0, 0), (3, SECRET_MASK), (1, 0x2A5_F00D)] {
                let code = PairingCode::new(ip, slot, secret);
                let text = code.to_string();
                assert_eq!(code.compact().len(), len, "{ip}: {text}");
                assert_eq!(PairingCode::parse(&text), Ok(code), "{text}");
                // Typed in lower case, without dashes, with spaces.
                assert_eq!(PairingCode::parse(&code.compact().to_lowercase()), Ok(code));
                let spaced = text.replace('-', " ");
                assert_eq!(PairingCode::parse(&spaced), Ok(code));
            }
        }
        let c = PairingCode::new(Ipv4Addr::new(192, 168, 1, 23), 2, 12345);
        assert_eq!(c.port(), 47812);
        assert_eq!(c.url(), "https://192.168.1.23:47812/ocw-bridge/1");
        // Groups of four.
        let t = c.to_string();
        assert_eq!(t.len(), 11 + 2);
        assert_eq!(t.matches('-').count(), 2);
    }

    #[test]
    fn confusable_letters_read_as_digits() {
        let c = PairingCode::new(Ipv4Addr::new(192, 168, 0, 10), 0, 0x0101_0101 & SECRET_MASK);
        let text = c.compact();
        let mangled: String = text
            .chars()
            .map(|ch| match ch {
                '0' => 'o',
                '1' => 'l',
                c => c,
            })
            .collect();
        assert_eq!(PairingCode::parse(&mangled), Ok(c));
    }

    #[test]
    fn typos_are_caught() {
        let c = PairingCode::new(Ipv4Addr::new(192, 168, 178, 40), 0, 0x1234_5678 & SECRET_MASK);
        let text = c.compact();
        assert_eq!(PairingCode::parse(""), Err(CodeError::Empty));
        assert_eq!(PairingCode::parse("--"), Err(CodeError::Empty));
        assert_eq!(PairingCode::parse("7K2U"), Err(CodeError::BadChar('U')));
        assert_eq!(PairingCode::parse(&text[..text.len() - 1]), Err(CodeError::Length));
        assert_eq!(PairingCode::parse(&format!("{text}7")), Err(CodeError::Length));
        // Every single changed symbol is caught (by the check symbol, or as
        // another address form of the wrong length), except where the check
        // symbol happens to collide (1 in 32): count them.
        let mut missed = 0;
        let mut tried = 0;
        for i in 0..text.len() {
            for &a in ALPHABET {
                let mut b = text.clone().into_bytes();
                if b[i] == a {
                    continue;
                }
                b[i] = a;
                tried += 1;
                if PairingCode::parse(std::str::from_utf8(&b).unwrap()).is_ok() {
                    missed += 1;
                }
            }
        }
        assert!(missed * 16 < tried, "{missed} of {tried} typos pass");
    }
}
