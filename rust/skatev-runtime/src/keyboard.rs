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
//! down, let go = flick up), arrow trick stick, E the board (Y:
//! mount, dismount, hold to put away), Q hippie jump, T Skate's X, Ctrl B,
//! Mouse Flick uses CapsLock as a clutch; otherwise the mouse looks around.

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
    path: PathBuf,
}

static KEYBOARD: Mutex<Option<Keyboard>> = Mutex::new(None);

/// Four translation mentalities. Start empty: no hidden aliases or disabled
/// gesture bindings that unexpectedly return when a scheme is enabled.
fn preset(name: &str) -> Result<Controls, String> {
    let mut c = Controls::default();
    for keys in &mut c.keys { keys.clear(); }
    c.classic = true;
    c.modern = false;
    c.mouse = skate_input::MouseTrick::Off;
    let mut bindings = vec![
        ("lean_forward", "W"), ("lean_back", "S"), ("steer_left", "A"), ("steer_right", "D"),
        ("a", "LShift"), ("x", "T"), ("b", "LCtrl"), ("y", "E"), ("hippie", "Q"),
        ("lt", "Z"), ("rt", "C"), ("manual", "1"), ("nose_manual", "2"),
        ("lb", "X"), ("rb", "B"), ("rs", "Mouse3"),
        ("marker_place", "N"), ("marker_return", "J"),
        ("retry", "Comma"), ("object_dropper", "Period"),
    ];
    match name {
        "standard" => bindings.extend([
            ("pop", "Space"), ("trick_up", "Up"), ("trick_down", "Down"),
            ("trick_left", "Left"), ("trick_right", "Right"),
        ]),
        "skate" => {
            bindings.retain(|(id, _)| !matches!(*id, "a" | "x" | "marker_return" | "lt" | "rt" | "manual" | "nose_manual"));
            bindings.extend([("a", "Space"), ("x", "LShift"), ("marker_return", "5"),
                ("lt", "U"), ("rt", "O"), ("manual", "H"), ("nose_manual", "P"),
                ("trick_up", "I"), ("trick_down", "K"), ("trick_left", "J"), ("trick_right", "L")]);
        }
        "mouse" => {
            c.mouse = skate_input::MouseTrick::Hold;
            // A keyboard clutch works on a normal two-button mouse too.
            bindings.push(("mouse_trick", "CapsLock"));
        }
        "arcade" => {
            c.modern = true;
            bindings.extend([("ollie", "Space"), ("nollie", "K"),
                ("kickflip", "U"), ("heelflip", "O"), ("shove_back", "H"),
                ("shove_front", "L"), ("flip_360", "P")]);
        }
        _ => return Err(format!("no layout {name}")),
    }
    for (id, key) in bindings {
        c.bind(Controls::action_index(id).expect("known action"), key)?;
    }
    Ok(c)
}

pub fn default_controls() -> Controls { preset("standard").unwrap() }
pub fn skate_controls() -> Controls { preset("skate").unwrap() }

/// Explicit menu commands only; loading a user's file never selects a preset.
/// Keep the original layout once, plus an explicit personal save slot.
pub fn layout(name: &str) -> Result<(), String> {
    let mut guard = KEYBOARD.lock().map_err(|_| "keyboard state poisoned".to_string())?;
    let k = guard.as_mut().ok_or("the keyboard is not configured")?;
    let custom = k.path.with_extension("custom.txt");
    if name == "save_custom" {
        return std::fs::write(custom, k.controls.to_text()).map_err(|e| e.to_string());
    }
    let controls = if name == "custom" || name == "original" {
        let path = if name == "custom" { custom } else { k.path.with_extension("before-presets.txt") };
        let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
        let (controls, problems) = Controls::from_text(&text);
        if !problems.is_empty() { return Err(problems.join("; ")); }
        controls
    } else { preset(name)? };
    let backup = k.path.with_extension("before-presets.txt");
    if !backup.exists() {
        std::fs::copy(&k.path, backup).map_err(|e| e.to_string())?;
    }
    std::fs::write(&k.path, controls.to_text()).map_err(|e| e.to_string())?;
    k.controls = controls;
    k.translator = Translator::new();
    Ok(())
}

/// Mouse ownership is reported even while the trick stick is centered.
pub fn mouse_tricks() -> bool {
    KEYBOARD.lock().ok().and_then(|k| k.as_ref().map(|k| k.translator.mouse_tricks())).unwrap_or(false)
}

/// Loads the keys from `path`, writing the defaults there when it does not
/// exist. Returns problems found in the file (unknown keys or actions).
pub fn configure(path: PathBuf) -> Vec<String> {
    let (controls, problems) = match std::fs::read_to_string(&path) {
        Ok(text) => {
            Controls::from_text(&text)
        }
        Err(_) => {
            let controls = default_controls();
            let _ = std::fs::write(&path, controls.to_text());
            (controls, Vec::new())
        }
    };
    if let Ok(mut k) = KEYBOARD.lock() {
        *k = Some(Keyboard { controls, translator: Translator::new(), path: path.clone() });
    }
    problems
}

fn key_down(name: &str) -> bool {
    skate_input::keys::virtual_key(name).is_some_and(|vk| (unsafe { GetAsyncKeyState(vk as i32) } as u16) & 0x8000 != 0)
}

/// This frame's pad from the keyboard and mouse (`mouse`: the frame's look
/// input, x right and y down), or none when nothing is configured. `active`
/// false (the game has no focus, a menu is open) releases everything.
pub fn pad(mouse: [f32; 2], dt: f32, active: bool, allow_mouse: bool) -> Option<skate_input::Pad> {
    let mut guard = KEYBOARD.lock().ok()?;
    let k = guard.as_mut()?;
    if !active {
        k.translator.reset();
        return None;
    }
    if allow_mouse {
        Some(k.translator.update(&k.controls, key_down, mouse, dt))
    } else {
        let mut controls = k.controls.clone();
        controls.mouse = skate_input::MouseTrick::Off;
        Some(k.translator.update(&controls, key_down, [0., 0.], dt))
    }
}

/// Whether a pad from `pad` does anything (a key held or the stick moved).
/// The keys' hints for the HUD, one "key	what" line each; empty before
/// `configure`.
pub fn hints(on_foot: bool) -> String {
    let Ok(guard) = KEYBOARD.lock() else { return String::new() };
    let Some(k) = guard.as_ref() else { return String::new() };
    skate_input::hints::keyboard(&k.controls, on_foot)
        .into_iter()
        .map(|(key, what)| format!("{key}	{what}
"))
        .collect()
}

/// The key bound to action `id` (by `Controls::ACTIONS` id), "" when none.
pub fn key_of(id: &str) -> String {
    let Ok(guard) = KEYBOARD.lock() else { return String::new() };
    let (Some(k), Some(i)) = (guard.as_ref(), Controls::action_index(id)) else { return String::new() };
    k.controls.keys.get(i).and_then(|keys| keys.first()).map(|k| k.to_string()).unwrap_or_default()
}

/// Binds Windows virtual key `vk` to action `id` alone (taking it from any
/// other action) and saves the key file. The key's name, or an error.
pub fn bind(id: &str, vk: u32) -> Result<String, String> {
    let name = skate_input::keys::from_virtual_key(vk).ok_or_else(|| format!("key {vk:#x} has no name"))?;
    let mut guard = KEYBOARD.lock().map_err(|_| "keyboard state poisoned".to_string())?;
    let k = guard.as_mut().ok_or("the keyboard is not configured")?;
    let index = Controls::action_index(id).ok_or_else(|| format!("no action {id}"))?;
    if matches!(name, "F" | "G" | "M" | "R" | "V" | "Tab" | "Mouse1" | "Mouse2" |
        "Escape" | "Enter" | "LAlt" | "RAlt" | "F5" | "F6" | "F7" | "F8" | "F10") ||
        (name == "E" && id != "y") {
        return Err(format!("{name} is reserved for GTA / the package"));
    }
    let mut controls = k.controls.clone();
    controls.bind(index, name)?;
    match skate_input::ACTIONS[index].0 {
        skate_input::Action::Gesture(_) => controls.modern = true,
        skate_input::Action::MouseTrick => controls.mouse = skate_input::MouseTrick::Hold,
        skate_input::Action::Trick(_) | skate_input::Action::Manual | skate_input::Action::NoseManual => controls.classic = true,
        _ => {}
    }
    std::fs::write(&k.path, controls.to_text()).map_err(|e| e.to_string())?;
    k.controls = controls;
    k.translator.reset();
    Ok(name.to_string())
}

pub fn is_active(p: &skate_input::Pad) -> bool {
    p.buttons != 0 || p.triggers != [0, 0] || p.left.iter().chain(&p.right).any(|v| v.abs() > 0.01)
}

#[cfg(test)]
mod tests {
    use super::*;
    static TEST_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn loading_preserves_personal_keys() {
        let _serial = TEST_LOCK.lock().unwrap();
        let path = std::env::temp_dir().join(format!("skatev-keys-old-{}.txt", std::process::id()));
        let mut old = default_controls();
        let index = |id: &str| Controls::action_index(id).unwrap();
        old.bind(index("object_dropper"), "Y").unwrap();
        old.bind(index("marker_return"), "M").unwrap();
        std::fs::write(&path, old.to_text()).unwrap();
        configure(path.clone());
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(text, old.to_text());
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn the_skate_layout_puts_the_trick_stick_on_ijkl_and_uses_each_key_once() {
        let c = skate_controls();
        let key = |id: &str| c.keys[Controls::action_index(id).unwrap()].clone();
        assert_eq!((key("trick_up"), key("trick_left"), key("trick_down"), key("trick_right")), (vec!["I"], vec!["J"], vec!["K"], vec!["L"]));
        assert_eq!(key("a"), vec!["Space"]);
        assert!(key("pop").is_empty());
        let mut seen = std::collections::HashSet::new();
        for keys in &c.keys {
            for k in keys {
                assert!(seen.insert(*k), "{k} bound twice");
            }
        }
    }

    #[test]
    fn a_key_is_rebound_and_saved() {
        let _serial = TEST_LOCK.lock().unwrap();
        let path = std::env::temp_dir().join(format!("skatev-keys-bind-{}.txt", std::process::id()));
        let _ = std::fs::remove_file(&path);
        configure(path.clone());
        assert_eq!(bind("hippie", 0x35).unwrap(), "5"); // VK 5
        assert_eq!(key_of("hippie"), "5");
        assert!(std::fs::read_to_string(&path).unwrap().contains("bind hippie = 5"));
        assert!(bind("no_such_action", 0x47).is_err());
    }

    #[test]
    fn the_hints_name_the_default_keys() {
        let _serial = TEST_LOCK.lock().unwrap();
        configure(std::env::temp_dir().join(format!("skatev-keys-{}.txt", std::process::id())));
        let riding = hints(false);
        assert!(riding.contains("Space	Hold: crouch, let go: ollie"), "{riding}");
        assert!(riding.contains("LShift	Push"), "{riding}");
        assert!(riding.contains("E	Get off"), "{riding}");
        assert!(hints(true).contains("E	Get on the board"));
    }

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
    #[test]
    fn all_presets_are_unique_roundtrip_and_reserve_gta_keys() {
        for name in ["standard", "skate", "mouse", "arcade"] {
            let c = preset(name).unwrap();
            let (back, errors) = Controls::from_text(&c.to_text());
            assert!(errors.is_empty(), "{name}: {errors:?}");
            assert_eq!(back, c);
            let mut seen = std::collections::HashSet::new();
            for (i, keys) in c.keys.iter().enumerate() {
                for key in keys {
                    assert!(c.uses(skate_input::ACTIONS[i].0), "{name}: inactive {key}");
                    assert!(seen.insert(*key), "{name}: duplicate {key}");
                    assert!(!["F", "G", "M", "R", "V", "Tab", "Mouse1", "Mouse2", "LAlt", "F10"].contains(key));
                }
            }
            for id in ["a", "x", "b", "y", "hippie", "lt", "rt", "lb", "rb", "rs",
                "manual", "nose_manual", "marker_place", "marker_return", "retry", "object_dropper"] {
                assert_eq!(c.keys[Controls::action_index(id).unwrap()].len(), 1, "{name}: {id}");
            }
        }
    }

    #[test]
    fn presets_drive_real_pad_actions_and_mouse_ownership() {
        for name in ["standard", "skate", "mouse", "arcade"] {
            let c = preset(name).unwrap();
            let key = c.keys[Controls::action_index("a").unwrap()][0];
            let mut t = Translator::new();
            assert_ne!(t.update(&c, |k| k == key, [0.,0.], 0.016).buttons & 0x1000, 0);
            assert!(t.update(&c, |_| false, [1.,1.], 0.1).right.iter().all(|v| v.abs() < 0.001));
        }
        let c = preset("mouse").unwrap();
        let mut t = Translator::new();
        let down = t.update(&c, |k| k == "CapsLock", [0., 100.], 0.016);
        assert!(t.mouse_tricks() && down.right[1] < -0.5);
        t.update(&c, |_| false, [0.,0.], 0.016);
        assert!(!t.mouse_tricks());
        let again = t.update(&c, |k| k == "CapsLock", [0.,0.], 0.1);
        assert_eq!(again.right, [0.,0.], "clutch must not reuse the last stroke");
        t.reset();
        assert!(!t.mouse_tricks());
    }

    #[test]
    fn preset_switch_preserves_original_and_personal_save_and_rejects_reserved_keys() {
        let _serial = TEST_LOCK.lock().unwrap();
        let path = std::env::temp_dir().join(format!("skatev-presets-{}.txt", std::process::id()));
        for p in [&path, &path.with_extension("custom.txt"), &path.with_extension("before-presets.txt")] {
            let _ = std::fs::remove_file(p);
        }
        let mut original = default_controls();
        original.bind(Controls::action_index("hippie").unwrap(), "6").unwrap();
        std::fs::write(&path, original.to_text()).unwrap();
        configure(path.clone());
        layout("save_custom").unwrap();
        layout("mouse").unwrap();
        layout("arcade").unwrap();
        assert_eq!(std::fs::read_to_string(path.with_extension("before-presets.txt")).unwrap(), original.to_text());
        layout("custom").unwrap();
        assert_eq!(key_of("hippie"), "6");
        layout("original").unwrap();
        let before = std::fs::read_to_string(&path).unwrap();
        for vk in [0x46,0x47,0x4d,0x52,0x56,0x09,0x01,0x02,0x45] {
            assert!(bind("hippie", vk).is_err());
        }
        assert!(layout("unknown").is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), before);
        for p in [&path, &path.with_extension("custom.txt"), &path.with_extension("before-presets.txt")] {
            let _ = std::fs::remove_file(p);
        }
    }

}
