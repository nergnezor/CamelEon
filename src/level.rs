//! The levels: solid blocks, trees or props, flies, checkpoints and the
//! goal. Gameplay happens in the z = 0 plane; blocks extend in depth only for
//! looks.

use glam::DVec2;
use vello::peniko::Color;

#[derive(Clone, Copy, PartialEq)]
pub enum BlockKind {
    /// Grass on top, soil in front.
    Ground,
    Stone,
    /// Can be jumped through from below.
    Log,
}

#[derive(Clone, Copy)]
pub struct Block {
    pub x0: f64,
    pub x1: f64,
    pub y0: f64,
    pub y1: f64,
    pub z0: f64,
    pub z1: f64,
    pub kind: BlockKind,
}

impl Block {
    pub fn one_way(&self) -> bool {
        self.kind == BlockKind::Log
    }
}

/// Which world a level is set in; decides how everything is drawn.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Theme {
    Jungle,
    /// A future city at dusk: concrete, steel and a low sun.
    Dusk,
}

#[derive(Clone, Copy, PartialEq)]
pub enum PropKind {
    /// A street lamp that flickers on as evening falls.
    Lamp,
    /// A mast with a turning radar dish.
    Antenna,
    /// A roof vent: a spinning fan under a grille, puffing steam.
    Vent,
    /// A flickering hologram sign on a post.
    Sign,
}

/// Scenery standing on the platforms (only for looks, but it casts shadows).
#[derive(Clone, Copy)]
pub struct Prop {
    pub kind: PropKind,
    /// Where it stands: x, the height of the ground there, and depth.
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

/// A cloth banner hanging from `top` (the underside of a catwalk).
#[derive(Clone, Copy)]
pub struct BannerSpec {
    pub x0: f64,
    pub x1: f64,
    pub top: f64,
    pub length: f64,
    pub z: f64,
    /// The two colours of its stripes.
    pub colors: [Color; 2],
}

pub struct Level {
    pub theme: Theme,
    /// Where jellies live (see `soft`), standing on the ground.
    pub jellies: Vec<DVec2>,
    pub banners: Vec<BannerSpec>,
    pub blocks: Vec<Block>,
    pub props: Vec<Prop>,
    /// Trees behind the play area, for looks: x and the height of the trunk.
    pub trees: Vec<DVec2>,
    /// Home positions of the flies.
    pub flies: Vec<DVec2>,
    pub checkpoints: Vec<DVec2>,
    pub goal: DVec2,
    /// Falling below this means losing a life.
    pub kill_y: f64,
}

const BOTTOM: f64 = -30.0;

fn ground(x0: f64, x1: f64, top: f64) -> Block {
    Block { x0, x1, y0: BOTTOM, y1: top, z0: -1.4, z1: 3.0, kind: BlockKind::Ground }
}

fn stone(x0: f64, x1: f64, y0: f64, y1: f64) -> Block {
    Block { x0, x1, y0, y1, z0: -1.0, z1: 1.6, kind: BlockKind::Stone }
}

fn log(x0: f64, x1: f64, top: f64) -> Block {
    Block { x0, x1, y0: top - 0.5, y1: top, z0: -0.7, z1: 1.0, kind: BlockKind::Log }
}

pub fn jungle() -> Level {
    let p = DVec2::new;
    Level {
        theme: Theme::Jungle,
        props: Vec::new(),
        // Jellies to bounce on: one to find at the start, others for the
        // flies up high.
        jellies: vec![p(5.5, 0.0), p(36.0, 0.0), p(99.0, 0.0), p(139.0, 0.0), p(200.0, 0.0), p(275.0, 0.0)],
        banners: Vec::new(),
        blocks: vec![
            // Invisible-ish walls at both ends.
            Block { x0: -16.0, x1: -8.0, y0: BOTTOM, y1: 14.0, z0: -1.4, z1: 3.0, kind: BlockKind::Stone },
            // Start area.
            ground(-8.0, 22.0, 0.0),
            stone(8.0, 11.0, 0.0, 1.2),
            log(14.0, 17.5, 2.8),
            // First pit (22–27), then logs to jump up on by the tree.
            ground(27.0, 45.0, 0.0),
            log(31.0, 35.0, 3.0),
            log(37.0, 41.5, 5.8),
            // Wide pit (45–55): a running jump.
            ground(55.0, 70.0, 0.0),
            stone(59.0, 61.0, 0.0, 1.2),
            stone(61.0, 63.0, 0.0, 2.4),
            stone(63.0, 65.0, 0.0, 3.6),
            stone(65.0, 68.5, 0.0, 4.8),
            // The cliff, reached from the stones.
            Block { x0: 70.0, x1: 80.0, y0: BOTTOM, y1: 6.0, z0: -1.4, z1: 3.0, kind: BlockKind::Stone },
            // Big drop with a resting log.
            log(89.5, 92.5, 5.0),
            // The long run: open ground to build up speed (momentum grows
            // while running flat out), with logs to hop over from below.
            ground(93.0, 160.0, 0.0),
            log(106.0, 110.0, 3.2),
            log(128.0, 133.0, 2.4),
            log(145.0, 150.0, 2.4),
            // Speed gap (160–171): too wide at base speed, easy at full tilt.
            ground(171.0, 226.0, 0.0),
            stone(190.0, 191.5, 0.0, 0.8),
            stone(208.0, 209.5, 0.0, 0.8),
            // Launch ledge, then the big leap (240–253.5).
            ground(226.0, 240.0, 1.5),
            // Final sprint to the goal.
            ground(253.5, 330.0, 0.0),
            // The goal's pedestal.
            Block { x0: 319.4, x1: 320.6, y0: 0.0, y1: 1.0, z0: 0.1, z1: 1.2, kind: BlockKind::Stone },
            Block { x0: 330.0, x1: 338.0, y0: BOTTOM, y1: 14.0, z0: -1.4, z1: 3.0, kind: BlockKind::Stone },
        ],
        trees: vec![p(41.0, 9.6)],
        flies: vec![
            p(4.0, 1.8),
            p(9.5, 2.7),
            p(15.7, 4.2),
            p(24.5, 3.2),
            p(33.0, 4.7),
            p(39.0, 7.4),
            p(43.5, 4.0),
            p(50.5, 3.5),
            p(62.0, 4.0),
            p(64.0, 5.2),
            p(75.0, 7.5),
            p(86.2, 8.0),
            p(91.0, 6.5),
            p(101.5, 1.9),
            p(108.0, 4.6),
            // Snacks to run through at full speed.
            p(118.0, 1.9),
            p(136.0, 2.0),
            p(152.0, 1.9),
            p(166.0, 3.5),
            p(184.0, 2.0),
            p(199.0, 2.8),
            p(215.0, 2.0),
            p(233.0, 3.0),
            p(247.5, 5.0),
            p(265.0, 2.0),
            p(282.0, 2.2),
            p(300.0, 1.9),
        ],
        checkpoints: vec![
            p(1.0, 0.0),
            p(29.0, 0.0),
            p(57.0, 0.0),
            p(74.0, 6.0),
            p(96.0, 0.0),
            p(130.0, 0.0),
            p(175.0, 0.0),
            p(258.0, 0.0),
        ],
        goal: p(320.0, 0.0),
        kill_y: -7.0,
    }
}

fn catwalk(x0: f64, x1: f64, top: f64) -> Block {
    Block { x0, x1, y0: top - 0.35, y1: top, z0: -0.8, z1: 1.2, kind: BlockKind::Log }
}

/// A rooftop slab, ending in a sheer wall down into the dark.
fn slab(x0: f64, x1: f64, top: f64) -> Block {
    Block { x0, x1, y0: BOTTOM, y1: top, z0: -1.4, z1: 3.2, kind: BlockKind::Ground }
}

/// A cargo container or machine housing standing on a slab.
fn crate_(x0: f64, x1: f64, y0: f64, y1: f64) -> Block {
    Block { x0, x1, y0, y1, z0: -1.0, z1: 1.8, kind: BlockKind::Stone }
}

/// The future city at dusk: the same kind of run as the jungle (a warm-up,
/// stairs up to a tower, a long sprint, a speed gap and a big leap), over
/// rooftops at uneven heights.
pub fn dusk() -> Level {
    let p = DVec2::new;
    let prop = |kind, x: f64, y: f64, z: f64| Prop { kind, x, y, z };
    use PropKind::*;
    Level {
        theme: Theme::Dusk,
        jellies: vec![p(12.5, 0.0), p(57.5, -0.8), p(100.0, 0.0), p(141.0, 0.0), p(200.0, 0.0), p(275.0, 0.0)],
        banners: {
            let banner = |x0: f64, x1: f64, catwalk_top: f64, length: f64, colors: [Color; 2]| BannerSpec {
                x0,
                x1,
                top: catwalk_top - 0.35,
                length,
                z: 0.1,
                colors,
            };
            let (red, cream) = (Color::from_rgb8(0xb8, 0x2a, 0x3a), Color::from_rgb8(0xe8, 0xd8, 0xc0));
            let (teal, ink) = (Color::from_rgb8(0x1e, 0x8a, 0x90), Color::from_rgb8(0x24, 0x1c, 0x3a));
            let (violet, gold) = (Color::from_rgb8(0x6a, 0x2e, 0x8c), Color::from_rgb8(0xf0, 0xb0, 0x40));
            vec![
                banner(32.2, 33.4, 3.8, 2.2, [red, cream]),
                banner(90.4, 91.6, 5.0, 2.6, [teal, ink]),
                banner(107.3, 108.5, 3.2, 2.0, [violet, gold]),
                banner(129.8, 131.0, 2.4, 1.6, [red, cream]),
                banner(147.0, 148.2, 2.4, 1.6, [teal, ink]),
            ]
        },
        blocks: vec![
            Block { x0: -16.0, x1: -8.0, y0: BOTTOM, y1: 14.0, z0: -1.4, z1: 3.2, kind: BlockKind::Stone },
            // The start roof.
            slab(-8.0, 22.0, 0.0),
            crate_(8.0, 11.0, 0.0, 1.3),
            catwalk(14.0, 17.5, 2.8),
            // A small gap up onto a higher roof, catwalks above it.
            slab(27.0, 45.0, 0.8),
            catwalk(31.0, 35.0, 3.8),
            catwalk(37.0, 41.5, 6.6),
            // The wide gap, down onto a lower roof.
            slab(55.0, 70.0, -0.8),
            crate_(59.0, 61.0, -0.8, 0.4),
            crate_(61.0, 63.0, -0.8, 1.6),
            crate_(63.0, 65.0, -0.8, 2.8),
            crate_(65.0, 68.5, -0.8, 4.0),
            // A tower top, reached from the containers.
            Block { x0: 70.0, x1: 80.0, y0: BOTTOM, y1: 6.0, z0: -1.4, z1: 3.2, kind: BlockKind::Stone },
            catwalk(89.5, 92.5, 5.0),
            // The long run.
            slab(93.0, 160.0, 0.0),
            catwalk(106.0, 110.0, 3.2),
            catwalk(128.0, 133.0, 2.4),
            catwalk(145.0, 150.0, 2.4),
            // Speed gap.
            slab(171.0, 226.0, 0.0),
            crate_(190.0, 191.5, 0.0, 0.8),
            crate_(208.0, 209.5, 0.0, 0.8),
            // Launch ledge and the big leap.
            slab(226.0, 240.0, 1.5),
            slab(253.5, 330.0, 0.0),
            // The teleporter's base.
            Block { x0: 319.0, x1: 321.0, y0: 0.0, y1: 0.5, z0: -0.2, z1: 1.4, kind: BlockKind::Stone },
            Block { x0: 330.0, x1: 338.0, y0: BOTTOM, y1: 14.0, z0: -1.4, z1: 3.2, kind: BlockKind::Stone },
        ],
        props: vec![
            prop(Lamp, 3.0, 0.0, -0.9),
            prop(Antenna, 19.0, 0.0, 2.3),
            prop(Vent, 5.5, 0.0, 1.8),
            prop(Sign, 29.5, 0.8, 1.9),
            prop(Lamp, 43.0, 0.8, -0.9),
            prop(Vent, 57.0, -0.8, 1.6),
            prop(Antenna, 77.0, 6.0, 2.2),
            prop(Lamp, 97.0, 0.0, -0.9),
            prop(Vent, 102.0, 0.0, 2.0),
            prop(Lamp, 117.0, 0.0, -0.9),
            prop(Sign, 124.0, 0.0, 2.2),
            prop(Lamp, 139.0, 0.0, -0.9),
            prop(Antenna, 153.0, 0.0, 2.4),
            prop(Lamp, 176.0, 0.0, -0.9),
            prop(Vent, 183.0, 0.0, 1.9),
            prop(Lamp, 198.0, 0.0, -0.9),
            prop(Sign, 214.0, 0.0, 2.1),
            prop(Lamp, 220.0, 0.0, -0.9),
            prop(Vent, 232.0, 1.5, 1.8),
            prop(Lamp, 260.0, 0.0, -0.9),
            prop(Antenna, 271.0, 0.0, 2.4),
            prop(Lamp, 280.0, 0.0, -0.9),
            prop(Vent, 291.0, 0.0, 2.0),
            prop(Lamp, 300.0, 0.0, -0.9),
            prop(Sign, 309.0, 0.0, 2.2),
        ],
        trees: Vec::new(),
        flies: vec![
            p(4.0, 1.8),
            p(9.5, 2.8),
            p(15.7, 4.2),
            p(24.5, 3.4),
            p(33.0, 5.5),
            p(39.0, 8.2),
            p(43.5, 4.8),
            p(50.5, 3.8),
            p(62.0, 3.2),
            p(64.0, 4.4),
            p(75.0, 7.5),
            p(86.2, 8.0),
            p(91.0, 6.5),
            p(101.5, 1.9),
            p(108.0, 4.6),
            p(118.0, 1.9),
            p(136.0, 2.0),
            p(152.0, 1.9),
            p(166.0, 3.5),
            p(184.0, 2.0),
            p(199.0, 2.8),
            p(215.0, 2.0),
            p(233.0, 3.0),
            p(247.5, 5.0),
            p(265.0, 2.0),
            p(282.0, 2.2),
            p(300.0, 1.9),
        ],
        checkpoints: vec![
            p(1.0, 0.0),
            p(29.0, 0.8),
            p(57.0, -0.8),
            p(74.0, 6.0),
            p(96.0, 0.0),
            p(130.0, 0.0),
            p(175.0, 0.0),
            p(258.0, 0.0),
        ],
        goal: p(320.0, 0.5),
        kill_y: -7.0,
    }
}

/// The levels in order: playing through one leads to the next.
pub fn all() -> [fn() -> Level; 2] {
    [dusk, jungle]
}
