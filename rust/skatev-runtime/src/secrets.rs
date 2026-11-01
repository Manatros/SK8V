//! Secret shop items (contributed by GTAxSkate, design
//! 2026-11-01-collectibles): some decks and griptapes stay out of the shop
//! until a collectible in the world unlocks them. The choice is made once
//! and kept in `SkateVSecrets.txt` next to the garages, so later catalog
//! additions never become secret and progress never drifts.

use std::collections::{BTreeSet, HashSet};

use skate_boards::{BoardChoice, Part};

use crate::shop::{Goods, DECK_GRIP};

pub const FILE: &str = "SkateVSecrets.txt";

fn fnv(text: &str) -> u32 {
    text.bytes().fold(0x811c_9dc5u32, |h, b| (h ^ u32::from(b)).wrapping_mul(0x0100_0193))
}

/// The key of goods that can be secret: a deck (not a paint) or a
/// griptape (not the deck's own grip). None for everything else.
pub fn key(goods: &Goods) -> Option<String> {
    match goods {
        Goods::Part(Part::Deck, r) if r.asset != skate_boards::paints::ASSET => Some(format!("deck:{}:{}", r.asset, r.material)),
        Goods::Grip(id) if id != DECK_GRIP => Some(format!("grip:{id}")),
        _ => None,
    }
}

/// The deck key of a complete board, if it has a deck.
fn deck_key(board: &BoardChoice) -> Option<String> {
    board.get(Part::Deck).and_then(|r| key(&Goods::Part(Part::Deck, r.clone())))
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Secrets {
    /// Every secret, in unlock order (lowest hash first).
    order: Vec<String>,
    unlocked: BTreeSet<String>,
    secret: HashSet<String>,
}

impl Secrets {
    /// The `count` keys of `keys` with the lowest hash (ties by name).
    pub fn choose(keys: impl IntoIterator<Item = String>, count: usize) -> Self {
        let mut all: Vec<String> = keys.into_iter().collect::<BTreeSet<_>>().into_iter().collect();
        all.sort_by(|a, b| fnv(a).cmp(&fnv(b)).then_with(|| a.cmp(b)));
        all.truncate(count);
        Self::from_parts(all, BTreeSet::new())
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

    /// Goods still hidden: a locked item, or a complete board whose deck is locked.
    pub fn locked(&self, goods: &Goods) -> bool {
        match goods {
            Goods::Complete(board) => self.board_locked(board),
            other => key(other).is_some_and(|k| self.key_locked(&k)),
        }
    }

    pub fn board_locked(&self, board: &BoardChoice) -> bool {
        deck_key(board).is_some_and(|k| self.key_locked(&k))
    }

    /// Goods that are secret at all (locked or unlocked); a complete board by its deck.
    pub fn secret(&self, goods: &Goods) -> bool {
        match goods {
            Goods::Complete(board) => self.board_secret(board),
            other => key(other).is_some_and(|k| self.secret.contains(&k)),
        }
    }

    pub fn board_secret(&self, board: &BoardChoice) -> bool {
        deck_key(board).is_some_and(|k| self.secret.contains(&k))
    }

    pub fn remaining(&self) -> usize {
        self.order.iter().filter(|k| !self.unlocked.contains(*k)).count()
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
    fn only_decks_and_griptapes_can_be_secret() {
        assert_eq!(key(&deck("d1")).as_deref(), Some("deck:deckmodel:d1"));
        assert_eq!(key(&Goods::Grip("g1".into())).as_deref(), Some("grip:g1"));
        assert_eq!(key(&Goods::Grip(DECK_GRIP.into())), None);
        assert_eq!(key(&Goods::Part(Part::Deck, PartRef::new(skate_boards::paints::ASSET, "paint_red"))), None);
        assert_eq!(key(&Goods::Part(Part::Wheels, PartRef::new("wheelmodel", "w1"))), None);
        assert_eq!(key(&Goods::Sticker("s1".into())), None);
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
    fn a_complete_board_is_locked_by_its_deck() {
        let s = Secrets::choose(["deck:deckmodel:d1".to_owned()], 1);
        let mut board = BoardChoice::default();
        board.set(Part::Deck, Some(PartRef::new("deckmodel", "d1")));
        assert!(s.locked(&Goods::Complete(board.clone())));
        assert!(s.locked(&deck("d1")));
        assert!(!s.locked(&deck("d2")));
        board.set(Part::Deck, Some(PartRef::new("deckmodel", "d2")));
        assert!(!s.locked(&Goods::Complete(board)));
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
