//! The jungle level: solid blocks, climbables, tongue hooks, flies, checkpoints
//! and the goal. Gameplay happens in the z = 0 plane; blocks extend in depth
//! only for looks.

use glam::DVec2;

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

#[derive(Clone, Copy, PartialEq)]
pub enum ClimbKind {
    Vine,
    Trunk,
}

#[derive(Clone, Copy)]
pub struct Climbable {
    pub x: f64,
    pub y0: f64,
    pub y1: f64,
    pub half_width: f64,
    pub kind: ClimbKind,
}

pub struct Level {
    pub blocks: Vec<Block>,
    pub climbables: Vec<Climbable>,
    /// Flowers Joe can swing from with the tongue.
    pub hooks: Vec<DVec2>,
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

fn vine(x: f64, y0: f64, y1: f64) -> Climbable {
    Climbable { x, y0, y1, half_width: 0.25, kind: ClimbKind::Vine }
}

fn trunk(x: f64, y1: f64) -> Climbable {
    Climbable { x, y0: 0.0, y1, half_width: 0.6, kind: ClimbKind::Trunk }
}

pub fn jungle() -> Level {
    let p = DVec2::new;
    Level {
        blocks: vec![
            // Invisible-ish walls at both ends.
            Block { x0: -16.0, x1: -8.0, y0: BOTTOM, y1: 14.0, z0: -1.4, z1: 3.0, kind: BlockKind::Stone },
            // Start area.
            ground(-8.0, 22.0, 0.0),
            stone(8.0, 11.0, 0.0, 1.2),
            log(14.0, 17.5, 2.8),
            // First pit (22–27), then the vine and tree area.
            ground(27.0, 45.0, 0.0),
            log(30.0, 36.0, 7.5),
            log(37.5, 44.5, 9.6),
            // Wide pit (45–55): swing across on the tongue.
            ground(55.0, 70.0, 0.0),
            stone(59.0, 61.0, 0.0, 1.2),
            stone(61.0, 63.0, 0.0, 2.4),
            stone(63.0, 65.0, 0.0, 3.6),
            // The cliff, climbed with a vine.
            Block { x0: 70.0, x1: 80.0, y0: BOTTOM, y1: 6.0, z0: -1.4, z1: 3.0, kind: BlockKind::Stone },
            // Big drop with two swing flowers and a resting log.
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
        climbables: vec![vine(33.0, 0.0, 7.5), trunk(41.0, 9.6), vine(69.35, 0.0, 6.5)],
        hooks: vec![
            p(24.5, 5.5),
            p(48.5, 6.0),
            p(53.0, 6.5),
            p(84.0, 11.0),
            p(88.5, 11.5),
            // Fallbacks for the speed sections.
            p(166.0, 7.0),
            p(245.5, 8.5),
            p(250.5, 8.5),
        ],
        flies: vec![
            p(4.0, 1.8),
            p(9.5, 2.7),
            p(15.7, 4.2),
            p(24.5, 3.2),
            p(31.5, 8.9),
            p(34.5, 8.9),
            p(43.0, 11.0),
            p(50.5, 3.5),
            p(62.0, 4.0),
            p(64.0, 5.2),
            p(75.0, 7.5),
            p(86.2, 8.0),
            p(91.0, 6.5),
            p(101.5, 1.9),
            p(108.0, 4.6),
            // Snacks to snap up with the tongue at full speed.
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
