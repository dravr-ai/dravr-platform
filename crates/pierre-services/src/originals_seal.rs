// ABOUTME: Seals an originals bundle with a passphrase as an age v1 file (scrypt recipient), so `age -d` opens it anywhere
// ABOUTME: The passphrase and keys are never stored; a truncated, reordered or altered file fails to open, and so does a wrong passphrase

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # A passphrase-sealed originals bundle
//!
//! An athlete may seal their originals bundle with a passphrase (owl concept
//! 18, lane C). The platform encrypts the bundle as it streams into the blob
//! store and forgets the passphrase and every key: none is stored, logged or
//! returned, so a lost passphrase is a lost bundle, and the athlete builds a
//! new one.
//!
//! The sealed file is an [age v1](https://c2sp.org/age) file with one scrypt
//! recipient, the format `age -p` writes, so the athlete opens it with any age
//! implementation (`age -d`, `rage -d`) and needs nothing from Dravr:
//!
//! ```text
//! age-encryption.org/v1
//! -> scrypt <salt, base64> <log2 N>
//! <file key wrapped by ChaCha20-Poly1305 under scrypt(passphrase), base64>
//! --- <HMAC-SHA256 of the header above, base64>
//! <payload nonce (16)> <payload: ChaCha20-Poly1305 STREAM, 64 KiB chunks>
//! ```
//!
//! The payload is the STREAM construction (Hoang, Reyhanitabar, Rogaway,
//! Vizár): the counter in each chunk's nonce makes a reordered chunk fail, the
//! last-chunk flag makes a truncated file fail, and the header MAC makes an
//! altered header fail. The primitives are the platform's `ring`
//! (ChaCha20-Poly1305, HKDF-SHA256, HMAC-SHA256) and the `RustCrypto` `scrypt`
//! crate for the passphrase KDF the format names. The `age` crate itself is not
//! linked: it brings a localisation stack and nine duplicate crate versions
//! for a format this file writes in a page of code. A file the reference
//! `age` command wrote is a test fixture this reader must open, and `age -d`
//! opening what this file writes is checked by hand on a format change.

use std::{fmt, mem, str};

use base64::engine::general_purpose::STANDARD_NO_PAD;
use base64::Engine as _;
use dravr_barrage::export::BundleSeal;
use dravr_barrage::BarrageError;
use pierre_core::errors::{AppError, AppResult};
use ring::aead::{Aad, LessSafeKey, Nonce, UnboundKey, CHACHA20_POLY1305, NONCE_LEN};
use ring::hkdf::{self, KeyType, HKDF_SHA256};
use ring::hmac;
use ring::rand::{SecureRandom, SystemRandom};
use secrecy::{ExposeSecret, SecretString};
use zeroize::Zeroizing;

/// The version line every age v1 file starts with.
const VERSION_LINE: &str = "age-encryption.org/v1";

/// The label scrypt's salt is prefixed with.
const SCRYPT_LABEL: &[u8] = b"age-encryption.org/v1/scrypt";

/// scrypt's work factor, as log2 N: 2^17 with r = 8 takes 128 MiB per
/// derivation. Two builds run at once at most, beside the backend's other
/// work in 2 GiB; age's own default (18) would double that. Every age
/// implementation opens it: they accept up to 2^22 by default.
const SCRYPT_LOG_N: u8 = 17;

/// The largest work factor [`unseal`] accepts, the reference implementation's
/// default limit, so a file cannot make the platform derive for minutes.
const SCRYPT_MAX_LOG_N: u8 = 22;

const SCRYPT_R: u32 = 8;
const SCRYPT_P: u32 = 1;

/// Plaintext bytes per payload chunk, fixed by the format.
const CHUNK_SIZE: usize = 64 * 1024;

const FILE_KEY_LEN: usize = 16;
const SALT_LEN: usize = 16;
const PAYLOAD_NONCE_LEN: usize = 16;
const KEY_LEN: usize = 32;
const TAG_LEN: usize = 16;

/// The shortest passphrase a seal takes, in characters.
const MIN_PASSPHRASE_CHARS: usize = 12;

/// The longest passphrase a seal takes, in characters.
const MAX_PASSPHRASE_CHARS: usize = 1024;

/// HKDF output length for `ring`.
struct Len(usize);

impl KeyType for Len {
    fn len(&self) -> usize {
        self.0
    }
}

/// HKDF-SHA256 of `ikm` under `salt` and `info`, 32 bytes.
fn hkdf_sha256(ikm: &[u8], salt: &[u8], info: &[u8]) -> AppResult<Zeroizing<[u8; KEY_LEN]>> {
    let mut out = Zeroizing::new([0_u8; KEY_LEN]);
    hkdf::Salt::new(HKDF_SHA256, salt)
        .extract(ikm)
        .expand(&[info], Len(KEY_LEN))
        .and_then(|okm| okm.fill(out.as_mut()))
        .map_err(|_| AppError::internal("sealed bundle: HKDF failed"))?;
    Ok(out)
}

/// The key that wraps the file key: scrypt of `passphrase` under `salt`.
fn scrypt_key(
    passphrase: &SecretString,
    salt: &[u8],
    log_n: u8,
) -> AppResult<Zeroizing<[u8; KEY_LEN]>> {
    let params = scrypt::Params::new(log_n, SCRYPT_R, SCRYPT_P, KEY_LEN)
        .map_err(|e| AppError::invalid_input(format!("sealed bundle scrypt parameters: {e}")))?;
    let mut labelled = Vec::with_capacity(SCRYPT_LABEL.len() + salt.len());
    labelled.extend_from_slice(SCRYPT_LABEL);
    labelled.extend_from_slice(salt);
    let mut key = Zeroizing::new([0_u8; KEY_LEN]);
    scrypt::scrypt(
        passphrase.expose_secret().as_bytes(),
        &labelled,
        &params,
        key.as_mut(),
    )
    .map_err(|e| AppError::internal(format!("sealed bundle key derivation: {e}")))?;
    Ok(key)
}

fn aead_key(key: &[u8; KEY_LEN]) -> AppResult<LessSafeKey> {
    let unbound = UnboundKey::new(&CHACHA20_POLY1305, key)
        .map_err(|_| AppError::internal("sealed bundle key: refused by ChaCha20-Poly1305"))?;
    Ok(LessSafeKey::new(unbound))
}

/// The HMAC key that authenticates the header.
fn header_mac_key(file_key: &[u8; FILE_KEY_LEN]) -> AppResult<hmac::Key> {
    let key = hkdf_sha256(file_key, &[], b"header")?;
    Ok(hmac::Key::new(hmac::HMAC_SHA256, key.as_ref()))
}

/// The key the payload is encrypted under.
fn payload_key(
    file_key: &[u8; FILE_KEY_LEN],
    nonce: &[u8; PAYLOAD_NONCE_LEN],
) -> AppResult<LessSafeKey> {
    aead_key(&*hkdf_sha256(file_key, nonce, b"payload")?)
}

/// The nonce of payload chunk `counter`, the last one when `last`: an 11-byte
/// big-endian counter, then the flag.
fn chunk_nonce(counter: u64, last: bool) -> Nonce {
    let mut nonce = [0_u8; NONCE_LEN];
    nonce[3..NONCE_LEN - 1].copy_from_slice(&counter.to_be_bytes());
    nonce[NONCE_LEN - 1] = u8::from(last);
    Nonce::assume_unique_for_key(nonce)
}

/// `bytes` as a stanza body: unpadded base64 in lines of 64 columns, the last
/// line shorter than 64 (empty when the encoding fills its lines exactly).
fn stanza_body(bytes: &[u8]) -> String {
    let encoded = STANDARD_NO_PAD.encode(bytes);
    let mut body = String::with_capacity(encoded.len() + encoded.len() / 64 + 1);
    let mut rest = encoded.as_str();
    while rest.len() >= 64 {
        let (line, tail) = rest.split_at(64);
        body.push_str(line);
        body.push('\n');
        rest = tail;
    }
    body.push_str(rest);
    body.push('\n');
    body
}

/// Whether `passphrase` is long enough, and not absurdly long, to seal with.
///
/// # Errors
/// An invalid-input error naming the bound it misses.
pub fn check_passphrase(passphrase: &SecretString) -> AppResult<()> {
    let chars = passphrase.expose_secret().chars().count();
    if chars < MIN_PASSPHRASE_CHARS {
        return Err(AppError::invalid_input(format!(
            "A passphrase has at least {MIN_PASSPHRASE_CHARS} characters"
        )));
    }
    if chars > MAX_PASSPHRASE_CHARS {
        return Err(AppError::invalid_input(format!(
            "A passphrase has at most {MAX_PASSPHRASE_CHARS} characters"
        )));
    }
    Ok(())
}

/// The seal of one bundle: its payload key, held only while the bundle is
/// written.
pub struct ExportSeal {
    key: LessSafeKey,
    /// The header and payload nonce, written before the first chunk.
    preamble: Vec<u8>,
    counter: u64,
    buffer: Vec<u8>,
    preamble_sent: bool,
}

impl fmt::Debug for ExportSeal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ExportSeal(..)")
    }
}

impl ExportSeal {
    /// A seal under a fresh file key, wrapped by `passphrase` with a fresh
    /// salt. Slow by design (scrypt): call it off the async runtime.
    ///
    /// # Errors
    /// An invalid-input error for a passphrase [`check_passphrase`] refuses;
    /// an internal error when the system's randomness or the KDF fails.
    pub fn try_new(passphrase: &SecretString) -> AppResult<Self> {
        check_passphrase(passphrase)?;
        let random = SystemRandom::new();
        let mut file_key = Zeroizing::new([0_u8; FILE_KEY_LEN]);
        let mut salt = [0_u8; SALT_LEN];
        let mut nonce = [0_u8; PAYLOAD_NONCE_LEN];
        random
            .fill(file_key.as_mut())
            .and_then(|()| random.fill(&mut salt))
            .and_then(|()| random.fill(&mut nonce))
            .map_err(|_| AppError::internal("sealed bundle: no system randomness"))?;

        let wrap = aead_key(&*scrypt_key(passphrase, &salt, SCRYPT_LOG_N)?)?;
        let mut wrapped = file_key.to_vec();
        wrap.seal_in_place_append_tag(
            Nonce::assume_unique_for_key([0_u8; NONCE_LEN]),
            Aad::empty(),
            &mut wrapped,
        )
        .map_err(|_| AppError::internal("sealed bundle: wrapping the file key failed"))?;

        let mut header = format!(
            "{VERSION_LINE}\n-> scrypt {} {SCRYPT_LOG_N}\n{}---",
            STANDARD_NO_PAD.encode(salt),
            stanza_body(&wrapped),
        );
        let mac = hmac::sign(&header_mac_key(&file_key)?, header.as_bytes());
        header.push(' ');
        header.push_str(&STANDARD_NO_PAD.encode(mac.as_ref()));
        header.push('\n');

        let mut preamble = header.into_bytes();
        preamble.extend_from_slice(&nonce);
        Ok(Self {
            key: payload_key(&file_key, &nonce)?,
            preamble,
            counter: 0,
            buffer: Vec::with_capacity(CHUNK_SIZE),
            preamble_sent: false,
        })
    }

    /// Encrypt one chunk of `plain` into `out`.
    fn chunk(
        &mut self,
        mut plain: Vec<u8>,
        last: bool,
        out: &mut Vec<u8>,
    ) -> Result<(), BarrageError> {
        self.key
            .seal_in_place_append_tag(chunk_nonce(self.counter, last), Aad::empty(), &mut plain)
            .map_err(|_| BarrageError::Blob("sealing a bundle chunk failed".to_owned()))?;
        out.extend_from_slice(&plain);
        self.counter = self
            .counter
            .checked_add(1)
            .ok_or_else(|| BarrageError::Blob("a sealed bundle ran out of chunks".to_owned()))?;
        Ok(())
    }

    fn start(&mut self, out: &mut Vec<u8>) {
        if !self.preamble_sent {
            out.extend_from_slice(&self.preamble);
            self.preamble_sent = true;
        }
    }
}

impl BundleSeal for ExportSeal {
    fn seal(&mut self, plain: &[u8]) -> Result<Vec<u8>, BarrageError> {
        let mut out = Vec::new();
        self.start(&mut out);
        self.buffer.extend_from_slice(plain);
        // A chunk goes only once more bytes follow it: the last one is
        // flagged, and a payload that fills its chunks exactly ends on a full
        // one, never an empty one, so it is written by `finish`.
        while self.buffer.len() > CHUNK_SIZE {
            let rest = self.buffer.split_off(CHUNK_SIZE);
            let full = mem::replace(&mut self.buffer, rest);
            self.chunk(full, false, &mut out)?;
        }
        Ok(out)
    }

    fn finish(&mut self) -> Result<Vec<u8>, BarrageError> {
        let mut out = Vec::new();
        self.start(&mut out);
        let last = mem::take(&mut self.buffer);
        self.chunk(last, true, &mut out)?;
        Ok(out)
    }
}

/// The parts of a sealed file's header [`unseal`] reads.
struct Header<'sealed> {
    salt: Vec<u8>,
    log_n: u8,
    wrapped: Vec<u8>,
    /// The header up to and including `---`, which the MAC covers.
    authenticated: &'sealed [u8],
    mac: Vec<u8>,
    /// Where the payload nonce starts.
    end: usize,
}

/// The next `\n`-terminated line of `sealed` from `at`, and where the one
/// after it starts.
fn line_at(sealed: &[u8], at: usize) -> Option<(&str, usize)> {
    let rest = sealed.get(at..)?;
    let len = rest.iter().position(|&b| b == b'\n')?;
    let line = str::from_utf8(&rest[..len]).ok()?;
    Some((line, at + len + 1))
}

/// Read the header of an age file with exactly one scrypt stanza.
fn parse_header(sealed: &[u8]) -> AppResult<Header<'_>> {
    let not_age = || AppError::invalid_input("This file is not a passphrase-sealed age file");
    let decode = |text: &str| STANDARD_NO_PAD.decode(text).map_err(|_| not_age());
    let (version, at) = line_at(sealed, 0).ok_or_else(not_age)?;
    if version != VERSION_LINE {
        return Err(not_age());
    }
    let (stanza, mut at) = line_at(sealed, at).ok_or_else(not_age)?;
    let args: Vec<&str> = stanza.split(' ').collect();
    let [arrow, "scrypt", salt, log_n] = args.as_slice() else {
        return Err(not_age());
    };
    if *arrow != "->" {
        return Err(not_age());
    }
    let salt = decode(salt)?;
    // A decimal with no sign and no leading zero, as the format requires:
    // `parse` alone would take `+17` and `017`, which `age` refuses.
    if log_n.starts_with('0') || !log_n.bytes().all(|b| b.is_ascii_digit()) {
        return Err(not_age());
    }
    let log_n: u8 = log_n.parse().map_err(|_| not_age())?;
    if salt.len() != SALT_LEN || log_n == 0 || log_n > SCRYPT_MAX_LOG_N {
        return Err(AppError::invalid_input(
            "This sealed bundle names scrypt parameters out of range",
        ));
    }
    // The body: full 64-column lines, then one shorter line; a longer line
    // is not a body the format wraps.
    let mut body = String::new();
    loop {
        let (line, next) = line_at(sealed, at).ok_or_else(not_age)?;
        if line.len() > 64 {
            return Err(not_age());
        }
        at = next;
        body.push_str(line);
        if line.len() < 64 {
            break;
        }
    }
    let wrapped = decode(&body)?;
    // A second stanza, or anything but the MAC line, is not a file this
    // seal wrote: age refuses a scrypt stanza beside any other.
    let (mac_line, end) = line_at(sealed, at).ok_or_else(not_age)?;
    let mac = mac_line.strip_prefix("--- ").ok_or_else(not_age)?;
    Ok(Header {
        salt,
        log_n,
        wrapped,
        authenticated: &sealed[..at + 3],
        mac: decode(mac)?,
        end,
    })
}

/// Open a sealed bundle with `passphrase`: the archive it holds.
///
/// # Errors
/// An invalid-input error when the file is not an age file with one scrypt
/// recipient, names parameters out of range, or does not open with
/// `passphrase`: a wrong passphrase, or a file truncated, reordered or altered.
pub fn unseal(passphrase: &SecretString, sealed: &[u8]) -> AppResult<Vec<u8>> {
    let refused = || AppError::invalid_input("This file does not open with this passphrase");
    let header = parse_header(sealed)?;

    let wrap = aead_key(&*scrypt_key(passphrase, &header.salt, header.log_n)?)?;
    let mut wrapped = header.wrapped;
    let opened = wrap
        .open_in_place(
            Nonce::assume_unique_for_key([0_u8; NONCE_LEN]),
            Aad::empty(),
            &mut wrapped,
        )
        .map_err(|_| refused())?;
    let file_key: Zeroizing<[u8; FILE_KEY_LEN]> =
        Zeroizing::new(opened.try_into().map_err(|_| refused())?);
    hmac::verify(
        &header_mac_key(&file_key)?,
        header.authenticated,
        &header.mac,
    )
    .map_err(|_| refused())?;

    let nonce: [u8; PAYLOAD_NONCE_LEN] = sealed
        .get(header.end..header.end + PAYLOAD_NONCE_LEN)
        .and_then(|n| n.try_into().ok())
        .ok_or_else(refused)?;
    let key = payload_key(&file_key, &nonce)?;
    let body = &sealed[header.end + PAYLOAD_NONCE_LEN..];
    let chunk_len = CHUNK_SIZE + TAG_LEN;
    let mut plain = Vec::with_capacity(body.len());
    let mut at = 0;
    let mut counter = 0_u64;
    loop {
        let remaining = body.len() - at;
        // Every chunk but the last is exactly `chunk_len`; the last is at
        // most that, and nothing follows it.
        let last = remaining <= chunk_len;
        let take = if last { remaining } else { chunk_len };
        if take < TAG_LEN {
            return Err(refused());
        }
        let mut chunk = body[at..at + take].to_vec();
        let opened = key
            .open_in_place(chunk_nonce(counter, last), Aad::empty(), &mut chunk)
            .map_err(|_| refused())?;
        // Only a payload that is empty altogether ends on an empty chunk.
        if last && opened.is_empty() && counter > 0 {
            return Err(refused());
        }
        plain.extend_from_slice(opened);
        if last {
            return Ok(plain);
        }
        at += take;
        counter = counter.checked_add(1).ok_or_else(refused)?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn phrase(text: &str) -> SecretString {
        SecretString::from(text.to_owned())
    }

    fn seal_all(seal: &mut ExportSeal, plain: &[u8], piece: usize) -> Vec<u8> {
        let mut out = Vec::new();
        for chunk in plain.chunks(piece.max(1)) {
            out.extend(seal.seal(chunk).unwrap());
        }
        out.extend(seal.finish().unwrap());
        out
    }

    fn sample(len: usize) -> Vec<u8> {
        (0..len).map(|i| (i % 251) as u8).collect()
    }

    /// Where the payload starts in `sealed`.
    fn payload_at(sealed: &[u8]) -> usize {
        parse_header(sealed).unwrap().end + PAYLOAD_NONCE_LEN
    }

    #[test]
    fn a_sealed_bundle_opens_with_its_passphrase_whatever_its_size() {
        let passphrase = phrase("correct horse battery staple");
        for len in [
            0,
            1,
            CHUNK_SIZE - 1,
            CHUNK_SIZE,
            CHUNK_SIZE + 1,
            3 * CHUNK_SIZE + 17,
        ] {
            let plain = sample(len);
            let mut seal = ExportSeal::try_new(&passphrase).unwrap();
            let sealed = seal_all(&mut seal, &plain, 10_000);
            assert!(
                sealed.starts_with(b"age-encryption.org/v1\n-> scrypt "),
                "bug: the file is not an age file with a scrypt recipient"
            );
            assert_ne!(
                &sealed[payload_at(&sealed)..],
                plain.as_slice(),
                "bug: a sealed file holds no plaintext"
            );
            assert_eq!(
                unseal(&passphrase, &sealed).unwrap(),
                plain,
                "bug: {len} bytes open back as they were sealed"
            );
        }
    }

    #[test]
    fn a_wrong_passphrase_a_truncation_a_reorder_or_an_edit_does_not_open() {
        let passphrase = phrase("correct horse battery staple");
        let plain = sample(3 * CHUNK_SIZE + 5);
        let mut seal = ExportSeal::try_new(&passphrase).unwrap();
        let sealed = seal_all(&mut seal, &plain, CHUNK_SIZE);
        let start = payload_at(&sealed);
        let chunk_len = CHUNK_SIZE + TAG_LEN;

        assert!(
            unseal(&phrase("wrong horse battery staple"), &sealed).is_err(),
            "bug: a wrong passphrase opened the bundle"
        );
        let truncated = &sealed[..start + 2 * chunk_len];
        assert!(
            unseal(&passphrase, truncated).is_err(),
            "bug: a bundle cut at a chunk boundary opened as complete"
        );
        let mut reordered = sealed[..start].to_vec();
        reordered.extend_from_slice(&sealed[start + chunk_len..start + 2 * chunk_len]);
        reordered.extend_from_slice(&sealed[start..start + chunk_len]);
        reordered.extend_from_slice(&sealed[start + 2 * chunk_len..]);
        assert!(
            unseal(&passphrase, &reordered).is_err(),
            "bug: two swapped chunks opened"
        );
        let mut edited = sealed.clone();
        edited[start + 100] ^= 1;
        assert!(
            unseal(&passphrase, &edited).is_err(),
            "bug: an edited chunk opened"
        );
        // The work factor sits in the header: lowering it derives another
        // wrap key, so the edit is refused.
        let header_end = parse_header(&sealed).unwrap().end;
        let text = String::from_utf8_lossy(&sealed[..header_end]).replace(" 17\n", " 16\n");
        let mut header_edited = text.into_bytes();
        header_edited.extend_from_slice(&sealed[header_end..]);
        assert!(
            unseal(&passphrase, &header_edited).is_err(),
            "bug: an edited header opened"
        );
        // A header whose MAC is altered and nothing else is refused by the
        // MAC alone: the wrap still opens.
        let mac_at = sealed[..header_end]
            .iter()
            .rposition(|&b| b == b' ')
            .unwrap()
            + 1;
        let mut mac_edited = sealed.clone();
        mac_edited[mac_at] = if mac_edited[mac_at] == b'A' {
            b'B'
        } else {
            b'A'
        };
        assert!(
            unseal(&passphrase, &mac_edited).is_err(),
            "bug: a header with an altered MAC opened"
        );
    }

    #[test]
    fn a_header_the_format_does_not_allow_is_refused() {
        let passphrase = phrase("correct horse battery staple");
        let mut seal = ExportSeal::try_new(&passphrase).unwrap();
        let sealed = seal_all(&mut seal, b"bundle", 10);
        let header_end = parse_header(&sealed).unwrap().end;
        let header = str::from_utf8(&sealed[..header_end]).unwrap();
        for (from, to) in [(" 17\n", " 017\n"), (" 17\n", " +17\n")] {
            let mut edited = header.replace(from, to).into_bytes();
            edited.extend_from_slice(&sealed[header_end..]);
            assert!(
                parse_header(&edited).is_err(),
                "bug: a work factor written as {to:?} was read"
            );
        }
        // A 65-column line before the wrapped key's own 43 columns: 108
        // columns decode as base64, so only the line width refuses it.
        let body = header.lines().nth(2).unwrap();
        let mut edited = header
            .replacen(body, &format!("{}\n{body}", "A".repeat(65)), 1)
            .into_bytes();
        edited.extend_from_slice(&sealed[header_end..]);
        assert!(
            parse_header(&edited).is_err(),
            "bug: a stanza body line over 64 columns was read"
        );
    }

    #[test]
    fn two_seals_of_one_bundle_share_no_salt_key_or_nonce() {
        let passphrase = phrase("correct horse battery staple");
        let a = ExportSeal::try_new(&passphrase).unwrap();
        let b = ExportSeal::try_new(&passphrase).unwrap();
        let (ha, hb) = (
            parse_header(&a.preamble).unwrap(),
            parse_header(&b.preamble).unwrap(),
        );
        assert_ne!(ha.salt, hb.salt, "bug: two seals reused a salt");
        assert_ne!(
            ha.wrapped, hb.wrapped,
            "bug: two seals wrote one wrapped key"
        );
        assert_ne!(
            a.preamble[ha.end..],
            b.preamble[hb.end..],
            "bug: two seals reused a payload nonce"
        );
        assert_eq!(
            ha.log_n, SCRYPT_LOG_N,
            "bug: the header names the work factor used"
        );
    }

    #[test]
    fn a_short_passphrase_is_refused() {
        assert!(
            ExportSeal::try_new(&phrase("short")).is_err(),
            "bug: a passphrase under the minimum sealed a bundle"
        );
        assert!(
            check_passphrase(&phrase(&"x".repeat(MIN_PASSPHRASE_CHARS))).is_ok(),
            "bug: a passphrase of exactly the minimum was refused"
        );
        assert!(
            check_passphrase(&phrase(&"x".repeat(MAX_PASSPHRASE_CHARS + 1))).is_err(),
            "bug: a passphrase over the maximum was accepted"
        );
    }

    /// A file the reference `age` command (v1.3.2) wrote with `age -p` and
    /// the passphrase `reference passphrase`, around `test\n`: written by
    /// another implementation, it proves this reader speaks the format and
    /// not only the dialect this file writes. `age -d` opening what this file
    /// writes is checked by hand before a change to the format lands.
    #[test]
    fn a_file_the_reference_age_command_wrote_opens() {
        let sealed = include_bytes!("../tests/fixtures/age-scrypt-reference.age");
        let opened = unseal(&phrase("reference passphrase"), sealed).unwrap();
        assert_eq!(
            opened, b"test\n",
            "bug: the reference file opened to other bytes"
        );
        assert!(
            unseal(&phrase("the wrong passphrase"), sealed).is_err(),
            "bug: the reference file opened with a wrong passphrase"
        );
    }
}
