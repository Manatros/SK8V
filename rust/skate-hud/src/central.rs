//! Skate 3's centre-screen HUD message (the APT `hudcentral` movie): a plain
//! message, or the "celebratory" one with glow, flare and crown, for the
//! collectibles (design 2026-11-02-collectible-popups). Same VM and natives as
//! the Hall of Meat movie, plus `HUDComponents.Central_GetMessageData` and
//! `Central_GetAudioHook` from [`Message`].
use crate::apt_vm::Value;
use crate::layout::Layout;
use crate::movie::MoviePlayer;
use crate::player::{Assets, Frame};

/// `hudcentral.SetState` arguments (movie script).
mod state {
    pub const SHOW_MESSAGE: i32 = 4;
    pub const HIDE_MESSAGE: i32 = 5;
    pub const SHOW_CELEBRATORY: i32 = 17;
}

/// Longest message kept (the movie lays text out on one line).
pub const MAX_CHARS: usize = 40;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Style {
    /// White text, the default sound hook.
    Message,
    /// Glow, flare and crown, the success sound hook.
    Celebratory,
}

/// What the natives return: `[text, colour, x, y]` and the sound hook.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Message {
    pub text: String,
    /// 0 off-white (1 would be red; other values the script hands to
    /// ShowCelebratory).
    pub colour: i32,
    pub hook: f64,
}

/// One upper-case line of at most [`MAX_CHARS`] characters, as Skate's own
/// centre messages are; the lines of `text` join with ": " (the VM's text
/// layout has no line breaks).
pub fn message_text(text: &str) -> String {
    let line = text.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>().join(": ");
    line.to_uppercase().chars().take(MAX_CHARS).collect()
}

pub struct CentralPlayer {
    core: MoviePlayer,
    controller: usize,
}

impl CentralPlayer {
    pub fn new(assets: &Assets) -> Result<Self, String> {
        let core = MoviePlayer::new(assets, "hudcentral")?;
        let Value::Object(controller) = core.vm.get(core.bindings.movie.root, "screen") else {
            return Err("hudcentral controller was not constructed".into());
        };
        let mut player = Self { core, controller };
        player.call("Show", vec![Value::Bool(true)])?;
        Ok(player)
    }

    fn call(&mut self, method: &str, args: Vec<Value>) -> Result<(), String> {
        let core = &mut self.core;
        core.vm.begin_update();
        core.vm.call_method(self.controller, method, args, &mut core.bindings)?;
        core.drain()
    }

    /// The movie's own audio hook constant (`hudcentral.kAUDIOHOOK_*`), 0 if
    /// the class does not define it.
    fn hook(&self, name: &str) -> f64 {
        let vm = &self.core.vm;
        match vm.get(vm.global, "hudcentral") {
            Value::Object(class) => vm.get(class, name).number(),
            _ => 0.0,
        }
        .max(0.0)
    }

    /// Shows `text` (see [`message_text`]) in `style`; a newer message
    /// replaces the one on screen.
    pub fn show(&mut self, text: &str, style: Style) -> Result<(), String> {
        let (state, hook) = match style {
            Style::Message => (state::SHOW_MESSAGE, "kAUDIOHOOK_DEFAULT"),
            Style::Celebratory => (state::SHOW_CELEBRATORY, "kAUDIOHOOK_SUCCESS"),
        };
        let hook = self.hook(hook);
        self.core.bindings.central = Message { text: message_text(text), colour: 0, hook: if hook.is_finite() { hook } else { 0.0 } };
        self.call("SetState", vec![Value::Number(state as f64)])
    }

    /// Plays the outro of both message kinds.
    pub fn hide(&mut self) -> Result<(), String> {
        self.call("SetState", vec![Value::Number(state::HIDE_MESSAGE as f64)])
    }

    /// One timeline frame.
    pub fn advance(&mut self) -> Result<(), String> {
        self.core.advance()
    }

    /// The movie's scene in `layout`'s pixels (its own centred placement).
    pub fn frame(&self, assets: &Assets, layout: &Layout) -> Result<Frame, String> {
        self.core.native_frame(assets, layout)
    }

    pub fn visible_text(&self) -> Vec<String> {
        self.core.visible_text()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_are_one_short_upper_case_line() {
        assert_eq!(message_text("Deck unlocked\nBusenitz Seizures"), "DECK UNLOCKED: BUSENITZ SEIZURES");
        assert_eq!(message_text("a\n\n b \nc"), "A: B: C");
        assert_eq!(message_text(&"x".repeat(50)), "X".repeat(MAX_CHARS));
        assert_eq!(message_text(""), "");
    }

    /// Plays both styles on the prepared movie and writes previews to
    /// out/tests/central/ (needs `SKATEV_HUD_CENTRAL` = the prepared
    /// private/hud-central folder; the movie comes from the player's own game).
    #[test]
    #[ignore = "prepared hudcentral movie required (SKATEV_HUD_CENTRAL)"]
    fn prepared_movie_shows_both_styles() {
        let dir = std::path::PathBuf::from(std::env::var("SKATEV_HUD_CENTRAL").expect("prepared hud-central folder"));
        let assets = Assets::load_central(&dir).unwrap();
        let layout = Layout::compute(1920, 1080, 1.0, crate::layout::DEFAULT_MAX_ASPECT);
        let textures: Vec<_> = assets.textures.iter().map(crate::raster::LinearTexture::from).collect();
        let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../out/tests/central");
        std::fs::create_dir_all(&out).unwrap();
        for (style, text, name) in [
            (Style::Message, "Hidden deck found", "message"),
            (Style::Celebratory, "Deck unlocked\nBusenitz Seizures", "celebratory"),
        ] {
            let mut p = CentralPlayer::new(&assets).unwrap();
            p.show(text, style).unwrap();
            let (mut drawn, mut text) = (0, Vec::new());
            for f in 0..160 {
                // The outro after a second; nothing (no placeholder) may stay.
                if f == 60 {
                    text = p.visible_text();
                    p.hide().unwrap();
                }
                p.advance().unwrap();
                let frame = p.frame(&assets, &layout).unwrap();
                drawn = drawn.max(frame.draws.len());
                if [10, 30, 59, 70, 159].contains(&f) {
                    let image = crate::raster::render_target(&frame, &textures);
                    let png = std::fs::File::create(out.join(format!("{name}-{f:03}.png"))).unwrap();
                    let mut e = png::Encoder::new(std::io::BufWriter::new(png), image.width, image.height);
                    e.set_color(png::ColorType::Rgba);
                    e.set_depth(png::BitDepth::Eight);
                    e.write_header().unwrap().write_image_data(&image.to_rgba8()).unwrap();
                }
            }
            let last = p.frame(&assets, &layout).unwrap().draws.len();
            eprintln!("{name}: up to {drawn} draws, {last} after the outro, text {text:?} then {:?}", p.visible_text());
            assert!(drawn > 0, "{name} drew nothing");
            assert!(text.iter().any(|t| t.contains("UNLOCKED") || t.contains("FOUND")), "{name}: {text:?}");
            assert_eq!(last, 0, "{name}: something stays after the outro: {:?}", p.visible_text());
        }
    }
}
