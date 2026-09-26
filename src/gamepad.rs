//! Gamepad input via gilrs (Linux, Windows, macOS and the web).
//!
//! Layout (Xbox names, PlayStation in brackets):
//! left stick / d-pad: move and climb, A [Cross]: jump,
//! X [Square], B [Circle] or right trigger: tongue.

use gilrs::{Axis, Button, Gilrs};

use crate::game::Input;

const STICK_DEADZONE: f32 = 0.4;

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
            let x = pad.value(Axis::LeftStickX);
            let y = pad.value(Axis::LeftStickY);
            input.left |= x < -STICK_DEADZONE || pad.is_pressed(Button::DPadLeft);
            input.right |= x > STICK_DEADZONE || pad.is_pressed(Button::DPadRight);
            input.up |= y > STICK_DEADZONE || pad.is_pressed(Button::DPadUp);
            input.down |= y < -STICK_DEADZONE || pad.is_pressed(Button::DPadDown);
            input.jump |= pad.is_pressed(Button::South);
            input.tongue |= pad.is_pressed(Button::West)
                || pad.is_pressed(Button::East)
                || pad.is_pressed(Button::RightTrigger)
                || pad.is_pressed(Button::RightTrigger2);
        }
        input
    }
}
