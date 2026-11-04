//! Secret shop items (contributed by GTAxSkate, design
//! 2026-11-01-collectibles): some decks, griptapes, trucks and wheels stay
//! out of the shop until a collectible in the world unlocks them. The choice
//! is made once and kept in `SkateVSecrets3.txt` next to the garages, so later
//! catalog additions never become secret and progress never drifts
//! (collectibles v3, design 2026-11-03-collectibles-v3; v2's
//! `SkateVSecrets.txt` stays untouched for a rollback).

use std::collections::{BTreeSet, HashSet};

use skate_boards::{BoardChoice, Part};

use crate::shop::{Goods, DECK_GRIP};

pub const FILE: &str = "SkateVSecrets3.txt";
/// Kinds a secret can be, in the order collectibles take them.
const KINDS: [&str; 4] = ["deck", "grip", "trucks", "wheels"];
/// At most one in this many of a kind's goods is secret (design rev 2: the
/// shop never runs dry).
const KIND_SHARE: usize = 3;

fn fnv(text: &str) -> u32 {
    text.bytes().fold(0x811c_9dc5u32, |h, b| (h ^ u32::from(b)).wrapping_mul(0x0100_0193))
}

/// The key of goods that can be secret: a deck, trucks or wheels (not a
/// paint) or a griptape (not the deck's own grip). None for everything else.
pub fn key(goods: &Goods) -> Option<String> {
    match goods {
        Goods::Part(part, r) if r.asset != skate_boards::paints::ASSET => {
            let kind = match part { Part::Deck => "deck", Part::Trucks => "trucks", Part::Wheels => "wheels" };
            Some(format!("{kind}:{}:{}", r.asset, r.material))
        }
        Goods::Grip(id) if id != DECK_GRIP => Some(format!("grip:{id}")),
        _ => None,
    }
}

/// The keys of a complete board's parts (deck, trucks, wheels).
fn board_keys(board: &BoardChoice) -> Vec<String> {
    Part::ALL.iter().filter_map(|&p| board.get(p).and_then(|r| key(&Goods::Part(p, r.clone())))).collect()
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Secrets {
    /// Every secret, in unlock order (lowest hash first).
    order: Vec<String>,
    unlocked: BTreeSet<String>,
    secret: HashSet<String>,
}

impl Secrets {
    /// `count` keys of `keys`: per kind the lowest hashes (ties by name), at
    /// most one in `KIND_SHARE` of the kind, taken in turn deck, grip,
    /// trucks, wheels so each kind gets its share of the collectibles.
    pub fn choose(keys: impl IntoIterator<Item = String>, count: usize) -> Self {
        Self::choose_with_decks(keys, count, 0)
    }

    /// As `choose`, with the first `decks` secrets decks (one per board
    /// collectible, collectibles v3 re-review), then the kinds in turn.
    pub fn choose_with_decks(keys: impl IntoIterator<Item = String>, count: usize, decks: usize) -> Self {
        let all: BTreeSet<String> = keys.into_iter().collect();
        let mut kinds: Vec<Vec<String>> = KINDS.iter().map(|kind| {
            let mut of: Vec<String> = all.iter().filter(|k| k.split(':').next() == Some(*kind)).cloned().collect();
            of.sort_by(|a, b| fnv(a).cmp(&fnv(b)).then_with(|| a.cmp(b)));
            of.truncate(of.len().div_ceil(KIND_SHARE));
            of.reverse();
            of
        }).collect();
        let mut order = Vec::new();
        while order.len() < decks.min(count) {
            let Some(k) = kinds[0].pop() else { break };
            order.push(k);
        }
        while order.len() < count && kinds.iter().any(|k| !k.is_empty()) {
            for kind in &mut kinds {
                if order.len() < count {
                    if let Some(k) = kind.pop() { order.push(k); }
                }
            }
        }
        Self::from_parts(order, BTreeSet::new())
    }

    fn from_parts(order: Vec<String>, unlocked: BTreeSet<String>) -> Self {
        let secret = order.iter().cloned().collect();
        Self { order, unlocked, secret }
    }

    pub fn from_text(text: &str) -> Self {
        let (mut order, mut unlocked) = (Vec::new(), BTreeSet::new());
        for line in text.lines() {
            if let Some(k) = line.strip_prefix("secret ") {
                let k = k.trim();
                if !k.is_empty() && !order.iter().any(|o| o == k) {
                    order.push(k.to_owned());
                }
            } else if let Some(k) = line.strip_prefix("unlocked ") {
                unlocked.insert(k.trim().to_owned());
            }
        }
        Self::from_parts(order, unlocked)
    }

    pub fn to_text(&self) -> String {
        let mut out = String::from("# GTAxSkate: shop items found as collectibles (design 2026-11-01-collectibles)\n");
        for k in &self.order {
            out.push_str(&format!("secret {k}\n"));
        }
        for k in &self.unlocked {
            out.push_str(&format!("unlocked {k}\n"));
        }
        out
    }

    pub fn is_empty(&self) -> bool {
        self.order.is_empty()
    }

    fn key_locked(&self, key: &str) -> bool {
        self.secret.contains(key) && !self.unlocked.contains(key)
    }

    /// Goods still hidden: a locked item, or a complete board with a locked part.
    pub fn locked(&self, goods: &Goods) -> bool {
        match goods {
            Goods::Complete(board) => self.board_locked(board),
            other => key(other).is_some_and(|k| self.key_locked(&k)),
        }
    }

    pub fn board_locked(&self, board: &BoardChoice) -> bool {
        board_keys(board).iter().any(|k| self.key_locked(k))
    }

    /// Goods that are secret at all (locked or unlocked); a complete board by any part.
    pub fn secret(&self, goods: &Goods) -> bool {
        match goods {
            Goods::Complete(board) => self.board_secret(board),
            other => key(other).is_some_and(|k| self.secret.contains(&k)),
        }
    }

    pub fn board_secret(&self, board: &BoardChoice) -> bool {
        board_keys(board).iter().any(|k| self.secret.contains(k))
    }

    pub fn remaining(&self) -> usize {
        self.order.iter().filter(|k| !self.unlocked.contains(*k)).count()
    }

    /// Every secret, in unlock order (design 2026-11-02-collectibles-v2:
    /// the k-th part collectible holds the k-th).
    pub fn order(&self) -> &[String] {
        &self.order
    }

    pub fn is_unlocked(&self, key: &str) -> bool {
        self.unlocked.contains(key)
    }

    /// Unlocks `key`: 1 unlocked now, 2 it already was, 0 no secret.
    pub fn unlock(&mut self, key: &str) -> u32 {
        if !self.secret.contains(key) {
            0
        } else if self.unlocked.insert(key.to_owned()) {
            1
        } else {
            2
        }
    }

    /// Locks every secret again (the Developer reset); the choice stays.
    pub fn relock_all(&mut self) {
        self.unlocked.clear();
    }

    /// Unlocks the next locked secret; its key, none when all are unlocked.
    pub fn unlock_next(&mut self) -> Option<String> {
        let next = self.order.iter().find(|k| !self.unlocked.contains(*k))?.clone();
        self.unlocked.insert(next.clone());
        Some(next)
    }
}

/// The first `count` of `order`, each secret slot standing in for its item
/// while that is locked: the j-th secret slot always gets the j-th spare (a
/// later item that is no secret at all), so the stand-ins never depend on
/// what else is unlocked and an unlock changes only its own slot (A18).
pub fn substitute<T: Clone + PartialEq>(order: &[T], count: usize, secret: impl Fn(&T) -> bool, locked: impl Fn(&T) -> bool) -> Vec<T> {
    let count = count.min(order.len());
    let mut spares = order[count..].iter().filter(|s| !secret(s) && !order[..count].contains(s));
    let mut out = Vec::new();
    for t in &order[..count] {
        if !secret(t) {
            out.push(t.clone());
        } else {
            let stand_in = spares.next();
            if !locked(t) {
                out.push(t.clone());
            } else if let Some(s) = stand_in {
                out.push(s.clone());
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use skate_boards::PartRef;

    fn deck(m: &str) -> Goods {
        Goods::Part(Part::Deck, PartRef::new("deckmodel", m))
    }

    #[test]
    fn decks_griptapes_trucks_and_wheels_can_be_secret_but_no_paint() {
        assert_eq!(key(&deck("d1")).as_deref(), Some("deck:deckmodel:d1"));
        assert_eq!(key(&Goods::Grip("g1".into())).as_deref(), Some("grip:g1"));
        assert_eq!(key(&Goods::Grip(DECK_GRIP.into())), None);
        assert_eq!(key(&Goods::Part(Part::Deck, PartRef::new(skate_boards::paints::ASSET, "paint_red"))), None);
        assert_eq!(key(&Goods::Part(Part::Wheels, PartRef::new("wheelmodel", "w1"))).as_deref(), Some("wheels:wheelmodel:w1"));
        assert_eq!(key(&Goods::Part(Part::Trucks, PartRef::new("truckmodel", "t1"))).as_deref(), Some("trucks:truckmodel:t1"));
        assert_eq!(key(&Goods::Part(Part::Wheels, PartRef::new(skate_boards::paints::ASSET, "paint_red"))), None);
        assert_eq!(key(&Goods::Sticker("s1".into())), None);
    }

    #[test]
    fn boards_get_their_decks_first() {
        // 41 boards and 70 parts against a shop of 300 decks, 21 grips, 30 trucks, 90 wheels.
        let keys = (0..300).map(|i| format!("deck:m:{i}"))
            .chain((0..21).map(|i| format!("grip:{i}")))
            .chain((0..30).map(|i| format!("trucks:t:{i}")))
            .chain((0..90).map(|i| format!("wheels:w:{i}")));
        let s = Secrets::choose_with_decks(keys, 111, 41);
        assert!(s.order[..41].iter().all(|k| k.starts_with("deck:")));
        let of = |kind: &str| s.order[41..].iter().filter(|k| k.starts_with(kind)).count();
        assert_eq!(s.order.len(), 111);
        assert!(of("grip:") == 7 && of("trucks:") >= 10 && of("wheels:") >= 10, "parts get the scarce kinds");
    }

    #[test]
    fn the_choice_takes_kinds_in_turn_and_keeps_two_thirds_on_sale() {
        let keys = (0..30).map(|i| format!("deck:m:{i}"))
            .chain((0..9).map(|i| format!("trucks:t:{i}")))
            .chain((0..30).map(|i| format!("grip:{i}")))
            .chain((0..12).map(|i| format!("wheels:w:{i}")));
        let s = Secrets::choose(keys, 100);
        let of = |kind: &str| s.order.iter().filter(|k| k.starts_with(kind)).count();
        assert_eq!((of("deck:"), of("grip:"), of("trucks:"), of("wheels:")), (10, 10, 3, 4));
        // In turn: deck, grip, trucks, wheels, deck, ...
        assert!(s.order[0].starts_with("deck:") && s.order[1].starts_with("grip:")
            && s.order[2].starts_with("trucks:") && s.order[3].starts_with("wheels:"));
    }

    #[test]
    fn the_choice_is_stable_and_survives_the_file() {
        let keys = (0..50).map(|i| format!("deck:m:{i}"));
        let a = Secrets::choose(keys.clone(), 7);
        let b = Secrets::choose(keys.rev(), 7);
        assert_eq!(a, b);
        assert_eq!(a.remaining(), 7);
        assert_eq!(Secrets::from_text(&a.to_text()), a);
        // More keys later (a new board DLC) never change a kept file.
        let kept = Secrets::from_text(&a.to_text());
        assert_eq!(kept.order, a.order);
    }

    #[test]
    fn unlocking_goes_in_order_and_ends() {
        let mut s = Secrets::choose(["deck:m:a".to_owned(), "grip:b".to_owned()], 2);
        let first = s.order[0].clone();
        assert_eq!(s.unlock_next().as_deref(), Some(first.as_str()));
        assert_eq!(s.remaining(), 1);
        assert!(s.unlock_next().is_some());
        assert_eq!(s.unlock_next(), None);
        let back = Secrets::from_text(&s.to_text());
        assert_eq!(back.remaining(), 0);
    }

    #[test]
    fn a_complete_board_is_locked_by_any_locked_part() {
        let s = Secrets::choose(["deck:deckmodel:d1".to_owned()], 1);
        let mut board = BoardChoice::default();
        board.set(Part::Deck, Some(PartRef::new("deckmodel", "d1")));
        assert!(s.locked(&Goods::Complete(board.clone())));
        assert!(s.locked(&deck("d1")));
        assert!(!s.locked(&deck("d2")));
        board.set(Part::Deck, Some(PartRef::new("deckmodel", "d2")));
        assert!(!s.locked(&Goods::Complete(board.clone())));
        // Its trucks locked: hidden too (collectibles v3).
        let t = Secrets::from_text("secret trucks:truckmodel:t1\n");
        board.set(Part::Trucks, Some(PartRef::new("truckmodel", "t1")));
        assert!(t.locked(&Goods::Complete(board)));
    }

    #[test]
    fn substitution_changes_only_the_slot_that_unlocks() {
        let order = [1, 2, 3, 4, 5, 6];
        let secret = |t: &i32| [2, 3, 4].contains(t);
        let wall = |locked: &[i32]| substitute(&order, 3, secret, |t| locked.contains(t));
        assert_eq!(substitute(&order, 3, |_| false, |_| false), vec![1, 2, 3]);
        // Secrets 2 and 3 on the wall stand in by 5 and 6 (4 is a secret, never a spare).
        assert_eq!(wall(&[2, 3, 4]), vec![1, 5, 6]);
        // Unlocking 4 (not on the wall) changes nothing; unlocking 2 only its slot.
        assert_eq!(wall(&[2, 3]), vec![1, 5, 6]);
        assert_eq!(wall(&[3]), vec![1, 2, 6]);
        assert_eq!(wall(&[]), vec![1, 2, 3]);
        // Too few spares: a locked slot without one stays empty.
        assert_eq!(substitute(&order, 6, |_| true, |&t| t > 1), vec![1]);
    }
}
