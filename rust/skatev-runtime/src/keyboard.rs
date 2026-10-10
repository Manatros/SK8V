//! Keyboard and mouse as a controller (contributed by GTAxSkate).
//!
//! SkateV reads an XInput controller. Without one, the host asks this module
//! each frame for a pad made from the keyboard and mouse (`sv_keyboard_pad`):
//! the `skate_input` crate translates keys into Skate's buttons and sticks,
//! with a fighting-game style trick stick (direction keys typed as motions)
//! and the mouse flicking the stick. The keys are a plain text file beside
//! the game (`SkateVKeys.txt`, written with the defaults on first use, so a
//! player can rebind any action there).
//!
//! Default layout (GTA-like): WASD move, Shift push, Space pop (held = stick
//! down, let go = flick up), arrows / numpad trick stick, E the board (Y:
//! mount, dismount, hold to put away), Q hippie jump, T Skate's X, Ctrl B,
//! the mouse flicks tricks while its trick key is held.

use skate_input::{Controls, Translator};
use std::path::PathBuf;
use std::sync::Mutex;

#[cfg(windows)]
#[link(name = "user32")]
unsafe extern "system" {
    fn GetAsyncKeyState(key: i32) -> i16;
}

#[cfg(not(windows))]
#[allow(non_snake_case)]
unsafe fn GetAsyncKeyState(_key: i32) -> i16 {
    0
}

struct Keyboard {
    controls: Controls,
    translator: Translator,
}

static KEYBOARD: Mutex<Option<Keyboard>> = Mutex::new(None);

/// The default keys: GTA-like, every key used once.
pub fn default_controls() -> Controls {
    let mut controls = Controls::default();
    let index = |id: &str| Controls::action_index(id).expect("a Skate action");
    // Alt is GTA's character wheel, R its reload, the numpad keeps the trick
    // stick only (no one-key tricks there).
    for id in ["trick_7", "trick_8", "trick_9", "trick_4", "trick_6", "trick_1", "trick_2", "trick_3", "camera", "mouse_trick", "ls"] {
        controls.unbind(index(id));
    }
    for (id, key) in [("pop", "Space"), ("a", "LShift"), ("x", "T"), ("b", "LCtrl"), ("hippie", "Q"), ("y", "E")] {
        controls.bind(index(id), key).expect("a key");
    }
    controls.modern = false;
    controls.mouse = skate_input::MouseTrick::Off;
    controls
}

/// Loads the keys from `path`, writing the defaults there when it does not
/// exist. Returns problems found in the file (unknown keys or actions).
pub fn configure(path: PathBuf) -> Vec<String> {
    let (controls, problems) = match std::fs::read_to_string(&path) {
        Ok(text) => Controls::from_text(&text),
        Err(_) => {
            let controls = default_controls();
            let _ = std::fs::write(&path, controls.to_text());
            (controls, Vec::new())
        }
    };
    if let Ok(mut k) = KEYBOARD.lock() {
        *k = Some(Keyboard { controls, translator: Translator::new() });
    }
    problems
}

fn key_down(name: &str) -> bool {
    skate_input::keys::virtual_key(name).is_some_and(|vk| (unsafe { GetAsyncKeyState(vk as i32) } as u16) & 0x8000 != 0)
}

/// This frame's pad from the keyboard and mouse (`mouse`: the frame's look
/// input, x right and y down), or none when nothing is configured. `active`
/// false (the game has no focus, a menu is open) releases everything.
pub fn pad(mouse: [f32; 2], dt: f32, active: bool) -> Option<skate_input::Pad> {
    let mut guard = KEYBOARD.lock().ok()?;
    let k = guard.as_mut()?;
    if !active {
        k.translator.reset();
        return None;
    }
    Some(k.translator.update(&k.controls, key_down, mouse, dt))
}

/// Whether a pad from `pad` does anything (a key held or the stick moved).
pub fn is_active(p: &skate_input::Pad) -> bool {
    p.buttons != 0 || p.triggers != [0, 0] || p.left.iter().chain(&p.right).any(|v| v.abs() > 0.01)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_keys_round_trip_and_use_every_key_once() {
        let controls = default_controls();
        let (back, problems) = Controls::from_text(&controls.to_text());
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(back.to_text(), controls.to_text());
        let mut seen = std::collections::HashSet::new();
        for line in controls.to_text().lines().filter(|l| l.starts_with("bind ")) {
            if let Some((_, key)) = line.split_once(" = ") {
                let key = key.trim();
                assert!(key.is_empty() || seen.insert(key.to_owned()), "{key} bound twice");
            }
        }
    }
}
