// SPDX-License-Identifier: GPL-2.0-only
// Copyright (C) 2026 The Platypus Authors
//! Serial transport — the I/O sibling of the zero-dep `platypus-core`.
//!
//! A minimal blocking serial port (open a device path, configure 8N1 at a chosen baud, read
//! and write) plus the **FT-60 clone-read driver**. The wire *protocol constants* live in
//! [`platypus_core::device::CloneSpec`] (I/O-free facts); this crate performs the actual
//! syscalls, over the `nix` termios wrapper so the core stays dependency-free.
//!
//! Unix/macOS first. Windows would add a `#[cfg(windows)]` port impl behind the same API.

use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::io::AsRawFd;
use std::path::Path;
use std::time::{Duration, Instant};

use nix::libc;
use nix::sys::termios::{self, BaudRate, ControlFlags, FlushArg, SetArg, SpecialCharacterIndices};
use platypus_core::device::CloneSpec;

/// A blocking serial port configured 8N1 with a read timeout (via `VMIN=0`/`VTIME`).
pub struct SerialPort {
    file: File,
}

impl SerialPort {
    /// Open `path` (e.g. `/dev/cu.usbserial-XXXX`) at `baud`, 8 data bits, no parity, 1 stop.
    /// Non-canonical ("raw") mode; a `read` returns after up to `read_timeout` of idle.
    pub fn open(path: &Path, baud: u32, read_timeout: Duration) -> std::io::Result<Self> {
        // O_NOCTTY: don't become the controlling terminal. O_NONBLOCK: don't block on open
        // waiting for carrier — we clear it below for blocking reads bounded by VTIME.
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(libc::O_NOCTTY | libc::O_NONBLOCK)
            .open(path)?;
        let fd = file.as_raw_fd();

        let mut t = termios::tcgetattr(&file).map_err(errno)?;
        termios::cfmakeraw(&mut t); // 8N1, no echo/canon/flow — the clone stream is binary
        t.control_flags |= ControlFlags::CLOCAL | ControlFlags::CREAD;
        let rate = baud_rate(baud)?;
        termios::cfsetspeed(&mut t, rate).map_err(errno)?;
        // VMIN=0, VTIME=Nds: a read returns available bytes, else after N deciseconds idle.
        let vtime = (read_timeout.as_millis() / 100).clamp(1, 255) as u8;
        t.control_chars[SpecialCharacterIndices::VMIN as usize] = 0;
        t.control_chars[SpecialCharacterIndices::VTIME as usize] = vtime;
        termios::tcsetattr(&file, SetArg::TCSANOW, &t).map_err(errno)?;
        // Discard any stale bytes buffered by the adapter before we start a transfer.
        let _ = termios::tcflush(&file, FlushArg::TCIFLUSH);

        // Switch to blocking reads (bounded by VTIME) now that the port is configured.
        unsafe {
            let flags = libc::fcntl(fd, libc::F_GETFL);
            libc::fcntl(fd, libc::F_SETFL, flags & !libc::O_NONBLOCK);
        }
        Ok(SerialPort { file })
    }

    /// Read available bytes (up to `buf.len()`), returning 0 on a timeout with no data.
    pub fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.file.read(buf)
    }

    /// Write all `buf`.
    pub fn write_all(&mut self, buf: &[u8]) -> std::io::Result<()> {
        self.file.write_all(buf)
    }
}

/// The byte read/write surface the clone drivers need. [`SerialPort`] implements it for real
/// I/O; tests drive the framing loops with an in-memory fake. `read` returns `Ok(0)` when no
/// data is available (the VTIME idle timeout on a real port) — the signal the loops use to
/// detect end-of-stream.
pub trait Port {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize>;
    fn write_all(&mut self, buf: &[u8]) -> std::io::Result<()>;
}

impl Port for SerialPort {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        SerialPort::read(self, buf)
    }
    fn write_all(&mut self, buf: &[u8]) -> std::io::Result<()> {
        SerialPort::write_all(self, buf)
    }
}

/// Timeouts for a clone transfer. Defaults are the hardware bring-up values; tests pass tiny
/// values so the idle-detection loops (which key off wall-clock silence) finish instantly.
#[derive(Debug, Clone, Copy)]
pub struct CloneTimeouts {
    /// How long to wait for the radio to begin sending.
    pub startup: Duration,
    /// Sustained silence that marks the end of the stream (or of a read unit).
    pub idle: Duration,
    /// How long to wait for the radio's ACK after a written chunk.
    pub ack: Duration,
}

impl Default for CloneTimeouts {
    fn default() -> Self {
        Self {
            startup: Duration::from_secs(30),
            idle: Duration::from_secs(3),
            ack: Duration::from_secs(4),
        }
    }
}

fn errno(e: nix::errno::Errno) -> std::io::Error {
    std::io::Error::from_raw_os_error(e as i32)
}

fn baud_rate(baud: u32) -> std::io::Result<BaudRate> {
    Ok(match baud {
        9600 => BaudRate::B9600,
        19200 => BaudRate::B19200,
        38400 => BaudRate::B38400,
        57600 => BaudRate::B57600,
        115200 => BaudRate::B115200,
        _ => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "unsupported baud",
            ))
        }
    })
}

/// Candidate serial ports: `/dev/cu.*` callout devices (USB-serial adapters), excluding the
/// Bluetooth/debug ones. Returns full device paths.
pub fn list_ports() -> Vec<String> {
    let mut ports = Vec::new();
    if let Ok(entries) = std::fs::read_dir("/dev") {
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with("cu.")
                && !name.contains("Bluetooth")
                && !name.contains("debug-console")
                && !name.contains("wlan")
            {
                ports.push(format!("/dev/{name}"));
            }
        }
    }
    ports.sort();
    ports
}

/// Progress / cancellation for a long clone transfer.
pub trait Progress {
    /// `bytes` received so far, out of `total` expected.
    fn update(&mut self, bytes: usize, total: usize);
    /// Return true to abort the transfer.
    fn cancelled(&self) -> bool {
        false
    }
}

/// A no-op progress sink.
pub struct NoProgress;
impl Progress for NoProgress {
    fn update(&mut self, _: usize, _: usize) {}
}

/// Errors from a clone transfer.
#[derive(Debug)]
pub enum CloneError {
    Io(std::io::Error),
    /// The radio never started sending within the startup window.
    Timeout,
    /// Received but the model magic didn't match (wrong radio / mode).
    BadMagic,
    /// The radio didn't acknowledge a written chunk (not in receive mode, or wrong framing).
    NoAck,
    /// The user cancelled.
    Cancelled,
}

impl std::fmt::Display for CloneError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CloneError::Io(e) => write!(f, "serial I/O error: {e}"),
            CloneError::Timeout => write!(f, "the radio didn't start sending — is it in F8 CLONE and did you press the send key?"),
            CloneError::BadMagic => write!(f, "unexpected data — not an FT-60 clone stream"),
            CloneError::NoAck => write!(f, "the radio didn't acknowledge — is it in CLONE receive mode (press MONI → \"-WAIT-\")?"),
            CloneError::Cancelled => write!(f, "cancelled"),
        }
    }
}
impl std::error::Error for CloneError {}
impl From<std::io::Error> for CloneError {
    fn from(e: std::io::Error) -> Self {
        CloneError::Io(e)
    }
}

/// Passively receive whatever the radio streams — **writes nothing back** (maximally
/// non-destructive). Waits up to the startup window for the first bytes, then accumulates
/// until the stream goes idle. Used for the first bring-up capture to observe the real wire
/// framing before we trust any ACK/handshake logic.
pub fn read_passive(
    port: &mut dyn Port,
    total_hint: usize,
    timeouts: CloneTimeouts,
    progress: &mut dyn Progress,
) -> Result<Vec<u8>, CloneError> {
    let startup = timeouts.startup;
    // Require a sustained idle gap before declaring the transfer done, so a brief mid-stream
    // pause (or a slow start) doesn't truncate the capture.
    let idle_grace = timeouts.idle;
    let mut data: Vec<u8> = Vec::with_capacity(total_hint);
    let mut chunk = vec![0u8; 256];
    let start = Instant::now();
    let mut last_data: Option<Instant> = None;
    loop {
        if progress.cancelled() {
            return Err(CloneError::Cancelled);
        }
        let n = port.read(&mut chunk)?;
        if n > 0 {
            data.extend_from_slice(&chunk[..n]);
            last_data = Some(Instant::now());
            progress.update(data.len(), total_hint);
            if data.len() >= total_hint {
                break;
            }
        } else {
            match last_data {
                None if start.elapsed() > startup => return Err(CloneError::Timeout),
                None => continue,
                Some(t) if t.elapsed() > idle_grace => break, // sustained idle after data
                Some(_) => continue,
            }
        }
    }
    Ok(data)
}

/// Read a full FT-60 clone image from the radio.
///
/// The radio sends when the operator triggers a clone-out (F8 CLONE → send key); the PC
/// receives and acknowledges each block with `spec.ack`. We accumulate every received byte,
/// ACKing per `spec.block_size`, until the stream goes idle after data has flowed. The
/// returned bytes are verified to begin with `spec.magic`; the exact length is validated by
/// the caller against `spec.image_size` (framing is confirmed against hardware during
/// bring-up). Non-destructive: this only reads.
pub fn read_ft60_image(
    port: &mut dyn Port,
    spec: &CloneSpec,
    timeouts: CloneTimeouts,
    progress: &mut dyn Progress,
) -> Result<Vec<u8>, CloneError> {
    let startup = timeouts.startup; // time for the operator to trigger the radio
    let idle = timeouts.idle; // sustained idle ⇒ the radio has finished
    let cap = spec.wire_len() * 2; // runaway guard

    // Phase 1 — the header. The radio sends `header_len` bytes (model magic) then waits.
    // Resync on the magic rather than demanding it at byte 0, so a few stale bytes in the
    // adapter's buffer (or a spurious byte at open) don't kill an otherwise-good stream.
    let mut buf: Vec<u8> = Vec::new();
    let mut chunk = vec![0u8; spec.block_size];
    let start = Instant::now();
    let hdr_start = loop {
        if progress.cancelled() {
            return Err(CloneError::Cancelled);
        }
        // A complete header available?
        if let Some(pos) = find_sub(&buf, spec.magic) {
            if buf.len() - pos >= spec.header_len {
                break pos;
            }
        }
        let n = port.read(&mut chunk)?;
        if n > 0 {
            buf.extend_from_slice(&chunk[..n]);
        } else if buf.is_empty() {
            if start.elapsed() > startup {
                return Err(CloneError::Timeout);
            }
        } else if find_sub(&buf, spec.magic).is_none() && buf.len() >= 64 {
            // A burst of data arrived with no `AH017` anywhere → not a clone stream
            // (the radio wasn't actually clone-sending).
            return Err(CloneError::BadMagic);
        } else if start.elapsed() > startup {
            return Err(CloneError::BadMagic);
        }
    };
    let mut image = buf[hdr_start..hdr_start + spec.header_len].to_vec();
    progress.update(image.len(), spec.wire_len());

    // Phase 2 — ACK the unit we just read, then read the next block. The clone cable is
    // half-duplex, so our 1-byte ACK echoes back on RX first; drop it, keep the block.
    loop {
        if progress.cancelled() {
            return Err(CloneError::Cancelled);
        }
        port.write_all(&[spec.ack])?;
        // Read up to echo(1) + one block; stop early once the line goes idle.
        let unit = read_up_to(port, 1 + spec.block_size, idle)?;
        if unit.is_empty() {
            break; // radio finished (nothing after our ACK)
        }
        let payload: &[u8] = if unit[0] == spec.ack {
            &unit[1..]
        } else {
            &unit[..]
        };
        if payload.is_empty() {
            break; // only the echoed ACK came back ⇒ done
        }
        image.extend_from_slice(payload);
        progress.update(image.len(), spec.wire_len());
        if image.len() >= cap {
            break;
        }
    }
    Ok(image)
}

/// Write (clone-out) an FT-60 image **to** the radio. The radio must be armed in CLONE and
/// switched to **receive** (press MONI → the display shows `-WAIT-` / "Clone RX"). The PC
/// sends the header, then the payload in `block_size` chunks; the radio ACKs each. Over the
/// half-duplex cable our sent bytes echo back on RX, so per chunk we drain the echo and then
/// require the radio's `spec.ack`.
///
/// **WARNING — this writes the radio's memory.** Writing back the exact image that was read
/// is safe (every byte equals what's already there, so even a partial write can't corrupt);
/// any other image must have passed the `decode → encode == bytes` round-trip gate first.
pub fn write_ft60_image(
    port: &mut dyn Port,
    spec: &CloneSpec,
    image: &[u8],
    timeouts: CloneTimeouts,
    progress: &mut dyn Progress,
) -> Result<(), CloneError> {
    if image.len() < spec.image_size || !spec.header_matches(image) {
        return Err(CloneError::BadMagic);
    }
    let image = &image[..spec.image_size];

    // Header first — this is where a radio not in receive mode fails fast (NoAck).
    send_chunk(port, &image[..spec.header_len], spec.ack, timeouts.ack)?;
    progress.update(spec.header_len, image.len());

    // Then the payload, one block at a time.
    let mut sent = spec.header_len;
    for block in image[spec.header_len..].chunks(spec.block_size) {
        if progress.cancelled() {
            return Err(CloneError::Cancelled);
        }
        send_chunk(port, block, spec.ack, timeouts.ack)?;
        sent += block.len();
        progress.update(sent, image.len());
    }
    Ok(())
}

/// Send `data`, drain the half-duplex echo of it, and require the peer's `ack` byte. The ACK
/// is the last byte received (after the echo); if the adapter doesn't echo, it's the only one.
fn send_chunk(
    port: &mut dyn Port,
    data: &[u8],
    ack: u8,
    ack_timeout: Duration,
) -> Result<(), CloneError> {
    port.write_all(data)?;
    let back = read_up_to(port, data.len() + 1, ack_timeout)?;
    if back.last() == Some(&ack) {
        Ok(())
    } else {
        Err(CloneError::NoAck)
    }
}

/// First index where `needle` occurs in `hay`, if any.
fn find_sub(hay: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || hay.len() < needle.len() {
        return None;
    }
    hay.windows(needle.len()).position(|w| w == needle)
}

/// Read up to `max` bytes, returning early once no data arrives for `idle`.
fn read_up_to(port: &mut dyn Port, max: usize, idle: Duration) -> Result<Vec<u8>, CloneError> {
    let mut buf = vec![0u8; max];
    let mut got = 0;
    let mut last = Instant::now();
    while got < max {
        let m = port.read(&mut buf[got..])?;
        if m > 0 {
            got += m;
            last = Instant::now();
        } else if last.elapsed() > idle {
            break;
        }
    }
    buf.truncate(got);
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_sub_locates_needle() {
        // Found at the start, in the middle, and at the end.
        assert_eq!(find_sub(b"AH017\x00\x00", b"AH017"), Some(0));
        assert_eq!(find_sub(b"\x00\x01AH017rest", b"AH017"), Some(2));
        assert_eq!(find_sub(b"prefixAH017", b"AH017"), Some(6));
        // Not present.
        assert_eq!(find_sub(b"nothing here", b"AH017"), None);
        // Needle longer than the haystack.
        assert_eq!(find_sub(b"AH", b"AH017"), None);
        // Empty needle is never matched (guarded).
        assert_eq!(find_sub(b"anything", b""), None);
        // Overlapping candidates: returns the first full match.
        assert_eq!(find_sub(b"aaab", b"aab"), Some(1));
    }

    #[test]
    fn baud_rate_maps_supported_and_rejects_others() {
        // Every supported rate maps without error.
        for b in [9600u32, 19200, 38400, 57600, 115200] {
            assert!(baud_rate(b).is_ok(), "{b} should be supported");
        }
        assert_eq!(baud_rate(9600).unwrap(), BaudRate::B9600);
        assert_eq!(baud_rate(115200).unwrap(), BaudRate::B115200);
        // An unsupported rate hits the InvalidInput error path.
        let err = baud_rate(12345).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
    }

    // ---- Framing driver tests over an in-memory fake port ----

    use std::collections::VecDeque;

    /// A scripted port: `to_read` is a queue of chunks the "radio" returns on successive reads
    /// (exhausted ⇒ `Ok(0)`, i.e. idle). Writes are recorded in `written`.
    struct FakePort {
        to_read: VecDeque<Vec<u8>>,
        written: Vec<u8>,
    }
    impl FakePort {
        fn new(chunks: Vec<Vec<u8>>) -> Self {
            FakePort {
                to_read: chunks.into(),
                written: Vec::new(),
            }
        }
    }
    impl Port for FakePort {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            let Some(front) = self.to_read.front_mut() else {
                return Ok(0);
            };
            if front.is_empty() {
                self.to_read.pop_front();
                return Ok(0); // a scripted idle gap
            }
            let n = front.len().min(buf.len());
            buf[..n].copy_from_slice(&front[..n]);
            front.drain(..n);
            if front.is_empty() {
                self.to_read.pop_front();
            }
            Ok(n)
        }
        fn write_all(&mut self, buf: &[u8]) -> std::io::Result<()> {
            self.written.extend_from_slice(buf);
            Ok(())
        }
    }

    /// A tiny spec so tests move a 13-byte "image" (5-byte header + two 4-byte blocks).
    fn tiny_spec() -> CloneSpec {
        CloneSpec {
            baud: 9600,
            image_size: 13,
            header_len: 5,
            block_size: 4,
            block_count: 2,
            ack: 0x06,
            magic: b"AH017",
        }
    }

    /// Fast timeouts so the wall-clock idle loops terminate instantly.
    fn fast() -> CloneTimeouts {
        CloneTimeouts {
            startup: Duration::from_millis(50),
            idle: Duration::from_millis(5),
            ack: Duration::from_millis(50),
        }
    }

    #[test]
    fn read_ft60_image_drains_echo_acks_blocks_and_assembles() {
        let ack = 0x06u8;
        let b0 = [0xA1, 0xA2, 0xA3, 0xA4];
        let b1 = [0xB1, 0xB2, 0xB3, 0xB4];
        // header, then per ACK the half-duplex echo(ack)+block, then only the echoed ACK ⇒ done.
        let mut port = FakePort::new(vec![
            b"AH017".to_vec(),
            [&[ack][..], &b0[..]].concat(),
            [&[ack][..], &b1[..]].concat(),
            vec![ack],
        ]);
        let img = read_ft60_image(&mut port, &tiny_spec(), fast(), &mut NoProgress).unwrap();
        assert_eq!(img, [b"AH017".as_ref(), &b0, &b1].concat());
        assert_eq!(port.written, vec![ack; 3]); // one ACK per read unit (incl. the terminating one)
    }

    #[test]
    fn read_ft60_image_resyncs_past_stale_leading_bytes() {
        let ack = 0x06u8;
        let b0 = [0xA1, 0xA2, 0xA3, 0xA4];
        // Two stale bytes precede the magic; the driver must resync on AH017, not fail.
        let mut port = FakePort::new(vec![
            [&[0x00u8, 0x99][..], b"AH017".as_ref()].concat(),
            [&[ack][..], &b0[..]].concat(),
            vec![ack],
        ]);
        let img = read_ft60_image(&mut port, &tiny_spec(), fast(), &mut NoProgress).unwrap();
        assert_eq!(img, [b"AH017".as_ref(), &b0].concat());
    }

    #[test]
    fn read_ft60_image_times_out_when_radio_silent() {
        let mut port = FakePort::new(vec![]); // never sends
        let err = read_ft60_image(&mut port, &tiny_spec(), fast(), &mut NoProgress).unwrap_err();
        assert!(matches!(err, CloneError::Timeout));
    }

    #[test]
    fn read_ft60_image_rejects_non_clone_burst() {
        // A burst of ≥64 bytes with no magic anywhere ⇒ not a clone stream.
        let mut port = FakePort::new(vec![vec![0x55; 80]]);
        let err = read_ft60_image(&mut port, &tiny_spec(), fast(), &mut NoProgress).unwrap_err();
        assert!(matches!(err, CloneError::BadMagic));
    }

    #[test]
    fn write_ft60_image_sends_header_and_blocks_and_requires_ack() {
        let ack = 0x06u8;
        let image: Vec<u8> = [b"AH017".as_ref(), &[1, 2, 3, 4], &[5, 6, 7, 8]].concat();
        // Per chunk: the radio echoes the chunk back then sends its ACK.
        let hdr_echo = [b"AH017".as_ref(), &[ack]].concat();
        let b0_echo = [&[1u8, 2, 3, 4][..], &[ack]].concat();
        let b1_echo = [&[5u8, 6, 7, 8][..], &[ack]].concat();
        let mut port = FakePort::new(vec![hdr_echo, b0_echo, b1_echo]);
        write_ft60_image(&mut port, &tiny_spec(), &image, fast(), &mut NoProgress).unwrap();
        assert_eq!(port.written, image); // header + both blocks, in order
    }

    #[test]
    fn write_ft60_image_no_ack_is_reported() {
        // The radio echoes the header but never sends its ACK ⇒ NoAck (not in receive mode).
        let mut port = FakePort::new(vec![b"AH017".to_vec()]);
        let image: Vec<u8> = [b"AH017".as_ref(), &[1, 2, 3, 4], &[5, 6, 7, 8]].concat();
        let err =
            write_ft60_image(&mut port, &tiny_spec(), &image, fast(), &mut NoProgress).unwrap_err();
        assert!(matches!(err, CloneError::NoAck));
    }

    #[test]
    fn write_ft60_image_rejects_wrong_magic() {
        let mut port = FakePort::new(vec![]);
        let bad: Vec<u8> = [b"XXXXX".as_ref(), &[1, 2, 3, 4], &[5, 6, 7, 8]].concat();
        let err =
            write_ft60_image(&mut port, &tiny_spec(), &bad, fast(), &mut NoProgress).unwrap_err();
        assert!(matches!(err, CloneError::BadMagic));
    }

    #[test]
    fn list_ports_does_not_crash() {
        // Machine-dependent (which `/dev/cu.*` exist), so only assert the shape and that
        // any returned entry is a `/dev/cu.*` path — never the excluded Bluetooth/debug ones.
        let ports: Vec<String> = list_ports();
        for p in &ports {
            assert!(p.starts_with("/dev/cu."), "unexpected port {p}");
            assert!(!p.contains("Bluetooth"));
            assert!(!p.contains("debug-console"));
        }
        // Sorted ascending.
        let mut sorted = ports.clone();
        sorted.sort();
        assert_eq!(ports, sorted);
    }
}
