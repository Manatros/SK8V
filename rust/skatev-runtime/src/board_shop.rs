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
}

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
        Ok(Self::new(library, available, binds, garages))
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
        }
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
        if Self::is_default(&self.garage) { Vec::new() } else { self.models(&self.garage) }
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

    /// Opens the store's menu; false where boards are not sold.
    pub fn open(&mut self, tier: Tier) -> bool {
        if !tier.sells_boards() {
            return false;
        }
        let mut sections = crate::shop::stock(&self.library, tier);
        for section in &mut sections {
            section.offers.retain(|offer| self.sells(&offer.goods));
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

/// Opens the store menu: tier 0 Binco / Discount Store, 1 Suburban, 2
/// Ponsonbys (sells none). Returns 1 when open.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sv_shop_open(_rt: *mut c_void, tier: u32) -> u32 {
    crate::guard(0, || {
        let tier = match tier {
            0 => Tier::Low,
            1 => Tier::Mid,
            _ => Tier::High,
        };
        with(0, |s| u32::from(s.open(tier)))
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
/// ridden painted wheels' hardness ("0.60"; empty for unpainted wheels).
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
            4 => s.wheel_hardness().map(|h| format!("{h:.2}")).unwrap_or_default(),
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

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("skatev-shop-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
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
