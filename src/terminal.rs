//! Terminal frontend: renders on the GPU into a texture, reads the pixels back
//! and displays them with kitty's graphics protocol.
//!
//! Protocol: <https://sw.kovidgoyal.net/kitty/graphics-protocol/>
//! Keyboard: kitty's progressive enhancement, which also reports key releases.

use std::io::{self, Write};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crossterm::event::{
    self, Event, KeyCode, KeyEventKind, KeyModifiers, KeyboardEnhancementFlags,
    PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use crossterm::{cursor, execute, terminal};
use vello::util::RenderContext;

use crate::frame::FrameRenderer;
use crate::game::Game;
use crate::gamepad::Gamepads;
use crate::stats::FrameStats;

/// The image id used with kitty. Reusing it every frame replaces the previous image.
const IMAGE_ID: u32 = 1;
const FRAME_TIME: Duration = Duration::from_micros(16_667);
/// How long a shared-memory frame may stay unread before we assume the
/// terminal doesn't support that transfer mode.
const SHM_TIMEOUT: Duration = Duration::from_secs(3);

/// How the pixels get to the terminal.
#[derive(Clone, Copy)]
pub enum Transfer {
    /// POSIX shared memory (`t=s`): fast, but local only.
    SharedMemory,
    /// Base64 in the terminal stream (`t=d`): slower, but works over SSH.
    Direct,
}

impl Default for Transfer {
    fn default() -> Self {
        if cfg!(target_os = "linux") {
            Transfer::SharedMemory
        } else {
            Transfer::Direct
        }
    }
}

impl Transfer {
    /// Maximum number of pixels to render per frame; kitty scales the image
    /// up to fill the terminal. Full resolution is too much data per frame
    /// (a 1900×2100 frame is 16 MB) for the terminal to keep up at 60 FPS.
    fn pixel_budget(self) -> f64 {
        match self {
            Transfer::SharedMemory => 1_000_000.0,
            Transfer::Direct => 250_000.0,
        }
    }
}

pub fn run(transfer: Transfer) {
    if let Err(err) = run_inner(transfer) {
        eprintln!("camel-eon: {err}");
        std::process::exit(1);
    }
}

fn run_inner(transfer: Transfer) -> Result<(), Box<dyn std::error::Error>> {
    let mut context = RenderContext::new();
    let dev_id = pollster::block_on(context.device(None)).ok_or("no compatible GPU found")?;
    let device = &context.devices[dev_id].device;
    let queue = &context.devices[dev_id].queue;
    let mut renderer = FrameRenderer::new(device)?;
    let _guard = TerminalGuard::enter()?;
    let mut out = io::BufWriter::new(io::stdout().lock());
    let mut game = Game::new();
    // Over SSH (`--direct`) the sound would play on the wrong machine.
    let mut audio = crate::audio::Audio::new();
    if !matches!(transfer, Transfer::Direct) {
        audio.start();
    }
    let mut gamepads = Gamepads::new();
    // The last shared-memory frame sent, until the terminal has read it.
    let mut pending: Option<(PathBuf, Instant)> = None;
    let mut frame_number: u64 = 0;
    let mut last_frame = Instant::now();
    let mut stats = FrameStats::default();

    loop {
        let frame_start = Instant::now();

        while event::poll(Duration::ZERO)? {
            let Event::Key(key) = event::read()? else { continue };
            let pressed = key.kind != KeyEventKind::Release;
            match key.code {
                KeyCode::Left | KeyCode::Char('a') => game.input.left = pressed,
                KeyCode::Right | KeyCode::Char('d') => game.input.right = pressed,
                KeyCode::Char(' ') | KeyCode::Char('z') | KeyCode::Up | KeyCode::Char('w') => game.input.jump = pressed,
                KeyCode::Char('m') if key.kind == KeyEventKind::Press => audio.toggle_mute(),
                KeyCode::Esc | KeyCode::Char('q') => return Ok(()),
                KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    return Ok(());
                }
                _ => {}
            }
        }

        // Flow control: kitty deletes the shared-memory object once it has read
        // it. Until then, skip rendering so we never outrun the terminal.
        if let Some((path, sent)) = &pending {
            if path.exists() {
                if sent.elapsed() > SHM_TIMEOUT {
                    return Err("the terminal never read the shared-memory frame; \
                                use kitty, or try --direct"
                        .into());
                }
                std::thread::sleep(Duration::from_millis(1));
                continue;
            }
            pending = None;
        }

        // The terminal can be resized at any time, so read its size every frame.
        let size = terminal::window_size()?;
        if size.width == 0 || size.height == 0 {
            return Err("the terminal doesn't report its size in pixels; run in kitty".into());
        }
        let pixels = size.width as f64 * size.height as f64;
        let scale = (transfer.pixel_budget() / pixels).sqrt().min(1.0);
        let width = ((size.width as f64 * scale) as u32).max(1);
        let height = ((size.height as f64 * scale) as u32).max(1);

        let work_start = Instant::now();
        game.pad = gamepads.poll();
        let now = Instant::now();
        game.update(now.duration_since(last_frame).as_secs_f64());
        audio.feed(game.ambience(), &mut game.sounds);
        last_frame = now;
        renderer.render(device, queue, width, height, |layers, hair| {
            let info = game.draw(layers, width as f64, height as f64, hair);
            stats.draw(&mut layers.front, width as f64, height as f64, true, 1.0);
            info
        })?;
        let pixels = renderer.read_pixels(device, queue)?;

        execute!(out, cursor::MoveTo(0, 0))?;
        let placement = Placement {
            width,
            height,
            columns: size.columns,
            rows: size.rows,
        };
        match transfer {
            Transfer::SharedMemory => {
                let path = send_shared_memory(&mut out, &pixels, placement, frame_number)?;
                pending = Some((path, Instant::now()));
            }
            Transfer::Direct => send_direct(&mut out, &pixels, placement)?,
        }
        out.flush()?;
        frame_number += 1;

        if let Some(fps) = stats.frame(work_start.elapsed().as_secs_f64()) {
            let max = stats.possible_fps();
            execute!(out, terminal::SetTitle(format!("Camel Eon — {fps:.0} FPS (max {max:.0})")))?;
        }

        if let Some(rest) = FRAME_TIME.checked_sub(frame_start.elapsed()) {
            std::thread::sleep(rest);
        }
    }
}

/// The image size in pixels and the area in terminal cells it should fill.
#[derive(Clone, Copy)]
struct Placement {
    width: u32,
    height: u32,
    columns: u16,
    rows: u16,
}

impl Placement {
    /// Shared keys: raw RGBA, replace image `IMAGE_ID`, don't move the cursor,
    /// scale to the whole terminal and suppress responses (`q=2`).
    fn keys(self) -> String {
        format!(
            "a=T,f=32,s={},v={},i={IMAGE_ID},p=1,q=2,C=1,c={},r={}",
            self.width, self.height, self.columns, self.rows
        )
    }
}

/// Writes the pixels to `/dev/shm` and tells kitty to read them from there.
/// Kitty unlinks the object after reading it; returns its path so the caller
/// can wait for that.
fn send_shared_memory(
    out: &mut impl Write,
    pixels: &[u8],
    placement: Placement,
    frame: u64,
) -> io::Result<PathBuf> {
    let name = format!("camel-eon-{}-{frame}", std::process::id());
    let path = PathBuf::from(format!("/dev/shm/{name}"));
    std::fs::write(&path, pixels)?;
    write!(
        out,
        "\x1b_G{},t=s,S={};{}\x1b\\",
        placement.keys(),
        pixels.len(),
        base64(format!("/{name}").as_bytes())
    )?;
    Ok(path)
}

/// Sends the pixels base64-encoded in the terminal stream, in chunks of at most 4096 bytes.
fn send_direct(out: &mut impl Write, pixels: &[u8], placement: Placement) -> io::Result<()> {
    let encoded = base64(pixels);
    let mut chunks = encoded.as_bytes().chunks(4096).peekable();
    let mut first = true;
    while let Some(chunk) = chunks.next() {
        let more = chunks.peek().is_some() as u8;
        if first {
            write!(out, "\x1b_G{},t=d,m={more};", placement.keys())?;
            first = false;
        } else {
            write!(out, "\x1b_Gm={more};")?;
        }
        out.write_all(chunk)?;
        out.write_all(b"\x1b\\")?;
    }
    Ok(())
}

fn base64(data: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = (b[0] as u32) << 16 | (b[1] as u32) << 8 | b[2] as u32;
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(ALPHABET[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// Puts the terminal in game mode and restores it when dropped, even on panic.
struct TerminalGuard;

impl TerminalGuard {
    fn enter() -> io::Result<Self> {
        terminal::enable_raw_mode()?;
        execute!(
            io::stdout(),
            terminal::EnterAlternateScreen,
            cursor::Hide,
            PushKeyboardEnhancementFlags(
                KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
                    | KeyboardEnhancementFlags::REPORT_EVENT_TYPES
            )
        )?;
        Ok(Self)
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let mut out = io::stdout();
        let _ = write!(out, "\x1b_Ga=d,d=I,i={IMAGE_ID},q=2\x1b\\");
        let _ = execute!(
            out,
            PopKeyboardEnhancementFlags,
            cursor::Show,
            terminal::LeaveAlternateScreen
        );
        let _ = terminal::disable_raw_mode();
        // Remove any frame the terminal never read.
        let prefix = format!("camel-eon-{}-", std::process::id());
        if let Ok(entries) = std::fs::read_dir("/dev/shm") {
            for entry in entries.flatten() {
                if entry.file_name().to_string_lossy().starts_with(&prefix) {
                    let _ = std::fs::remove_file(entry.path());
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::base64;

    #[test]
    fn base64_matches_rfc4648() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }
}
