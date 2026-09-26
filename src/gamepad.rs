//! Gamepad input via gilrs (Linux, Windows, macOS and the web).
//!
//! Layout (Xbox names, PlayStation in brackets):
//! left stick / d-pad: move and climb, A [Cross]: jump,
//! X [Square], B [Circle] or right trigger: tongue.

use gilrs::{Axis, Button, Gilrs};

use crate::game::Input;

const STICK_DEADZONE: f64 = 0.15;

pub struct Gamepads {
    gilrs: Option<Gilrs>,
}

impl Gamepads {
    pub fn new() -> Self {
        let gilrs = match Gilrs::new() {
            Ok(g) => Some(g),
            Err(err) => {
                // Not fatal: the keyboard still works.
                eprintln!("cameljon: gamepads unavailable: {err}");
                None
            }
        };
        Self { gilrs }
    }

    /// Processes pending gamepad events and returns the combined state of all
    /// connected pads.
    pub fn poll(&mut self) -> Input {
        let mut input = Input::default();
        let Some(gilrs) = &mut self.gilrs else { return input };
        while gilrs.next_event().is_some() {}
        for (_, pad) in gilrs.gamepads() {
            // Radial deadzone, rescaled so the stick goes smoothly from 0 to 1.
            let (x, y) = (pad.value(Axis::LeftStickX) as f64, pad.value(Axis::LeftStickY) as f64);
            let len = x.hypot(y);
            if len > STICK_DEADZONE {
                let k = ((len - STICK_DEADZONE) / (1.0 - STICK_DEADZONE)).min(1.0) / len;
                if (x * k).abs() > input.stick_x.abs() {
                    input.stick_x = x * k;
                }
                if (y * k).abs() > input.stick_y.abs() {
                    input.stick_y = y * k;
                }
            }
            input.left |= pad.is_pressed(Button::DPadLeft);
            input.right |= pad.is_pressed(Button::DPadRight);
            input.up |= pad.is_pressed(Button::DPadUp);
            input.down |= pad.is_pressed(Button::DPadDown);
            input.jump |= pad.is_pressed(Button::South);
            input.tongue |= pad.is_pressed(Button::West)
                || pad.is_pressed(Button::East)
                || pad.is_pressed(Button::RightTrigger)
                || pad.is_pressed(Button::RightTrigger2);
        }
        input
    }
}
