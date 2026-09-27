//! Frame statistics overlay: actual FPS, how long a frame takes to prepare,
//! and the FPS that would be possible without waiting for vsync. Text is drawn
//! with a tiny built-in stroke font, so no font files are needed.

use vello::kurbo::{Affine, BezPath, Cap, Join, Point, Stroke};
use vello::peniko::Color;
use vello::Scene;
use web_time::Instant;

pub struct FrameStats {
    pub visible: bool,
    window_start: Instant,
    frames: u32,
    work: f64,
    fps: f64,
    work_ms: f64,
}

impl Default for FrameStats {
    fn default() -> Self {
        Self {
            visible: true,
            window_start: Instant::now(),
            frames: 0,
            work: 0.0,
            fps: 0.0,
            work_ms: 0.0,
        }
    }
}

impl FrameStats {
    /// Records one frame that took `work` seconds to prepare (simulation,
    /// scene building and GPU submission, excluding the wait for vsync).
    /// Returns the new FPS twice a second, for e.g. the window title.
    pub fn frame(&mut self, work: f64) -> Option<f64> {
        self.frames += 1;
        self.work += work;
        let elapsed = self.window_start.elapsed().as_secs_f64();
        if elapsed < 0.5 {
            return None;
        }
        self.fps = self.frames as f64 / elapsed;
        self.work_ms = self.work / self.frames as f64 * 1000.0;
        self.frames = 0;
        self.work = 0.0;
        self.window_start = Instant::now();
        Some(self.fps)
    }

    /// Average CPU time per frame over the last half second.
    pub fn cpu_ms(&self) -> f64 {
        self.work_ms
    }

    /// The frame rate the work alone would allow.
    pub fn possible_fps(&self) -> f64 {
        if self.work_ms > 0.0 { 1000.0 / self.work_ms } else { 0.0 }
    }

    pub fn draw(&self, scene: &mut Scene, w: f64, h: f64, vsync: bool, resolution: f64) {
        if !self.visible || self.fps == 0.0 {
            return;
        }
        // "cpu" is the time to prepare a frame on the CPU and the frame rate
        // that alone would allow; the GPU may still be the limit (press V to
        // turn vsync off and see the real maximum).
        let mut text = format!(
            "{:.0} fps  cpu {:.1} ms (max {:.0})",
            self.fps,
            self.work_ms,
            self.possible_fps()
        );
        if resolution < 0.995 {
            text.push_str(&format!("  res {:.0}%", resolution * 100.0));
        }
        let detail = crate::paint::detail();
        if detail > 0 {
            text.push_str(&format!("  detail -{detail}"));
        }
        if !vsync {
            text.push_str("  vsync off");
        }
        let size = (h * 0.014).max(6.0);
        let advance = size * 1.5;
        let width = text.chars().count() as f64 * advance;
        let origin = Point::new(w - width - size * 1.5, size * 2.8);
        draw_text(scene, &text, origin, size);
    }
}

/// Glyph strokes on a grid where x runs 0..1, the baseline is y = 0, lowercase
/// letters reach y = 1, digits y = 1.6 and descenders y = −0.5 (y up).
fn glyph(c: char) -> &'static [&'static [(f64, f64)]] {
    match c {
        '0' => &[&[(0.0, 0.0), (1.0, 0.0), (1.0, 1.6), (0.0, 1.6), (0.0, 0.0)]],
        '1' => &[&[(0.2, 1.3), (0.5, 1.6), (0.5, 0.0)], &[(0.2, 0.0), (0.8, 0.0)]],
        '2' => &[&[(0.0, 1.6), (1.0, 1.6), (1.0, 0.8), (0.0, 0.8), (0.0, 0.0), (1.0, 0.0)]],
        '3' => &[&[(0.0, 1.6), (1.0, 1.6), (1.0, 0.0), (0.0, 0.0)], &[(0.2, 0.8), (1.0, 0.8)]],
        '4' => &[&[(0.0, 1.6), (0.0, 0.8), (1.0, 0.8)], &[(1.0, 1.6), (1.0, 0.0)]],
        '5' => &[&[(1.0, 1.6), (0.0, 1.6), (0.0, 0.8), (1.0, 0.8), (1.0, 0.0), (0.0, 0.0)]],
        '6' => &[&[(1.0, 1.6), (0.0, 1.6), (0.0, 0.0), (1.0, 0.0), (1.0, 0.8), (0.0, 0.8)]],
        '7' => &[&[(0.0, 1.6), (1.0, 1.6), (0.4, 0.0)]],
        '8' => &[&[(0.0, 0.0), (1.0, 0.0), (1.0, 1.6), (0.0, 1.6), (0.0, 0.0)], &[(0.0, 0.8), (1.0, 0.8)]],
        '9' => &[&[(1.0, 0.8), (0.0, 0.8), (0.0, 1.6), (1.0, 1.6), (1.0, 0.0), (0.0, 0.0)]],
        '.' => &[&[(0.45, 0.0), (0.55, 0.0)]],
        'a' => &[&[(0.0, 1.0), (1.0, 1.0), (1.0, 0.0), (0.0, 0.0), (0.0, 0.5), (1.0, 0.5)]],
        'c' => &[&[(1.0, 1.0), (0.0, 1.0), (0.0, 0.0), (1.0, 0.0)]],
        'f' => &[&[(0.9, 1.6), (0.4, 1.6), (0.4, 0.0)], &[(0.1, 1.0), (0.8, 1.0)]],
        'm' => &[&[(0.0, 0.0), (0.0, 1.0), (1.0, 1.0), (1.0, 0.0)], &[(0.5, 1.0), (0.5, 0.0)]],
        'n' => &[&[(0.0, 0.0), (0.0, 1.0), (1.0, 1.0), (1.0, 0.0)]],
        'o' => &[&[(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0), (0.0, 0.0)]],
        'p' => &[&[(0.0, -0.5), (0.0, 1.0), (1.0, 1.0), (1.0, 0.2), (0.0, 0.2)]],
        's' => &[&[(1.0, 1.0), (0.0, 1.0), (0.0, 0.5), (1.0, 0.5), (1.0, 0.0), (0.0, 0.0)]],
        'v' => &[&[(0.0, 1.0), (0.5, 0.0), (1.0, 1.0)]],
        'x' => &[&[(0.0, 0.0), (1.0, 1.0)], &[(0.0, 1.0), (1.0, 0.0)]],
        'y' => &[&[(0.0, 1.0), (0.5, 0.25)], &[(1.0, 1.0), (0.3, -0.5)]],
        'u' => &[&[(0.0, 1.0), (0.0, 0.0), (1.0, 0.0), (1.0, 1.0)]],
        'r' => &[&[(0.0, 0.0), (0.0, 1.0), (0.9, 1.0)]],
        'd' => &[&[(1.0, 1.6), (1.0, 0.0), (0.0, 0.0), (0.0, 1.0), (1.0, 1.0)]],
        't' => &[&[(0.4, 1.5), (0.4, 0.0), (0.9, 0.0)], &[(0.0, 1.0), (0.9, 1.0)]],
        'i' => &[&[(0.5, 0.0), (0.5, 1.0)], &[(0.5, 1.35), (0.5, 1.45)]],
        'l' => &[&[(0.5, 1.6), (0.5, 0.0)]],
        '-' => &[&[(0.15, 0.7), (0.85, 0.7)]],
        'e' => &[&[(0.0, 0.5), (1.0, 0.5), (1.0, 1.0), (0.0, 1.0), (0.0, 0.0), (1.0, 0.0)]],
        '%' => &[&[(0.0, 0.0), (1.0, 1.6)], &[(0.1, 1.5), (0.3, 1.5), (0.3, 1.2), (0.1, 1.2), (0.1, 1.5)], &[(0.7, 0.4), (0.9, 0.4), (0.9, 0.1), (0.7, 0.1), (0.7, 0.4)]],
        '(' => &[&[(0.7, 1.7), (0.35, 1.2), (0.35, 0.0), (0.7, -0.4)]],
        ')' => &[&[(0.3, 1.7), (0.65, 1.2), (0.65, 0.0), (0.3, -0.4)]],
        _ => &[],
    }
}

fn draw_text(scene: &mut Scene, text: &str, origin: Point, size: f64) {
    let mut path = BezPath::new();
    for (i, c) in text.chars().enumerate() {
        let x0 = origin.x + i as f64 * size * 1.5;
        for stroke in glyph(c) {
            for (j, &(x, y)) in stroke.iter().enumerate() {
                let p = Point::new(x0 + x * size, origin.y - y * size);
                if j == 0 { path.move_to(p) } else { path.line_to(p) }
            }
        }
    }
    let style = |w: f64| Stroke::new(w).with_caps(Cap::Round).with_join(Join::Round);
    scene.stroke(&style(size * 0.45), Affine::IDENTITY, Color::from_rgb8(0x1f, 0x16, 0x1e).with_alpha(0.75), None, &path);
    scene.stroke(&style(size * 0.18), Affine::IDENTITY, Color::from_rgb8(0xfb, 0xf4, 0xe4), None, &path);
}
