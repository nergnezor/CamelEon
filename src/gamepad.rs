//! Gamepad input via gilrs (Linux, Windows, macOS and the web).
//!
//! Layout (Xbox names, PlayStation in brackets):
//! left stick / d-pad: run, A [Cross]: jump.

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
                eprintln!("camel-eon: gamepads unavailable: {err}");
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
            // Deadzone, rescaled so the stick goes smoothly from 0 to 1.
            let x = pad.value(Axis::LeftStickX) as f64;
            if x.abs() > STICK_DEADZONE {
                let x = x.signum() * ((x.abs() - STICK_DEADZONE) / (1.0 - STICK_DEADZONE)).min(1.0);
                if x.abs() > input.stick_x.abs() {
                    input.stick_x = x;
                }
            }
            input.left |= pad.is_pressed(Button::DPadLeft);
            input.right |= pad.is_pressed(Button::DPadRight);
            input.jump |= pad.is_pressed(Button::South);
        }
        input
    }
}
