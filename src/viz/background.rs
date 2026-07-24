//! Terminal background detection — is the operator on a dark or a light terminal?
//!
//! Two sources, most authoritative first:
//!
//! 1. **OSC 11 query**: ask the emulator for its background color (`ESC ] 11 ; ? BEL`) and read the
//!    `rgb:RRRR/GGGG/BBBB` reply off `/dev/tty` with a short timeout. This is the truth — it is the
//!    color the emulator is actually painting — but it needs a tty and a cooperating emulator.
//! 2. **`$COLORFGBG`**: a `fg;bg` pair of ANSI indexes some emulators export. Stale after a live
//!    theme switch, but better than nothing when the query goes unanswered.
//!
//! Detection is best-effort by design: `None` means "no idea", and callers fall back to a choice
//! that is safe either way (`honeycomb` is basic ANSI, which the terminal's own palette adapts).
//! The raw-mode dance uses `libc` termios directly rather than crossterm because theme resolution
//! lives in the core render path, which must stay free of a terminal backend (Constitution V).

use std::io::{Read, Write};

/// What the terminal's background turned out to be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Background {
    Dark,
    Light,
}

/// Detect the terminal background, best-effort. `None` off a tty, on `TERM=dumb`, or when neither
/// source answers.
pub fn detect() -> Option<Background> {
    if std::env::var_os("TERM").is_some_and(|t| t == "dumb") {
        return None;
    }
    query_osc11().or_else(|| {
        std::env::var("COLORFGBG")
            .ok()
            .and_then(|v| from_colorfgbg(&v))
    })
}

/// Classify a `$COLORFGBG` value (`"15;0"`, `"12;8"`, sometimes `"15;default;0"`): the *last*
/// field is the background index. 0–6 and 8 are the dark half of the ANSI table; 7 and 9–15 light.
fn from_colorfgbg(value: &str) -> Option<Background> {
    let bg: u8 = value.split(';').next_back()?.trim().parse().ok()?;
    Some(match bg {
        0..=6 | 8 => Background::Dark,
        _ => Background::Light,
    })
}

/// Classify an OSC 11 reply payload like `rgb:1e1e/1e1e/2e2e` (1–4 hex digits per channel).
fn from_osc11_payload(payload: &str) -> Option<Background> {
    let rgb = payload.strip_prefix("rgb:")?;
    let mut channels = rgb.split('/').map(|c| {
        // Scale to 8 bits whatever the digit count: "f" → 0xff, "ffff" → 0xff.
        let v = u32::from_str_radix(c, 16).ok()?;
        let bits = (c.len() as u32) * 4;
        Some((v * 255 / ((1u32 << bits) - 1).max(1)) as u8)
    });
    let (r, g, b) = (channels.next()??, channels.next()??, channels.next()??);
    // Rec. 601 luma on the emulator's own paint: past mid-gray it reads as a light background.
    let luma = 0.299 * f64::from(r) + 0.587 * f64::from(g) + 0.114 * f64::from(b);
    Some(if luma > 127.5 {
        Background::Light
    } else {
        Background::Dark
    })
}

/// Ask the emulator for its background color over `/dev/tty` and classify the reply. `None` when
/// there is no tty, raw mode can't be entered, or no well-formed reply arrives within ~200ms.
fn query_osc11() -> Option<Background> {
    let mut tty = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/tty")
        .ok()?;
    let fd = std::os::fd::AsRawFd::as_raw_fd(&tty);

    // Raw-ish mode just long enough for the reply: no echo (the reply must not print), no canonical
    // buffering (it arrives without a newline), VTIME=2 ⇒ each read waits at most 200ms.
    // SAFETY: termios is POD; tcgetattr/tcsetattr only read/write it and report failure by return.
    let saved = unsafe {
        let mut t: libc::termios = std::mem::zeroed();
        if libc::tcgetattr(fd, &mut t) != 0 {
            return None;
        }
        let saved = t;
        t.c_lflag &= !(libc::ICANON | libc::ECHO);
        t.c_cc[libc::VMIN] = 0;
        t.c_cc[libc::VTIME] = 2;
        if libc::tcsetattr(fd, libc::TCSANOW, &t) != 0 {
            return None;
        }
        saved
    };

    let result = (|| {
        tty.write_all(b"\x1b]11;?\x07").ok()?;
        tty.flush().ok()?;
        // Reply: ESC ] 11 ; rgb:RRRR/GGGG/BBBB terminated by BEL or ST (ESC \).
        let mut buf = Vec::with_capacity(64);
        let mut byte = [0u8; 1];
        loop {
            match tty.read(&mut byte) {
                Ok(1) => {
                    if byte[0] == 0x07 {
                        break; // BEL
                    }
                    if byte[0] == b'\\' && buf.last() == Some(&0x1b) {
                        buf.pop(); // drop the ESC half of ST
                        break;
                    }
                    buf.push(byte[0]);
                    if buf.len() > 64 {
                        return None; // not an OSC 11 reply
                    }
                }
                _ => return None, // timeout or error: the emulator isn't answering
            }
        }
        let reply = String::from_utf8(buf).ok()?;
        from_osc11_payload(reply.strip_prefix("\x1b]11;")?)
    })();

    // SAFETY: restoring the exact termios read above.
    unsafe {
        libc::tcsetattr(fd, libc::TCSANOW, &saved);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colorfgbg_classifies_by_its_last_field() {
        assert_eq!(from_colorfgbg("15;0"), Some(Background::Dark));
        assert_eq!(from_colorfgbg("0;15"), Some(Background::Light));
        assert_eq!(from_colorfgbg("15;default;0"), Some(Background::Dark));
        assert_eq!(from_colorfgbg("12;8"), Some(Background::Dark));
        assert_eq!(from_colorfgbg("0;7"), Some(Background::Light));
        assert_eq!(from_colorfgbg("garbage"), None);
        assert_eq!(from_colorfgbg(""), None);
    }

    #[test]
    fn osc11_payload_classifies_by_luma_at_any_digit_width() {
        // 16-bit channels, the common emulator form.
        assert_eq!(
            from_osc11_payload("rgb:1e1e/1e1e/2e2e"),
            Some(Background::Dark)
        );
        assert_eq!(
            from_osc11_payload("rgb:ffff/ffff/ffff"),
            Some(Background::Light)
        );
        // 8-bit channels.
        assert_eq!(from_osc11_payload("rgb:ef/f1/f5"), Some(Background::Light));
        // 4-bit channels.
        assert_eq!(from_osc11_payload("rgb:0/0/0"), Some(Background::Dark));
        // Not an rgb reply.
        assert_eq!(from_osc11_payload("cmyk:0/0/0/0"), None);
        assert_eq!(from_osc11_payload(""), None);
    }
}
