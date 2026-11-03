//! The board shop (contributed by GTAxSkate): Skate 3's complete boards and
//! every deck, truck, wheel, griptape and paint for sale in GTA's street
//! clothing stores (`shop`), a garage per character, and the part models
//! (tools/prepare-board-parts.py) that show the ridden board. The host
//! draws the menu, takes the money and places the models; this keeps the
//! catalog, the garage and what to show.

use std::collections::HashSet;
use std::ffi::{c_char, c_void};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use skate_boards::{BoardChoice, Library, Part, PartRef};

use crate::shop::{Action, Garage, Goods, Input, Line, Menu, Tier, VISIBLE};
use crate::SvBoardPose;

/// The plain silver half of a truck whose other half is painted.
const TRUCK_PLAIN: &str = "greyg";
/// How long a message ("Bought ...") replaces the help line.
const MESSAGE_FRAMES: u32 = 240;

pub struct BoardShop {
    library: Library,
    /// The part models that were built (file stems).
    available: HashSet<String>,
    /// The board bones' bind matrices in object space (host row-vector layout).
    binds: [[f32; 16]; 7],
    /// Where the garages are kept: `SkateVBoards-<who>.txt`.
    garages: PathBuf,
    who: String,
    garage: Garage,
    menu: Option<Menu>,
    message: String,
    message_frames: u32,
    /// Where each store shows its boards (kept in `SkateVDisplay.txt`).
    shows: Vec<crate::shop::ShowCase>,
    /// Searches for a board shop's front that found nothing, per store.
    misses: Vec<((i32, i32), u32)>,
    /// Stores shown at a fixed front this session (never saved).
    fixed: Vec<(i32, i32)>,
    /// What the last search for a board shop's front did, for the host's
    /// log (`sv_shop_text` 9, cleared when read).
    note: String,
    /// The part exporter's board frame to the native board's object space
    /// (prepare-board-parts.py's ROTATION and offset).
    frame: bevy_math::Mat4,
    /// How worn the character's parts are, and the riding not yet counted.
    wear: skate_boards::wear::Wear,
    riding: Riding,
    /// Decks and griptapes found as collectibles (secrets.rs): locked ones
    /// stay out of menus, walls and crew boards.
    secrets: crate::secrets::Secrets,
}

/// Riding as it happens, from Skate's state each frame: what wears the
/// board (skate_boards::wear::Usage), counted into the wear every 2 s.
#[derive(Default)]
struct Riding {
    usage: skate_boards::wear::Usage,
    seconds: f32,
    air: bool,
    wipeout: bool,
}

impl Riding {
    /// One frame in Skate state `state` at `speed` m/s; true every 2 s of riding.
    fn frame(&mut self, state: &str, speed: f32, dt: f32) -> bool {
        // "Hit": a blow struck with the board in hand (counted at once).
        if state == "Hit" {
            self.usage.hits += 1;
            return true;
        }
        if !dt.is_finite() || dt <= 0.0 || !speed.is_finite() {
            return false;
        }
        let dt = dt.min(0.25);
        let u = &mut self.usage;
        let on_board = !state.starts_with("Biped") && !state.starts_with("Wipeout") && !state.starts_with("Teleport");
        if on_board && !state.contains("Air") {
            u.rolled += speed * dt;
        }
        if state.starts_with("Grind") {
            u.truck_grind += dt;
        } else if state.contains("Powerslide") {
            u.powerslide += dt;
        } else if state.contains("Slide") {
            u.deck_slide += dt;
        }
        let air = state.contains("Air");
        if self.air && !air && on_board {
            u.landings += 1;
        }
        self.air = air;
        let wipeout = state.starts_with("Wipeout");
        if wipeout && !self.wipeout {
            u.bails += 1;
        }
        self.wipeout = wipeout;
        self.seconds += dt;
        self.seconds >= 2.0
    }
}

/// The boards a store shows: up to this many.

/// What an input did, for the host: it takes the money for a purchase.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SvShopResult {
    /// 0 nothing, 1 closed, 2 bought (charge `price`), 3 now riding an owned
    /// part, 4 back to the default board, 5 too little money.
    pub kind: u32,
    pub price: u32,
}

/// `$1,234`.
pub fn money(value: u32) -> String {
    let digits = value.to_string();
    let mut out = String::from("$");
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// The part models that make `garage`'s board: deck, trucks (or painted
/// baseplate and hanger), wheels and the griptape laid on the deck.
pub fn models_of(riding: &BoardChoice, grip: Option<&str>, base: Option<&str>, hanger: Option<&str>) -> Vec<String> {
    let named = |letter: &str, part: Option<&PartRef>| match part {
        Some(p) => format!("skatev_{letter}_{}", p.material.to_ascii_lowercase()),
        None => format!("skatev_{letter}_default"),
    };
    let deck = riding.get(Part::Deck);
    let mut out = vec![named("d", deck)];
    if base.is_some() || hanger.is_some() {
        out.push(format!("skatev_tb_{}", base.unwrap_or(TRUCK_PLAIN)));
        out.push(format!("skatev_th_{}", hanger.unwrap_or(TRUCK_PLAIN)));
    } else {
        out.push(named("t", riding.get(Part::Trucks)));
    }
    out.push(named("w", riding.get(Part::Wheels)));
    if let Some(id) = grip {
        out.push(skate_boards::grips::grip_model(deck, id).replacen("sk3_", "skatev_", 1));
    }
    out
}

/// The models a collectible shows for `goods`: the deck alone, a griptape on
/// the default deck (deck first, the grip last, as `models_of`), or the
/// trucks or wheels alone (collectibles v3).
pub fn pickup_models(goods: &Goods) -> Vec<String> {
    let mut garage = Garage::default();
    garage.ride(goods);
    let all = models_of(&garage.riding, garage.grip.as_deref(), None, None);
    match goods {
        Goods::Part(Part::Trucks, _) => return vec![all[1].clone()],
        Goods::Part(Part::Wheels, _) => return vec![all[2].clone()],
        _ => {}
    }
    let mut out = vec![all[0].clone()];
    if matches!(goods, Goods::Grip(_)) && all.len() > 3 {
        out.push(all[all.len() - 1].clone());
    }
    out
}

/// Where the stores' shows are kept, in the garages' folder.
const DISPLAY_FILE: &str = "SkateVDisplay.txt";
/// The section of found collectibles (design 2026-11-02-skate-shop-consistency).
pub const FOUND: &str = "Found";

/// The exporter's board frame (length +Z from the grip end, deck facing -X)
/// to the native board's object space: rows (0 1 0), (0 0 -1), (-1 0 0),
/// then `offset`.
pub fn exporter_frame(offset: bevy_math::Vec3) -> bevy_math::Mat4 {
    use bevy_math::{Vec3, Vec4};
    bevy_math::Mat4::from_cols(
        Vec4::new(0.0, 0.0, -1.0, 0.0),
        Vec4::new(1.0, 0.0, 0.0, 0.0),
        Vec4::new(0.0, -1.0, 0.0, 0.0),
        Vec3::ZERO.extend(1.0) + offset.extend(0.0),
    )
}

/// Row-vector `a` then `b` (4x4, rows X, Y, Z, translation).
fn compose(a: &[f32; 16], b: &[f32; 16]) -> [f32; 16] {
    let mut o = [0.0; 16];
    for row in 0..4 {
        for c in 0..4 {
            o[row * 4 + c] = (0..4).map(|k| a[row * 4 + k] * b[k * 4 + c]).sum();
        }
    }
    o
}

/// The board posed at rest at `at` (its entity matrix): every bone at its bind.
pub fn rest_pose(binds: &[[f32; 16]; 7], at: &[f32; 16]) -> SvBoardPose {
    let mut world = [[0.0; 16]; 7];
    for (w, bind) in world.iter_mut().zip(binds) {
        *w = compose(bind, at);
    }
    SvBoardPose { size: std::mem::size_of::<SvBoardPose>() as u32, bone_count: 7, tick: 0, entity: *at, world }
}

impl BoardShop {
    /// `assets`: SK8V's data root (board library, private\board-parts).
    pub fn start(assets: &Path, garages: &Path) -> Result<Self, String> {
        let library = Library::load(assets)?;
        let parts = assets.join("private").join("board-parts").join("parts.json");
        let text = std::fs::read_to_string(&parts).map_err(|e| format!("cannot read {}: {e}", parts.display()))?;
        let json: serde_json::Value = serde_json::from_str(&text).map_err(|e| format!("{}: {e}", parts.display()))?;
        let available = json["parts"].as_array().into_iter().flatten().filter_map(|v| v.as_str().map(str::to_owned)).collect();
        let mut binds = [[0.0f32; 16]; 7];
        for (i, bind) in binds.iter_mut().enumerate() {
            for (j, v) in bind.iter_mut().enumerate() {
                *v = json["binds"][i][j].as_f64().ok_or_else(|| format!("{}: no bind matrices", parts.display()))? as f32;
            }
        }
        let offset = json["offset"].as_array().map(|o| {
            let v = |i: usize| o.get(i).and_then(serde_json::Value::as_f64).unwrap_or(0.0) as f32;
            bevy_math::Vec3::new(v(0), v(1), v(2))
        });
        let mut shop = Self::new(library, available, binds, garages);
        if let Some(offset) = offset {
            shop.frame = exporter_frame(offset);
        }
        Ok(shop)
    }

    pub fn new(library: Library, available: HashSet<String>, binds: [[f32; 16]; 7], garages: &Path) -> Self {
        Self {
            library,
            available,
            binds,
            garages: garages.to_owned(),
            who: String::new(),
            garage: Garage::default(),
            menu: None,
            message: String::new(),
            message_frames: 0,
            shows: std::fs::read_to_string(garages.join(DISPLAY_FILE))
                .or_else(|_| std::fs::read_to_string(garages.join("SkateGTA-display.txt")))
                .map(|t| crate::shop::shows_from_text(&t))
                .unwrap_or_default(),
            misses: Vec::new(),
            fixed: Vec::new(),
            note: String::new(),
            frame: exporter_frame(bevy_math::Vec3::new(0.0, 0.352, 0.064)),
            wear: Default::default(),
            riding: Default::default(),
            secrets: Default::default(),
        }
    }

    /// The secret items (design 2026-11-01-collectibles): the kept
    /// `SkateVSecrets.txt`, or `count` chosen now from what can be sold and
    /// kept. Returns how many are still locked.
    pub fn set_secrets(&mut self, count: usize) -> usize {
        let path = self.garages.join(crate::secrets::FILE);
        self.secrets = match std::fs::read_to_string(&path) {
            Ok(text) => crate::secrets::Secrets::from_text(&text),
            // Unreadable (locked, damaged): lock nothing and never write over it.
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Default::default(),
            Err(_) if count == 0 => Default::default(),
            Err(_) => {
                let keys: Vec<String> = crate::shop::stock(&self.library, Tier::Skate)
                    .into_iter()
                    .flat_map(|section| section.offers)
                    .filter(|offer| self.sells(&offer.goods))
                    .filter_map(|offer| crate::secrets::key(&offer.goods))
                    .collect();
                let chosen = crate::secrets::Secrets::choose(keys, count);
                let _ = std::fs::write(&path, chosen.to_text());
                chosen
            }
        };
        self.secrets.remaining()
    }

    fn save_secrets(&self) {
        let _ = std::fs::write(self.garages.join(crate::secrets::FILE), self.secrets.to_text());
    }

    /// The secret items in unlock order, one line each:
    /// `key \t title \t models` (comma-separated; a deck alone, a griptape
    /// on the default deck), for the collectibles that show them (design
    /// 2026-11-02-collectibles-v2).
    pub fn secret_list(&self) -> String {
        let offers: Vec<crate::shop::Offer> = crate::shop::stock(&self.library, Tier::Skate)
            .into_iter()
            .flat_map(|section| section.offers)
            .collect();
        let mut out = String::new();
        for key in self.secrets.order() {
            let offer = offers.iter().find(|o| crate::secrets::key(&o.goods).as_deref() == Some(key.as_str()));
            let title = offer.map_or_else(|| key.clone(), |o| o.title.clone());
            let models = offer.map(|o| pickup_models(&o.goods)).unwrap_or_default();
            out.push_str(&format!("{key}\t{title}\t{}\n", models.join(",")));
        }
        out
    }

    /// Unlocks `key` and keeps it: 1 unlocked now, 2 it already was, 0 no secret.
    pub fn unlock(&mut self, key: &str) -> u32 {
        let result = self.secrets.unlock(key);
        if result == 1 {
            self.save_secrets();
        }
        result
    }

    /// Locks every secret again and keeps it (the Developer reset): how many are locked.
    pub fn relock_all(&mut self) -> usize {
        self.secrets.relock_all();
        self.save_secrets();
        self.secrets.remaining()
    }

    /// Unlocks the next secret item and keeps it: its key and title.
    pub fn unlock_next(&mut self) -> Option<(String, String)> {
        let key = self.secrets.unlock_next()?;
        let _ = std::fs::write(self.garages.join(crate::secrets::FILE), self.secrets.to_text());
        let title = crate::shop::stock(&self.library, Tier::Skate)
            .into_iter()
            .flat_map(|section| section.offers)
            .find(|offer| crate::secrets::key(&offer.goods).as_deref() == Some(key.as_str()))
            .map_or_else(|| key.clone(), |offer| offer.title);
        Some((key, title))
    }

    /// Goods a collectible has not unlocked yet. The same for every
    /// character and for menus and walls alike; owned ones still show in
    /// "Yours".
    fn hidden(&self, goods: &Goods) -> bool {
        self.secrets.locked(goods)
    }

    /// Goods not sold or shown at a store of `tier`: a locked secret, and
    /// any secret outside the Skate Shop (design 2026-11-02-collectibles-v2,
    /// Manatros: "collected boards should be available at the dedicated
    /// skateshops"). Elsewhere a secret keeps its stand-in for good, so an
    /// unlock changes nothing there (A18).
    fn hidden_at(&self, goods: &Goods, tier: Tier) -> bool {
        self.secrets.locked(goods) || (tier != Tier::Skate && self.secrets.secret(goods))
    }

    fn board_hidden_at(&self, board: &BoardChoice, tier: Tier) -> bool {
        self.secrets.board_locked(board) || (tier != Tier::Skate && self.secrets.board_secret(board))
    }

    fn garage_path(&self) -> PathBuf {
        self.garages.join(format!("SkateVBoards-{}.txt", self.who))
    }

    /// Michael, Franklin and Trevor (or any other ped model) each keep their
    /// own boards. A garage from Skate 3 for GTA V (SkateGTA-boards) is taken over.
    pub fn character(&mut self, who: &str) {
        if who == self.who {
            return;
        }
        self.who = who.to_owned();
        self.menu = None;
        let own = self.garage_path();
        let text = std::fs::read_to_string(&own)
            .or_else(|_| std::fs::read_to_string(self.garages.join(format!("SkateGTA-boards-{who}.txt"))))
            .unwrap_or_default();
        self.garage = Garage::from_text(&text);
        let wear = std::fs::read_to_string(self.wear_path())
            .or_else(|_| std::fs::read_to_string(self.garages.join(format!("SkateGTA-wear-{who}.txt"))))
            .unwrap_or_default();
        self.wear = skate_boards::wear::Wear::from_text(&wear);
        self.riding = Riding::default();
    }

    fn wear_path(&self) -> PathBuf {
        self.garages.join(format!("SkateVWear-{}.txt", self.who))
    }

    /// One frame of skating (Skate's state, speed in m/s, seconds): wears
    /// the ridden parts; true when one wore through (its worn finish shows now).
    pub fn ride(&mut self, state: &str, speed: f32, dt: f32) -> bool {
        if self.who.is_empty() || !self.riding.frame(state, speed, dt) {
            return false;
        }
        let usage = std::mem::take(&mut self.riding.usage);
        self.riding.seconds = 0.0;
        let through = self.wear.ride(&self.garage.riding, &usage);
        let _ = std::fs::write(self.wear_path(), self.wear.to_text());
        through
    }

    /// The ridden board as its wear shows it: worn-through parts in Skate 3's
    /// worn finish.
    fn ridden(&self) -> Garage {
        let mut garage = self.garage.clone();
        let look = self.wear.look(&self.garage.riding, &self.library);
        for part in Part::ALL {
            // A worn finish without a built model keeps the new one.
            let letter = ["d", "t", "w"][part.index()];
            if let Some(worn) = look.get(part)
                && self.available.contains(&format!("skatev_{letter}_{}", worn.material.to_ascii_lowercase()))
            {
                garage.riding.set(part, Some(worn.clone()));
            }
        }
        garage
    }

    fn save(&self) {
        if !self.who.is_empty() {
            let _ = std::fs::write(self.garage_path(), self.garage.to_text());
        }
    }

    /// The models for `garage`, each one that was not built replaced by the
    /// default board's (a missing griptape is left off).
    fn models(&self, garage: &Garage) -> Vec<String> {
        models_of(&garage.riding, garage.grip.as_deref(), garage.truck_base.as_deref(), garage.truck_hanger.as_deref())
            .into_iter()
            .filter_map(|name| {
                if self.available.contains(&name) {
                    return Some(name);
                }
                let kind = name.split('_').nth(1).unwrap_or_default();
                let fallback = format!("skatev_{kind}_default");
                (kind != "g" && self.available.contains(&fallback)).then_some(fallback)
            })
            .collect()
    }

    fn is_default(garage: &Garage) -> bool {
        garage.riding == BoardChoice::default() && garage.grip.is_none() && garage.truck_base.is_none() && garage.truck_hanger.is_none()
    }

    /// The ridden board's models; none for the default board (the native
    /// board shows).
    pub fn riding(&self) -> Vec<String> {
        if Self::is_default(&self.garage) { Vec::new() } else { self.models(&self.ridden()) }
    }

    /// The ridden wheels' colour (the maintainer 2026-10-18: glowing wheels shine in
    /// their own colour by default): a paint's colour, else the mean of the
    /// wheel finish's opaque texture pixels.
    pub fn wheel_colour(&self) -> Option<[u8; 3]> {
        let part = self.garage.riding.get(Part::Wheels)?;
        if let Some(paint) = skate_boards::paints::of(part) {
            return Some(paint.rgb);
        }
        let texture = self.library.texture(&part.material, 64).ok()?;
        let (mut sum, mut n) = ([0u64; 3], 0u64);
        let (mut csum, mut cn) = ([0u64; 3], 0u64);
        for p in texture.rgba.chunks_exact(4).filter(|p| p[3] >= 128) {
            let [r, g, b, _] = [u64::from(p[0]), u64::from(p[1]), u64::from(p[2]), 0];
            let max = r.max(g).max(b);
            let min = r.min(g).min(b);
            let chroma = max - min;
            for c in 0..3 {
                sum[c] += u64::from(p[c]);
            }
            n += 1;
            if chroma >= 24 && max >= 40 {
                let w = chroma * chroma;
                for c in 0..3 {
                    csum[c] += u64::from(p[c]) * w;
                }
                cn += w;
            }
        }
        if cn > 0 {
            Some(csum.map(|s| (s / cn) as u8))
        } else {
            (n > 0).then(|| sum.map(|s| (s / n) as u8))
        }
    }

    /// The ridden wheels' hardness when they are painted (the paint's
    /// finish: matte soft, gloss medium, metallic hard); none for Skate's own.
    pub fn wheel_hardness(&self) -> Option<f32> {
        self.garage.riding.get(Part::Wheels).and_then(skate_boards::paints::of).map(|p| p.finish.hardness())
    }

    /// What the store shows: the ridden board with the current offer on it.
    pub fn preview(&self) -> Vec<String> {
        let mut garage = self.garage.clone();
        if let Some(offer) = self.menu.as_ref().and_then(Menu::current) {
            garage.ride(&offer.goods);
        }
        self.models(&garage)
    }

    /// Three rows each way in the active list, including its wraparound.
    /// Forward then backward at each distance; group rows have no models.
    /// Keep empty rows so the host can reduce the radius under session pressure.
    pub fn neighbours(&self) -> String {
        let Some(menu) = self.menu.as_ref() else { return String::new() };
        let lines = menu.lines();
        if lines.is_empty() { return String::new(); }
        let mut out = String::new();
        for distance in 1..=3 {
            for row in [(menu.row + distance) % lines.len(), (menu.row + lines.len() - distance % lines.len()) % lines.len()] {
                if row != menu.row {
                    if let Line::Offer(offer) = &lines[row] {
                        let mut garage = self.garage.clone();
                        garage.ride(&offer.goods);
                        out.push_str(&self.models(&garage).join(","));
                    }
                }
                out.push('\n');
            }
        }
        out
    }

    /// The store's part view: only the current offer's part (the deck with
    /// its griptape, the trucks, or the wheels); the whole board for a
    /// complete one.
    pub fn preview_part(&self) -> Vec<String> {
        let models = self.preview();
        let Some(part) = self.menu.as_ref().and_then(Menu::current).and_then(|o| crate::shop::focus_of(&o.goods)) else {
            return models;
        };
        let kinds: &[&str] = match part {
            Part::Deck => &["d", "g"],
            Part::Trucks => &["t", "tb", "th"],
            Part::Wheels => &["w"],
        };
        models.into_iter().filter(|m| kinds.contains(&m.split('_').nth(1).unwrap_or_default())).collect()
    }

    /// The side of the board the preview turns to the player for the offer
    /// at the cursor: "top" for a griptape, "bottom" for everything else (the
    /// graphic, the trucks and wheels), "" outside the shop's offers.
    pub fn preview_face(&self) -> &'static str {
        match self.menu.as_ref().and_then(Menu::current).map(|o| &o.goods) {
            Some(Goods::Grip(_)) => "top",
            Some(_) => "bottom",
            None => "",
        }
    }

    /// A complete board from a Suburban's stock for the player's crew (the
    /// AI skaters' friends), one per `slot`, the same each time: the models to
    /// show (only those built).
    pub fn crew_board(&self, slot: u32) -> Vec<String> {
        let completes: Vec<BoardChoice> = crate::shop::stock(&self.library, Tier::Mid)
            .into_iter()
            .flat_map(|section| section.offers)
            .filter(|offer| self.sells(&offer.goods))
            .filter_map(|offer| match offer.goods {
                Goods::Complete(board) => Some(board),
                _ => None,
            })
            .collect();
        let order = crate::shop::show_picks(&completes, (slot as i32 * 7919, 4049), 0, completes.len());
        let Some(board) = crate::secrets::substitute(&order, 1, |b| self.secrets.board_secret(b), |b| self.board_hidden_at(b, Tier::Mid)).into_iter().next() else {
            return Vec::new();
        };
        models_of(&board, None, None, None).into_iter().filter(|m| self.available.contains(m)).collect()
    }

    fn sells(&self, goods: &Goods) -> bool {
        match goods {
            // Stickers are decals the part models cannot draw yet.
            Goods::Sticker(_) => false,
            _ => {
                let mut garage = Garage::default();
                garage.ride(goods);
                models_of(&garage.riding, garage.grip.as_deref(), garage.truck_base.as_deref(), garage.truck_hanger.as_deref())
                    .iter()
                    .all(|m| self.available.contains(m))
            }
        }
    }

    /// Today's stock of the store `store` (design 2026-10-26-shops-step1):
    /// "Yours" (every owned board and part, to ride again), the complete
    /// boards on show (Binco all of them, Suburban the first 3), then a
    /// few of each part by store kind, different each game day.
    pub fn stock_for(&self, tier: Tier, store: (i32, i32), day: u32) -> Vec<crate::shop::Section> {
        let mut full = crate::shop::stock(&self.library, tier);
        for section in &mut full {
            section.offers.retain(|offer| self.sells(&offer.goods));
        }
        let mut out = Vec::new();
        // Owned goods keep their price: riding one is free, buying the one
        // ridden again is a new one and costs it (peer review 2026-10-26:
        // at 0 it renewed worn parts for nothing). Skate's own grip is no
        // possession of note.
        let yours: Vec<crate::shop::Offer> = full.iter().flat_map(|s| s.offers.iter())
            .filter(|o| self.garage.owns(&o.goods) && o.goods != Goods::Grip(crate::shop::DECK_GRIP.into()))
            .cloned()
            .collect();
        if !yours.is_empty() {
            out.push(crate::shop::Section { title: "Yours".into(), offers: yours });
        }
        // Collectibles: every secret found and not yet owned is on sale here
        // every day, outside the rotation (design 2026-11-02-skate-shop-consistency).
        if tier == Tier::Skate {
            let found: Vec<crate::shop::Offer> = full.iter().flat_map(|s| s.offers.iter())
                // Parts only (the decks and grips found; their complete
                // boards stay in the rotation and Suburban's limit).
                .filter(|o| !matches!(o.goods, Goods::Complete(_)))
                .filter(|o| self.secrets.secret(&o.goods) && !self.hidden(&o.goods) && !self.garage.owns(&o.goods))
                .cloned()
                .collect();
            if !found.is_empty() {
                out.push(crate::shop::Section { title: FOUND.into(), offers: found });
            }
        }
        for section in full {
            // Each section's daily order over everything sold; a locked secret
            // gives its slot to the next unlocked one (secrets.rs), so an
            // unlock changes no other slot (A18).
            let all = section.offers.len();
            let hidden = |offer: &crate::shop::Offer| self.hidden_at(&offer.goods, tier);
            let secret = |offer: &crate::shop::Offer| self.secrets.secret(&offer.goods);
            let offers = if section.title == "Complete boards" {
                // The first picks are the boards on show (one daily order):
                // Suburban sells those and a few more (Manatros 2026-10-26).
                let extra = if tier == Tier::Mid { crate::shop::MID_EXTRA } else { 0 };
                let order = crate::shop::picks_distinct(&section.offers, store, day, 0, all);
                crate::secrets::substitute(&order, crate::shop::on_show(tier) + extra, secret, hidden)
            } else {
                let salt = section.title.bytes().fold(0u32, |h, b| h.wrapping_mul(31).wrapping_add(u32::from(b)));
                let order = crate::shop::picks_distinct(&section.offers, store, day, salt, all);
                crate::secrets::substitute(&order, crate::shop::section_count(tier, &section.title), secret, hidden)
            };
            if !offers.is_empty() {
                out.push(crate::shop::Section { title: section.title, offers });
            }
        }
        out
    }

    /// Opens today's menu of store `store`; false where boards are not sold.
    pub fn open_at(&mut self, tier: Tier, store: (i32, i32), day: u32) -> bool {
        if !tier.sells_boards() {
            return false;
        }
        let sections = self.stock_for(tier, store, day);
        self.menu = (!sections.is_empty()).then(|| Menu::new(tier, sections));
        self.message.clear();
        self.menu.is_some()
    }

    /// Opens the store's menu; false where boards are not sold.
    pub fn open(&mut self, tier: Tier) -> bool {
        if !tier.sells_boards() {
            return false;
        }
        let mut sections = crate::shop::stock(&self.library, tier);
        for section in &mut sections {
            section.offers.retain(|offer| self.sells(&offer.goods) && !self.hidden_at(&offer.goods, tier));
        }
        sections.retain(|s| !s.offers.is_empty());
        self.menu = (!sections.is_empty()).then(|| Menu::new(tier, sections));
        self.message.clear();
        self.menu.is_some()
    }

    pub fn close(&mut self) {
        self.menu = None;
    }

    pub fn is_open(&self) -> bool {
        self.menu.is_some()
    }

    fn say(&mut self, text: String) {
        self.message = text;
        self.message_frames = MESSAGE_FRAMES;
    }

    /// One menu input; `cash` is what the player has.
    pub fn input(&mut self, input: Input, cash: i64) -> SvShopResult {
        let Some(menu) = self.menu.as_mut() else { return SvShopResult { kind: 1, price: 0 } };
        match menu.handle(input, &self.garage) {
            Action::None => SvShopResult::default(),
            Action::Close => {
                self.menu = None;
                SvShopResult { kind: 1, price: 0 }
            }
            Action::Buy(offer) => {
                if cash < i64::from(offer.price) {
                    self.say(format!("{} costs {}. You have {}.", offer.title, money(offer.price), money(cash.max(0) as u32)));
                    return SvShopResult { kind: 5, price: offer.price };
                }
                self.garage.add(&offer.goods);
                self.garage.ride(&offer.goods);
                // A new one: its wear starts over.
                match &offer.goods {
                    Goods::Part(_, part) => self.wear.renew(part),
                    Goods::Complete(board) => Part::ALL.iter().filter_map(|&p| board.get(p)).for_each(|part| self.wear.renew(part)),
                    _ => {}
                }
                let _ = std::fs::write(self.wear_path(), self.wear.to_text());
                self.save();
                self.say(format!("Bought {} for {}. Riding it now.", offer.title, money(offer.price)));
                SvShopResult { kind: 2, price: offer.price }
            }
            Action::Ride(offer) => {
                self.garage.ride(&offer.goods);
                self.save();
                self.say(format!("Riding {}.", offer.title));
                SvShopResult { kind: 3, price: 0 }
            }
            Action::RideDefault => {
                self.garage.ride_default();
                self.save();
                self.say("Riding the default board.".into());
                SvShopResult { kind: 4, price: 0 }
            }
        }
    }

    /// The menu as text for the host: line 1 the title and "n / total"
    /// (tab between), line 2 the help or the last message, then one line per
    /// row shown: label, value, and "1" on the selected row (tabs between).
    pub fn view(&mut self) -> String {
        let Some(menu) = self.menu.as_ref() else { return String::new() };
        let section = menu.sections.get(menu.section).map_or("", |s| s.title.as_str());
        let title = match &menu.group {
            Some((group, _)) => format!("{section}: {group}"),
            None => section.to_owned(),
        };
        let lines = menu.lines();
        let mut out = format!("{title}\t{} / {}\n", (menu.row + 1).min(lines.len()), lines.len());
        let help = if self.message_frames > 0 {
            self.message.clone()
        } else {
            format!(
                "{}  Enter: buy or ride  Arrows: browse ({} of {})  Backspace: back  Space: default board",
                menu.tier.store(),
                menu.section + 1,
                menu.sections.len()
            )
        };
        out.push_str(&help.replace(['\n', '\t'], " "));
        out.push('\n');
        let first = menu.first_visible();
        for (i, line) in lines.iter().enumerate().skip(first).take(VISIBLE) {
            let (label, value) = match line {
                Line::Group(name, count) => (name.clone(), format!("{count} >")),
                Line::Offer(offer) => {
                    let value = if self.garage.rides(&offer.goods) {
                        "Riding".to_owned()
                    } else if self.garage.owns(&offer.goods) {
                        "Owned".to_owned()
                    } else {
                        money(offer.price)
                    };
                    (offer.title.clone(), value)
                }
            };
            out.push_str(&format!("{}\t{value}\t{}\n", label.replace('\t', " "), u8::from(i == menu.row)));
        }
        self.message_frames = self.message_frames.saturating_sub(1);
        out
    }

    pub fn rest_pose(&self, at: &[f32; 16]) -> SvBoardPose {
        rest_pose(&self.binds, at)
    }

    /// The boards a store of `tier` shows (its interior at `interior`), the
    /// player's eye at `eye`, its clerk (place, forward) if one stands near,
    /// on game day `day`: each board's entity matrix (host layout) and part
    /// models. The place is found once per store with lines `probe` casts
    /// (from, to -> where it hits) and kept: behind the till when there is a
    /// clerk, else a free stretch of wall. None found yet: empty.
    pub fn display(
        &mut self,
        tier: Tier,
        interior: bevy_math::Vec3,
        eye: bevy_math::Vec3,
        clerk: Option<(bevy_math::Vec3, bevy_math::Vec3)>,
        day: u32,
        mut probe: impl FnMut(bevy_math::Vec3, bevy_math::Vec3) -> Option<bevy_math::Vec3>,
    ) -> Vec<([f32; 16], Vec<String>)> {
        use bevy_math::Vec3;
        if !tier.sells_boards() {
            return Vec::new();
        }
        let store = crate::shop::store_key(interior);
        let show = match self.shows.iter().find(|s| s.store == store) {
            Some(kept) => kept.clone(),
            None => {
                let till = clerk.and_then(|(at, forward)| crate::shop::wall_behind(at + Vec3::Z * 0.3, forward, &mut probe));
                let mut relaxed = false;
                let mut wall = |probe: &mut dyn FnMut(Vec3, Vec3) -> Option<Vec3>| {
                    if tier == Tier::Skate {
                        crate::shop::storefront(eye, probe).map(|(at, normal, r)| {
                            relaxed = r;
                            (at, normal)
                        })
                    } else {
                        crate::shop::display_wall(eye, probe)
                    }
                };
                let mut found = till.map(|w| (w, true)).or_else(|| wall(&mut probe).map(|w| (w, false)));
                let mut how = match found {
                    Some((_, true)) => "behind the till",
                    Some(_) if relaxed => "on the nearest front (relaxed search)",
                    _ => "on a wall (strict search)",
                };
                // The board shop has no clerk and may have tables before its
                // front: after 5 misses, with its ground streamed in, a fixed
                // front (design 2026-11-02-skate-shop-consistency).
                // Only misses on streamed ground count.
                if found.is_none() && tier == Tier::Skate && probe(eye, eye - Vec3::Z * 4.0).is_some() {
                    let misses = match self.misses.iter_mut().find(|m| m.0 == store) {
                        Some(m) => { m.1 += 1; m.1 }
                        None => { self.misses.push((store, 1)); 1 }
                    };
                    if misses == 1 {
                        let rays: Vec<String> = (0..24)
                            .map(|k| {
                                let dir = bevy_math::Vec2::from_angle(k as f32 / 24. * std::f32::consts::TAU).extend(0.);
                                probe(eye, eye + dir * 8.).map_or("-".into(), |h| format!("{:.2}", (h - eye).truncate().length()))
                            })
                            .collect();
                        self.note = format!("no storefront found from {:.2} {:.2} {:.2}; rays every 15 deg from east: {}", eye.x, eye.y, eye.z, rays.join(" "));
                    }
                    if misses >= 5 {
                        found = Some((crate::shop::fixed_storefront(eye, &mut probe), false));
                        how = "at the fixed storefront";
                    }
                }
                let Some(((at, normal), till)) = found else { return Vec::new() };
                let above = at + normal * 0.3;
                let floor = probe(above, above - Vec3::Z * 4.0).map_or(eye.z - 1.3, |p| p.z);
                if tier == Tier::Skate {
                    self.note = format!("storefront {how} at {:.2} {:.2} {:.2}, facing {:.2} {:.2}, floor {:.2}", at.x, at.y, at.z, normal.x, normal.y, floor);
                }
                let show = crate::shop::ShowCase { store, at, normal, floor, till };
                self.shows.push(show.clone());
                // A fixed front is not kept: each start searches for the real one first.
                if how != "at the fixed storefront" {
                    let _ = std::fs::write(self.garages.join(DISPLAY_FILE), crate::shop::shows_to_text(&self.shows.iter().filter(|s| !self.fixed.contains(&s.store)).cloned().collect::<Vec<_>>()));
                } else {
                    self.fixed.push(store);
                }
                show
            }
        };
        let completes: Vec<BoardChoice> = crate::shop::stock(&self.library, tier)
            .into_iter()
            .flat_map(|section| section.offers)
            .filter(|offer| self.sells(&offer.goods))
            .filter_map(|offer| match offer.goods {
                Goods::Complete(board) => Some(board),
                _ => None,
            })
            .collect();
        let order = crate::shop::picks_distinct(&completes, store, day, 0, completes.len());
        let boards = crate::secrets::substitute(&order, crate::shop::on_show(tier), |b| self.secrets.board_secret(b), |b| self.board_hidden_at(b, tier));
        let to_board = self.frame.inverse();
        show.places(boards.len())
            .into_iter()
            .zip(boards)
            .map(|(place, board)| {
                let models = models_of(&board, None, None, None).into_iter().filter(|m| self.available.contains(m)).collect();
                ((place * to_board).to_cols_array(), models)
            })
            .collect()
    }
}

static SHOP: Mutex<Option<BoardShop>> = Mutex::new(None);
/// What `sv_shop_start` found, for the host's log (`sv_shop_text` 3).
static STATUS: Mutex<String> = Mutex::new(String::new());

fn with<T>(fallback: T, f: impl FnOnce(&mut BoardShop) -> T) -> T {
    let mut shop = SHOP.lock().unwrap_or_else(|e| e.into_inner());
    match shop.as_mut() {
        Some(s) => f(s),
        None => fallback,
    }
}

/// Copies `text` (NUL-terminated) into `buf`; returns the bytes it needs.
fn give(text: &str, buf: *mut u8, cap: u32) -> u32 {
    let need = text.len() as u32 + 1;
    if !buf.is_null() && cap >= need {
        unsafe {
            std::ptr::copy_nonoverlapping(text.as_ptr(), buf, text.len());
            *buf.add(text.len()) = 0;
        }
    }
    need
}

/// Loads the board library and the built part models. `assets`: the data
/// root; `garages`: the folder for the garages (the game folder). Returns 1
/// when the shop is ready; the reason is logged otherwise.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sv_shop_start(rt: *mut c_void, assets: *const c_char, garages: *const c_char) -> u32 {
    crate::guard(0, || {
        if rt.is_null() {
            return 0;
        }
        let (Some(assets), Some(garages)) = (unsafe { crate::path_arg(assets) }, unsafe { crate::path_arg(garages) }) else {
            return 0;
        };
        match BoardShop::start(&assets, &garages) {
            Ok(shop) => {
                *STATUS.lock().unwrap_or_else(|e| e.into_inner()) = format!("{} part models", shop.available.len());
                *SHOP.lock().unwrap_or_else(|e| e.into_inner()) = Some(shop);
                1
            }
            Err(e) => {
                *STATUS.lock().unwrap_or_else(|e| e.into_inner()) = e;
                0
            }
        }
    })
}

/// Whose garage: "michael", "franklin", "trevor" or the ped model's hash.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sv_shop_character(_rt: *mut c_void, who: *const c_char) -> u32 {
    crate::guard(0, || {
        let who = unsafe { crate::str_arg(who) };
        if who.is_empty() || !who.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return 0;
        }
        with(0, |s| {
            s.character(&who);
            1
        })
    })
}

/// The secret decks and griptapes (design 2026-11-01-collectibles), right
/// after sv_shop_start: the kept list, or `count` chosen now. Returns how
/// many are still locked.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sv_shop_secrets(_rt: *mut c_void, count: u32) -> u32 {
    crate::guard(0, || with(0, |s| s.set_secrets(count as usize) as u32))
}

/// Unlocks the next secret item (a collectible was earned) and writes
/// "key\ttitle" to `buf`. Returns the bytes it needs, 0 when none is left.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sv_shop_unlock_next(_rt: *mut c_void, buf: *mut u8, cap: u32) -> u32 {
    crate::guard(0, || match with(None, BoardShop::unlock_next) {
        Some((key, title)) => give(&format!("{key}\t{title}"), buf, cap),
        None => 0,
    })
}

/// The secret items for the collectibles (`BoardShop::secret_list`).
/// Returns the bytes it needs (with the NUL), 0 without a shop.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sv_shop_secret_list(_rt: *mut c_void, buf: *mut u8, cap: u32) -> u32 {
    crate::guard(0, || match with(None, |s| Some(s.secret_list())) {
        Some(text) => give(&text, buf, cap),
        None => 0,
    })
}

/// Unlocks the secret `key`: 1 unlocked now, 2 it already was, 0 no secret.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sv_shop_unlock(_rt: *mut c_void, key: *const c_char) -> u32 {
    crate::guard(0, || {
        let key = unsafe { crate::str_arg(key) };
        with(0, |s| s.unlock(&key))
    })
}

/// Locks every secret again (the Developer reset): how many are locked.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sv_shop_relock_all(_rt: *mut c_void) -> u32 {
    crate::guard(0, || with(0, |s| s.relock_all() as u32))
}

/// Host store kinds: 0 Binco / Discount Store, 1 Suburban, 2 Ponsonbys
/// (sells none), 3 a board shop.
fn tier_of(tier: u32) -> Tier {
    match tier {
        0 => Tier::Low,
        1 => Tier::Mid,
        3 => Tier::Skate,
        _ => Tier::High,
    }
}

/// Opens the store menu with the whole catalog (older hosts). Returns 1 when open.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sv_shop_open(_rt: *mut c_void, tier: u32) -> u32 {
    crate::guard(0, || with(0, |s| u32::from(s.open(tier_of(tier)))))
}

/// Opens today's menu of the store at GTA point (`x`, `y`) (the interior's
/// location, or a board shop's anchor) on game day `day`: the same boards
/// as its display. Returns 1 when open.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sv_shop_open_at(_rt: *mut c_void, tier: u32, x: f32, y: f32, day: u32) -> u32 {
    crate::guard(0, || {
        let store = crate::shop::store_key(bevy_math::Vec3::new(x, y, 0.0));
        with(0, |s| u32::from(s.open_at(tier_of(tier), store, day)))
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn sv_shop_close(_rt: *mut c_void) {
    crate::guard((), || with((), BoardShop::close))
}

/// One input: 0 up, 1 down, 2 left, 3 right, 4 accept, 5 back, 6 default board.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sv_shop_input(_rt: *mut c_void, input: u32, cash: i64, out: *mut SvShopResult) -> u32 {
    crate::guard(0, || {
        let input = match input {
            0 => Input::Up,
            1 => Input::Down,
            2 => Input::Left,
            3 => Input::Right,
            4 => Input::Accept,
            5 => Input::Back,
            6 => Input::Reset,
            _ => return 0,
        };
        let result = with(SvShopResult { kind: 1, price: 0 }, |s| s.input(input, cash));
        if let Some(out) = unsafe { out.as_mut() } {
            *out = result;
        }
        1
    })
}

/// Text for the host (UTF-8, NUL-terminated, into `buf` when `cap` is
/// enough; returns the bytes needed): 0 the menu (see `BoardShop::view`),
/// 1 the ridden board's models (comma list; empty for the native board),
/// 2 the store preview's models, 3 what starting the shop found, 4 the
/// ridden painted wheels' hardness ("0.60"; empty for unpainted wheels), 5 the
/// store preview's models in the part view (only the offer's part), 6 the side
/// the preview shows ("top" for a griptape, "bottom" else, "" outside offers),
/// 8 neighbour model lists: +1, -1, +2, -2, +3, -3 rows (one comma list
/// per line, empty on group rows; wraps within the active list),
/// 100 + slot (0..7) a complete board for crew member `slot` (models).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sv_shop_text(_rt: *mut c_void, which: u32, buf: *mut u8, cap: u32) -> u32 {
    crate::guard(0, || {
        if which == 3 {
            return give(&STATUS.lock().unwrap_or_else(|e| e.into_inner()), buf, cap);
        }
        let text = with(String::new(), |s| match which {
            0 => s.view(),
            1 => s.riding().join(","),
            2 => s.preview().join(","),
            5 => s.preview_part().join(","),
            6 => s.preview_face().to_string(),
            8 => s.neighbours(),
            9 => std::mem::take(&mut s.note),
            100..=107 => s.crew_board(which - 100).join(","),
            4 => s.wheel_hardness().map(|h| format!("{h:.2}")).unwrap_or_default(),
            7 => s.wheel_colour().map(|[r, g, b]| format!("{r},{g},{b}")).unwrap_or_default(),
            _ => String::new(),
        });
        give(&text, buf, cap)
    })
}

/// The board at rest at `at` (16 floats, the entity matrix), for the store
/// preview's part models. Returns 1 when written.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sv_shop_rest_pose(_rt: *mut c_void, at: *const f32, out: *mut SvBoardPose) -> u32 {
    crate::guard(0, || {
        if at.is_null() || out.is_null() {
            return 0;
        }
        let mut m = [0.0f32; 16];
        m.copy_from_slice(unsafe { std::slice::from_raw_parts(at, 16) });
        if m.iter().any(|v| !v.is_finite()) {
            return 0;
        }
        with(0, |s| {
            unsafe { *out = s.rest_pose(&m) };
            1
        })
    })
}

/// A line the host casts for the store's show: from, to (3 floats each);
/// writes where it hits to `hit` and returns 1, or returns 0.
pub type SvProbe = unsafe extern "C" fn(ctx: *mut c_void, from: *const f32, to: *const f32, hit: *mut f32) -> u32;

/// One frame of skating for the board's wear: Skate's state (UTF-8), speed
/// (m/s), seconds; the state "Hit" counts a blow struck with the board. Returns 1 when a part wore through (the ridden models
/// changed: ask for them again with `sv_shop_text` 1).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sv_shop_ride(_rt: *mut c_void, state: *const c_char, speed: f32, dt: f32) -> u32 {
    crate::guard(0, || {
        let state = unsafe { crate::str_arg(state) };
        with(0, |s| u32::from(s.ride(&state, speed, dt)))
    })
}

/// The boards a store shows (see `BoardShop::display`). `interior`, `eye`:
/// 3 floats; `clerk`: 6 (place, forward) or null; `out`: room for `max`
/// matrices of 16 floats; `models`: one comma list per board, a line each.
/// Returns how many boards (0: none yet, or `models` too small).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sv_shop_display(
    _rt: *mut c_void,
    tier: u32,
    interior: *const f32,
    eye: *const f32,
    clerk: *const f32,
    day: u32,
    probe: Option<SvProbe>,
    ctx: *mut c_void,
    out: *mut f32,
    max: u32,
    models: *mut u8,
    cap: u32,
) -> u32 {
    crate::guard(0, || {
        let Some(probe) = probe else { return 0 };
        if interior.is_null() || eye.is_null() || out.is_null() {
            return 0;
        }
        let v3 = |p: *const f32| unsafe { bevy_math::Vec3::new(*p, *p.add(1), *p.add(2)) };
        let clerk = (!clerk.is_null()).then(|| (v3(clerk), v3(unsafe { clerk.add(3) })));
        let tier = tier_of(tier);
        let cast = |a: bevy_math::Vec3, b: bevy_math::Vec3| {
            let (from, to, mut hit) = ([a.x, a.y, a.z], [b.x, b.y, b.z], [0.0f32; 3]);
            (unsafe { probe(ctx, from.as_ptr(), to.as_ptr(), hit.as_mut_ptr()) } != 0).then(|| bevy_math::Vec3::from(hit))
        };
        let shown = with(Vec::new(), |s| s.display(tier, v3(interior), v3(eye), clerk, day, cast));
        let shown: Vec<_> = shown.into_iter().take(max as usize).collect();
        let text: String = shown.iter().map(|(_, m)| m.join(",") + "\n").collect();
        if give(&text, models, cap) > cap {
            return 0;
        }
        for (i, (m, _)) in shown.iter().enumerate() {
            unsafe { std::ptr::copy_nonoverlapping(m.as_ptr(), out.add(i * 16), 16) };
        }
        shown.len() as u32
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIBRARY: &str = r#"{"models":{
      "deckmodel":{"slot":"SkateBoard","name":"Deck","materials":["d1"],"scene":"x"},
      "truckmodel":{"slot":"SkateTruck","name":"Truck","materials":["t1"],"scene":"x"},
      "wheelmodel":{"slot":"SkateWheel","name":"Wheel","materials":["w1"],"scene":"x"}},
     "materials":{
      "d1":{"name":"Deck One","diffuse":"d.png"},
      "t1":{"name":"Truck One","diffuse":"t.png"},
      "w1":{"name":"Wheel One","diffuse":"w.png"}}}"#;

    fn shop(dir: &Path) -> BoardShop {
        let library = Library::from_json(Path::new("/x"), LIBRARY).unwrap();
        let available = ["skatev_d_default", "skatev_t_default", "skatev_w_default", "skatev_d_d1", "skatev_t_t1", "skatev_w_w1"]
            .into_iter()
            .map(str::to_owned)
            .collect();
        let mut identity = [0.0f32; 16];
        for i in 0..4 {
            identity[i * 5] = 1.0;
        }
        BoardShop::new(library, available, [identity; 7], dir)
    }

    fn dir_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("skatev-shop-{name}-{}", std::process::id()))
    }

    fn dir(name: &str) -> PathBuf {
        let d = dir_path(name);
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn neighbours_follow_the_active_list_and_close() {
        let d = dir("neighbours");
        let mut s = shop(&d);
        assert!(s.neighbours().is_empty());
        let offers = (0..9).map(|i| {
            let id = format!("prefetch{i}");
            s.available.insert(format!("skatev_d_{id}"));
            crate::shop::Offer {
                title: id.clone(), goods: Goods::Part(Part::Deck, PartRef::new("deckmodel", &id)),
                price: 5, preview: id, group: None,
            }
        }).collect();
        s.menu = Some(Menu::new(Tier::Low, vec![crate::shop::Section { title: "Test".into(), offers }]));
        s.menu.as_mut().unwrap().row = 4;
        let neighbours = s.neighbours();
        let rows: Vec<_> = neighbours.lines().collect();
        assert_eq!(rows.len(), 6);
        for (row, index) in rows.iter().zip([5, 3, 6, 2, 7, 1]) {
            assert_eq!(row.split(',').next().unwrap(), format!("skatev_d_prefetch{index}"));
        }
        s.menu.as_mut().unwrap().row = 0;
        assert!(s.neighbours().lines().nth(1).unwrap().starts_with("skatev_d_prefetch8,"));
        // Group headings are rows, not offers to register.
        for offer in &mut s.menu.as_mut().unwrap().sections[0].offers {
            offer.group = Some("Maker".into());
        }
        assert_eq!(s.neighbours(), "\n\n\n\n\n\n");
        s.input(Input::Accept, 0);
        assert!(s.neighbours().contains("skatev_d_prefetch1,"));
        s.close();
        assert!(s.neighbours().is_empty());
    }

    #[test]
    fn the_models_follow_the_parts_and_paints() {
        let mut board = BoardChoice::default();
        board.set(Part::Deck, Some(PartRef::new("deckmodel", "D1")));
        assert_eq!(models_of(&board, None, None, None), ["skatev_d_d1", "skatev_t_default", "skatev_w_default"]);
        assert_eq!(
            models_of(&board, Some("black"), Some("redg"), None),
            ["skatev_d_d1", "skatev_tb_redg", "skatev_th_greyg", "skatev_w_default", "skatev_g_deck_black"]
        );
    }

    #[test]
    fn buying_needs_the_money_rides_and_is_kept_per_character() {
        let d = dir("buy");
        let mut s = shop(&d);
        s.character("franklin");
        assert!(s.riding().is_empty(), "the default board is the native board");
        assert!(s.open(Tier::Low));
        assert_eq!(s.input(Input::Accept, 0).kind, 5, "no money, no board");
        let bought = s.input(Input::Accept, 1_000_000);
        assert_eq!(bought.kind, 2);
        assert!(bought.price > 0);
        assert_eq!(s.riding(), ["skatev_d_d1", "skatev_t_t1", "skatev_w_w1"]);
        s.character("michael");
        assert!(s.riding().is_empty(), "Michael has his own garage");
        s.character("franklin");
        assert_eq!(s.riding(), ["skatev_d_d1", "skatev_t_t1", "skatev_w_w1"]);
        assert!(d.join("SkateVBoards-franklin.txt").is_file());
    }

    #[test]
    fn an_old_garage_is_taken_over() {
        let d = dir("old");
        std::fs::write(d.join("SkateGTA-boards-trevor.txt"), "owned deck deckmodel:d1\nriding deckmodel:d1,,\n").unwrap();
        let mut s = shop(&d);
        s.character("trevor");
        assert_eq!(s.riding().first().map(String::as_str), Some("skatev_d_d1"));
    }

    #[test]
    fn a_secret_deck_stays_out_until_unlocked_and_is_kept() {
        let d = dir("secrets");
        let mut s = shop(&d);
        s.character("franklin");
        assert_eq!(s.set_secrets(1), 1, "the one deck is the one lockable item");
        let sold = |s: &mut BoardShop, tier: Tier| {
            assert!(s.open(tier));
            s.menu.as_ref().unwrap().sections.iter().flat_map(|x| x.offers.iter()).map(|o| o.title.clone()).collect::<Vec<_>>()
        };
        let before = sold(&mut s, Tier::Mid);
        assert!(!before.iter().any(|t| t.contains("Deck One")), "{before:?}");
        assert!(before.iter().any(|t| t.contains("Truck One")), "trucks are never secret: {before:?}");
        let v = bevy_math::Vec3::new;
        std::fs::write(d.join("SkateVDisplay.txt"), "show3 10 20  12.000 20.000 1.500  -1.0000 0.0000  0.500  wall
").unwrap();
        let mut walls = shop(&d);
        walls.set_secrets(0);
        assert!(walls.display(Tier::Low, v(10.0, 20.0, 1.0), v(10.0, 20.0, 1.5), None, 3, |_, _| None).is_empty());
        assert!(walls.crew_board(0).is_empty());
        let (key, title) = s.unlock_next().unwrap();
        assert_eq!(key, "deck:deckmodel:d1");
        assert!(title.contains("Deck One"), "{title}");
        assert!(s.unlock_next().is_none());
        // Unlocked: sold at the Skate Shop only (design 2026-11-02-collectibles-v2).
        assert!(sold(&mut s, Tier::Skate).iter().any(|t| t.contains("Deck One")));
        assert!(!sold(&mut s, Tier::Mid).iter().any(|t| t.contains("Deck One")));
        // A new start keeps the file: still unlocked, nothing new chosen;
        // crew boards (Suburban's stock) never carry a secret.
        let mut again = shop(&d);
        assert_eq!(again.set_secrets(5), 0);
        assert!(again.crew_board(0).is_empty());
    }

    /// Lines against a floor at z 0 (when `floor`), a wall y = `wall` (if
    /// any) and a long table (x 2..18, y in front of the wall by 0.4 m, z < 1).
    fn scene(wall: Option<f32>, clutter: bool, floor: bool) -> impl FnMut(bevy_math::Vec3, bevy_math::Vec3) -> Option<bevy_math::Vec3> {
        use bevy_math::Vec3;
        move |a: Vec3, b: Vec3| {
            let d = b - a;
            let mut best: Option<(f32, Vec3)> = None;
            let mut take = |t: f32, ok: &dyn Fn(Vec3) -> bool| {
                if (0.0..=1.0).contains(&t) && ok(a + d * t) && best.is_none_or(|x| t < x.0) {
                    best = Some((t, a + d * t));
                }
            };
            if floor && d.z.abs() > 1e-6 {
                take(-a.z / d.z, &|_| true);
            }
            if let Some(y) = wall
                && d.y.abs() > 1e-6
            {
                take((y - a.y) / d.y, &|p: Vec3| p.z >= 0.0 && p.z < 4.0);
                if clutter {
                    take((y + 0.4 - a.y) / d.y, &|p: Vec3| (2.0..18.0).contains(&p.x) && p.z < 1.0);
                }
            }
            best.map(|x| x.1)
        }
    }

    #[test]
    fn the_board_shop_finds_its_front_close_by_or_cluttered() {
        let v = bevy_math::Vec3::new;
        let eye = v(10.0, 20.0, 1.5);
        // 0.6 m away: too close for the store walls' search.
        assert!(crate::shop::display_wall(eye, scene(Some(19.4), false, true)).is_none());
        let (at, normal, _) = crate::shop::storefront(eye, scene(Some(19.4), false, true)).unwrap();
        assert!((at.y - 19.4).abs() < 1e-3 && (normal - v(0.0, 1.0, 0.0)).length() < 1e-3, "{at:?} {normal:?}");
        // 3 m away with a table before it: not clear, still the front.
        assert!(crate::shop::display_wall(eye, scene(Some(17.0), true, true)).is_none());
        assert!(crate::shop::storefront(eye, scene(Some(17.0), true, true)).is_some_and(|(at, _, _)| (at.y - 17.0).abs() < 1e-3));
        assert!(crate::shop::storefront(eye, scene(None, false, true)).is_none());
        // A wall behind the eye faces away from the shop's front (-y): never taken.
        assert!(crate::shop::storefront(eye, scene(Some(21.0), false, true)).is_none());
        assert!(crate::shop::storefront(eye, scene(Some(19.4), false, true)).is_some_and(|w| w.2), "the relaxed search");
    }

    #[test]
    fn the_board_shop_shows_at_a_fixed_front_after_five_misses_on_streamed_ground() {
        let d = dir("front");
        let mut s = shop(&d);
        let v = bevy_math::Vec3::new;
        let (anchor, eye) = (v(10.0, 20.0, 0.0), v(10.0, 20.0, 1.5));
        // Not streamed: never placed.
        for _ in 0..8 {
            assert!(s.display(Tier::Skate, anchor, eye, None, 3, scene(None, false, false)).is_empty());
        }
        let mut s = shop(&dir("front2"));
        for i in 1..5 {
            assert!(s.display(Tier::Skate, anchor, eye, None, 3, scene(None, false, true)).is_empty(), "miss {i}");
            if i == 1 {
                assert!(s.note.contains("no storefront found"), "{}", s.note);
            }
        }
        assert!(!s.display(Tier::Skate, anchor, eye, None, 3, scene(None, false, true)).is_empty());
        let show = s.shows.iter().find(|x| x.store == crate::shop::store_key(anchor)).unwrap();
        assert!((show.normal.truncate() - bevy_math::Vec2::from(crate::shop::SKATE_FRONT).normalize()).length() < 1e-3);
        assert!(show.floor.abs() < 1e-3 && s.note.contains("fixed storefront"), "{}", s.note);
        // Never saved, also not by a later find in another store.
        assert!(!dir_path("front2").join(DISPLAY_FILE).exists());
        let other = v(30.0, 20.0, 1.5);
        assert!(!s.display(Tier::Skate, v(30.0, 20.0, 0.0), other, None, 3, scene(Some(19.4), false, true)).is_empty());
        let saved = std::fs::read_to_string(dir_path("front2").join(DISPLAY_FILE)).unwrap();
        assert!(saved.contains("30 20") && !saved.contains("10 20"), "{saved}");
        // Clothing stores keep their search: no fixed front.
        let mut c = shop(&dir("front3"));
        for _ in 0..8 {
            assert!(c.display(Tier::Mid, v(50.0, 50.0, 0.0), v(50.0, 50.0, 1.5), None, 3, scene(None, false, true)).is_empty());
        }
    }

    #[test]
    fn secrets_list_unlock_by_key_and_relock_all() {
        let d = dir("secretkeys");
        let mut s = shop(&d);
        s.character("franklin");
        assert_eq!(s.set_secrets(1), 1);
        let list = s.secret_list();
        let line = list.lines().next().unwrap();
        let cols: Vec<&str> = line.split('\t').collect();
        assert_eq!(cols[0], "deck:deckmodel:d1");
        assert!(cols[1].contains("Deck One"), "{line}");
        assert!(cols[2].starts_with("skatev_d_"), "the deck model first: {line}");
        assert_eq!(s.unlock("deck:nothing:x"), 0, "no secret");
        assert_eq!(s.unlock(cols[0]), 1);
        assert_eq!(s.unlock(cols[0]), 2, "already unlocked");
        // Kept: a new start still has it unlocked.
        assert_eq!(shop(&d).set_secrets(1), 0);
        // The reset locks it again, keeps the choice and saves.
        assert_eq!(s.relock_all(), 1);
        let mut again = shop(&d);
        assert_eq!(again.set_secrets(7), 1, "same single secret, locked");
        assert_eq!(again.secret_list().lines().next().unwrap().split('\t').next(), Some("deck:deckmodel:d1"));
    }

    #[test]
    fn a_found_secret_is_on_sale_every_day_until_owned() {
        let d = dir("found");
        let mut s = shop(&d);
        s.character("franklin");
        assert_eq!(s.set_secrets(1), 1);
        let found = |s: &BoardShop, tier: Tier, day: u32| {
            s.stock_for(tier, (-1224, -1443), day).into_iter().find(|sec| sec.title == FOUND).map(|sec| sec.offers)
        };
        assert!(found(&s, Tier::Skate, 1).is_none(), "locked: not listed");
        s.unlock_next().unwrap();
        for day in 0..30 {
            let offers = found(&s, Tier::Skate, day).unwrap_or_else(|| panic!("day {day}"));
            assert!(offers.iter().any(|o| o.title.contains("Deck One")), "{offers:?}");
            assert!(offers.iter().all(|o| s.secrets.secret(&o.goods)));
            // Only the Skate Shop sells what was found (design 2026-11-02-collectibles-v2).
            for tier in [Tier::Mid, Tier::Low] {
                assert!(found(&s, tier, day).is_none(), "{tier:?} day {day}: no Found section");
                let all = s.stock_for(tier, (-1224, -1443), day);
                assert!(all.iter().flat_map(|sec| &sec.offers).all(|o| !s.secrets.secret(&o.goods)), "{tier:?} day {day}: a secret on sale");
            }
        }
        let deck = found(&s, Tier::Skate, 3).unwrap().into_iter().find(|o| !matches!(o.goods, Goods::Complete(_))).unwrap();
        s.garage.add(&deck.goods);
        assert!(found(&s, Tier::Skate, 3).unwrap_or_default().iter().all(|o| o.goods != deck.goods), "owned: under Yours");
    }

    #[test]
    fn the_view_lists_rows_and_marks_the_selected_one() {
        let d = dir("view");
        let mut s = shop(&d);
        s.character("franklin");
        assert!(s.open(Tier::Mid));
        let view = s.view();
        let lines: Vec<&str> = view.lines().collect();
        assert!(lines[0].starts_with("Complete boards\t1 / "));
        assert!(lines[1].contains("Suburban"));
        assert!(lines[2].ends_with("\t1"), "the first row is selected: {view}");
        assert!(!s.preview().is_empty());
    }

    #[test]
    fn missing_models_fall_back_and_unbuilt_goods_are_not_sold() {
        let d = dir("missing");
        let mut s = shop(&d);
        s.available.remove("skatev_w_w1");
        assert!(s.open(Tier::Low));
        let sold: Vec<_> = s.menu.as_ref().unwrap().sections.iter().flat_map(|x| x.offers.iter()).map(|o| o.title.clone()).collect();
        assert!(!sold.iter().any(|t| t.contains("Wheel One")), "{sold:?}");
        let mut g = Garage::default();
        g.riding.set(Part::Wheels, Some(PartRef::new("wheelmodel", "w1")));
        assert_eq!(s.models(&g)[2], "skatev_w_default");
    }

    #[test]
    fn painted_wheels_ride_as_hard_as_their_paint() {
        let d = dir("paint");
        let mut s = shop(&d);
        assert_eq!(s.wheel_hardness(), None, "Skate 3's own wheels");
        let paint = skate_boards::paints::catalog().into_iter().next().unwrap();
        s.garage.riding.set(Part::Wheels, Some(paint.part_ref()));
        assert_eq!(s.wheel_hardness(), Some(paint.finish.hardness()));
    }

    #[test]
    fn a_kept_show_puts_its_boards_on_the_exporters_frame_without_repeats() {
        let d = dir("show");
        std::fs::write(d.join("SkateVDisplay.txt"), "show3 10 20  12.000 20.000 1.500  -1.0000 0.0000  0.500  wall\n").unwrap();
        let mut s = shop(&d);
        let v = bevy_math::Vec3::new;
        let shown = s.display(Tier::Low, v(10.0, 20.0, 1.0), v(10.0, 20.0, 1.5), None, 3, |_, _| None);
        // One complete board in the test catalog: shown once, never repeated.
        assert_eq!(shown.len(), 1);
        let places = crate::shop::ShowCase { store: (10, 20), at: v(12.0, 20.0, 1.5), normal: v(-1.0, 0.0, 0.0), floor: 0.5, till: false }
            .places(1);
        // A point of the exported board lands where the place puts it.
        let w = v(0.02, 0.05, 0.4);
        let entity = bevy_math::Mat4::from_cols_array(&shown[0].0);
        let at = entity.transform_point3(s.frame.transform_point3(w));
        assert!((at - places[0].transform_point3(w)).length() < 1e-4, "{at:?}");
        assert!(!shown[0].1.is_empty());
        assert_eq!(s.display(Tier::High, v(10.0, 20.0, 1.0), v(10.0, 20.0, 1.5), None, 3, |_, _| None).len(), 0);
    }

    #[test]
    fn binco_sells_its_display_and_owned_goods_stay_ridable() {
        let d = dir("rotation");
        std::fs::write(d.join("SkateVDisplay.txt"), "show3 10 20  12.000 20.000 1.500  -1.0000 0.0000  0.500  wall
").unwrap();
        let mut s = shop(&d);
        let v = bevy_math::Vec3::new;
        let shown = s.display(Tier::Low, v(10.0, 20.0, 1.0), v(10.0, 20.0, 1.5), None, 3, |_, _| None);
        let stock = s.stock_for(Tier::Low, (10, 20), 3);
        assert!(stock.iter().all(|sec| sec.title == "Complete boards" || sec.title == "Yours"), "Binco: complete boards only");
        let sold: usize = stock.iter().filter(|sec| sec.title == "Complete boards").map(|sec| sec.offers.len()).sum();
        assert_eq!(sold, shown.len(), "the boards on its wall");
        // What is owned is listed under Yours whatever is stocked today.
        let owned = s.stock_for(Tier::Mid, (10, 20), 3).into_iter().flat_map(|sec| sec.offers).next().unwrap();
        s.garage.add(&owned.goods);
        let yours = s.stock_for(Tier::Low, (99, 99), 400);
        assert_eq!(yours[0].title, "Yours");
        assert!(yours[0].offers.iter().any(|o| o.goods == owned.goods && o.price > 0), "listed, at this store's price");
        assert!(!yours[0].offers.iter().any(|o| o.goods == Goods::Grip(crate::shop::DECK_GRIP.into())));
        assert!(s.open_at(Tier::Skate, (-1303, -1369), 3));
        // Suburban's first boards are the ones on its wall.
        let wall = crate::shop::picks_distinct(&[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12], (10, 20), 3, 0, crate::shop::on_show(Tier::Mid));
        let sold = crate::shop::picks_distinct(&[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12], (10, 20), 3, 0, crate::shop::on_show(Tier::Mid) + crate::shop::MID_EXTRA);
        assert_eq!(&sold[..wall.len()], &wall[..]);
    }

    #[test]
    fn the_exporters_frame_is_the_parts_rotation() {
        let f = exporter_frame(bevy_math::Vec3::new(0.0, 0.352, 0.064));
        // (x, y, z) -> (y, 0.352 - z, 0.064 - x), as prepare-board-parts.py.
        let p = f.transform_point3(bevy_math::Vec3::new(-0.0517, 0.0216, -0.0929));
        assert!((p - bevy_math::Vec3::new(0.0216, 0.4449, 0.1157)).length() < 1e-3, "{p:?}");
    }

    #[test]
    fn riding_counts_rolling_grinds_landings_and_bails() {
        let mut r = Riding::default();
        for _ in 0..60 {
            r.frame("PhysicsGround", 5.0, 1.0 / 60.0);
        }
        r.frame("PhysicsAir", 5.0, 0.1);
        r.frame("PhysicsGround", 5.0, 0.1);
        r.frame("GrindFiveO", 3.0, 0.5);
        r.frame("WipeoutGround", 1.0, 0.1);
        r.frame("WipeoutGround", 1.0, 0.1);
        let u = r.usage;
        // A frame counts at most 0.25 s (a hitch does not wear the board).
        assert!((u.rolled - (5.0 + 0.5 + 0.75)).abs() < 0.01, "{u:?}");
        assert_eq!((u.landings, u.bails), (1, 1));
        assert!((u.truck_grind - 0.25).abs() < 1e-6);
        assert!(!r.frame("BipedGround", 2.0, 0.1), "under 2 s so far");
    }

    #[test]
    fn a_worn_through_deck_rides_its_worn_finish_and_a_new_one_is_fresh() {
        let d = dir("wear");
        let mut s = shop(&d);
        s.character("franklin");
        let deck = PartRef::new("deckmodel", "d1");
        s.garage.add(&Goods::Part(Part::Deck, deck.clone()));
        s.garage.ride(&Goods::Part(Part::Deck, deck.clone()));
        assert!(!s.ride("WipeoutGround", 0.0, 0.1));
        // Many bails wear the deck through.
        let mut through = false;
        for i in 0..400 {
            let state = if i % 2 == 0 { "WipeoutGround" } else { "PhysicsGround" };
            through |= s.ride(state, 0.0, 0.5);
        }
        assert!(through, "{}", s.wear.of(&deck));
        assert!(d.join("SkateVWear-franklin.txt").is_file());
        s.wear.renew(&deck);
        assert_eq!(s.wear.of(&deck), 0.0);
    }

    #[test]
    fn the_part_view_shows_only_the_offers_part() {
        let d = dir("partview");
        let mut s = shop(&d);
        assert!(s.open(Tier::Low));
        assert_eq!(s.preview_part().len(), 3, "a complete board shows whole");
        s.input(Input::Right, 0); // Build: Deck, listed by maker
        s.input(Input::Accept, 0); // open the maker
        assert!(s.preview_part().iter().all(|m| m.starts_with("skatev_d_") || m.starts_with("skatev_g_")), "{:?}", s.preview_part());
    }

    #[test]
    fn the_rest_pose_puts_each_bone_at_its_bind() {
        let mut bind = [0.0f32; 16];
        for i in 0..4 {
            bind[i * 5] = 1.0;
        }
        bind[14] = 0.089;
        let mut at = [0.0f32; 16];
        for i in 0..4 {
            at[i * 5] = 1.0;
        }
        at[12] = 10.0;
        let pose = rest_pose(&[bind; 7], &at);
        assert_eq!(pose.bone_count, 7);
        assert_eq!(&pose.world[0][12..15], &[10.0, 0.0, 0.089]);
        assert_eq!(money(1234567), "$1,234,567");
    }
}
