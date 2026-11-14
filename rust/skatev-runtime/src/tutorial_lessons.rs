//! Optional, runtime-owned Coach Frank objectives. No existing ABI records change.
use bevy_math::Vec3;
use std::{ffi::c_void, mem::size_of};

pub const ALL_LESSONS: u32 = (1 << 17) - 1;
pub const CMD_START: u32 = 1;
pub const CMD_STOP: u32 = 2;
pub const CMD_RETRY: u32 = 3;
pub const CMD_SKIP: u32 = 4;
pub const CMD_PREV: u32 = 5;
pub const CMD_RESTART: u32 = 6;
pub const CMD_RESTORE: u32 = 7;
pub const RIDING: u32 = 1;
pub const WIPEOUT: u32 = 2;
/// Explicit opt-in to objectives outside either district; still requires riding.
pub const FREE_ROAM: u32 = 4;
pub const CLEAN: u32 = 8;

#[repr(u32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum UiPreset {
    Sk8v = 0,
    #[default]
    GtaxSkate = 1,
    Rp = 2,
}
impl UiPreset {
    fn from_raw(v: u32) -> Option<Self> {
        Some(match v {
            0 => Self::Sk8v,
            1 => Self::GtaxSkate,
            2 => Self::Rp,
            _ => return None,
        })
    }
    pub fn prompt(self, title: &str, objective: &str, passed: bool) -> String {
        match self {
            Self::Sk8v if passed => format!("Coach Frank: Nailed it! {title} - LESSON PASSED!"),
            Self::Sk8v => format!("Coach Frank: {title} - {objective}"),
            Self::GtaxSkate if passed => format!("Objective complete: {title}"),
            Self::GtaxSkate => format!("Objective: {objective}"),
            Self::Rp if passed => format!("Completed {title}."),
            Self::Rp => objective.into(),
        }
    }
}
#[repr(u32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum State {
    #[default]
    Idle,
    Briefing,
    Active,
    Passed,
    CompletedAll,
}
#[derive(Clone, Copy)]
pub struct Lesson {
    pub id: &'static str,
    pub title: &'static str,
    pub objective: &'static str,
}
macro_rules! lesson {
    ($id:literal,$title:literal,$goal:literal) => {
        Lesson {
            id: $id,
            title: $title,
            objective: $goal,
        }
    };
}
pub const LESSONS: [Lesson; 17] = [
    lesson!(
        "tut_sksc_01",
        "Push & Carve",
        "Reach 5 m/s in Skate School."
    ),
    lesson!("tut_sksc_02", "Ollie Basics", "Land a clean Ollie."),
    lesson!(
        "tut_sksc_03",
        "Manuals",
        "Complete a clean Manual or Nose Manual."
    ),
    lesson!(
        "tut_sksc_04",
        "Flip Tricks",
        "Land a clean Kickflip or Heelflip."
    ),
    lesson!(
        "tut_sksc_05",
        "Pop Shuvit",
        "Land a clean Shuvit or Pop Shuvit."
    ),
    lesson!("tut_sksc_06", "Grinds", "Complete a clean grind or slide."),
    lesson!("tut_sksc_07", "180 Spin", "Land a clean 180 spin."),
    lesson!("tut_sksc_08", "Grabs", "Land a clean grab."),
    lesson!(
        "tut_sksc_09",
        "Transition & Pump",
        "Land a clean air, pump or vert trick in Skate School."
    ),
    lesson!(
        "tut_sksc_10",
        "Line Sequence",
        "Complete a clean 400 point line or reach 2x."
    ),
    lesson!("tut_sksc_11", "Hall of Meat Bail", "Bail into a Wipeout."),
    lesson!(
        "tut_sksc_12",
        "Skate School Final",
        "Complete a clean 800 point sequence in Skate School."
    ),
    lesson!(
        "tut_skpk_01",
        "StartPark: Warmup Ollie",
        "Land a clean Ollie in StartPark."
    ),
    lesson!(
        "tut_skpk_02",
        "StartPark: Box Grind",
        "Complete a clean grind in StartPark."
    ),
    lesson!(
        "tut_skpk_03",
        "StartPark: Flip + Line",
        "Land a clean flip in a 300 point line."
    ),
    lesson!(
        "tut_skpk_04",
        "StartPark: Air & Grab",
        "Land a clean air with a grab."
    ),
    lesson!(
        "tut_skpk_05",
        "StartPark: Pro Line",
        "Complete a clean 1200 point line."
    ),
];
#[repr(C)]
#[derive(Clone, Copy)]
pub struct SvTutorialInput {
    pub size: u32,
    pub preset: u32,
    pub flags: u32,
    pub position: [f32; 3],
    pub speed: f32,
    pub sequence_score: f32,
    pub line_score: f32,
    pub multiplier: f32,
    pub completed_lines: f32,
    pub sequence_active: u32,
    pub trick_utf8: [u8; 96],
    pub dt_seconds: f32,
}
impl Default for SvTutorialInput {
    fn default() -> Self {
        Self {
            size: size_of::<Self>() as u32,
            preset: 1,
            flags: 0,
            position: [0.; 3],
            speed: 0.,
            sequence_score: 0.,
            line_score: 0.,
            multiplier: 0.,
            completed_lines: 0.,
            sequence_active: 0,
            trick_utf8: [0; 96],
            dt_seconds: 0.,
        }
    }
}
#[repr(C)]
#[derive(Clone, Copy)]
pub struct SvTutorialStatus {
    pub size: u32,
    pub state: u32,
    pub lesson: u32,
    pub completed_mask: u32,
    pub completed_count: u32,
    pub preset: u32,
    pub banner_utf8: [u8; 256],
    pub lesson_id: [u8; 24],
}
impl Default for SvTutorialStatus {
    fn default() -> Self {
        Self {
            size: size_of::<Self>() as u32,
            state: 0,
            lesson: 0,
            completed_mask: 0,
            completed_count: 0,
            preset: 1,
            banner_utf8: [0; 256],
            lesson_id: [0; 24],
        }
    }
}
#[derive(Default)]
pub struct TutorialEngine {
    pub state: State,
    pub lesson: usize,
    pub completed_mask: u32,
    preset: UiPreset,
    last: Option<SvTutorialInput>,
    passed_seconds: f32,
}
fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(&bytes[..bytes.iter().position(|&c| c == 0).unwrap_or(bytes.len())])
        .into_owned()
}
fn tokens(name: &str) -> Vec<String> {
    name.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect()
}
fn has(t: &[String], words: &[&str]) -> bool {
    t.iter().any(|s| words.contains(&s.as_str()))
}
fn grind(t: &[String]) -> bool {
    has(
        t,
        &[
            "grind",
            "slide",
            "boardslide",
            "lipslide",
            "tailslide",
            "noseslide",
            "crooked",
            "smith",
            "feeble",
        ],
    ) || t.windows(2).any(|w| w == ["50", "50"])
}
fn grab(t: &[String]) -> bool {
    has(t, &["melon", "indy", "mute", "method", "grab"])
}
fn flip(t: &[String]) -> bool {
    has(t, &["kickflip", "heelflip"])
}
impl TutorialEngine {
    fn briefing(&mut self) {
        self.state = State::Briefing;
        self.last = None;
        self.passed_seconds = 0.;
    }
    fn next(&mut self) {
        if self.completed_mask == ALL_LESSONS {
            self.state = State::CompletedAll;
        } else {
            self.lesson = if self.lesson < 16 {
                self.lesson + 1
            } else {
                (0..17)
                    .find(|i| self.completed_mask & (1 << i) == 0)
                    .unwrap()
            };
            self.briefing();
        }
    }
    pub fn command(&mut self, cmd: u32, arg: u32) -> u32 {
        match cmd {
            CMD_START => {
                if self.completed_mask == ALL_LESSONS {
                    self.state = State::CompletedAll;
                } else {
                    self.briefing();
                }
            }
            CMD_STOP => self.state = State::Idle,
            CMD_RETRY => self.briefing(),
            CMD_SKIP => self.next(),
            CMD_PREV => {
                self.lesson = self.lesson.saturating_sub(1);
                self.briefing();
            }
            CMD_RESTART => {
                *self = Self::default();
                self.briefing();
            }
            CMD_RESTORE if arg & !ALL_LESSONS == 0 => {
                self.completed_mask = arg;
                self.lesson = (0..17).find(|i| arg & (1 << i) == 0).unwrap_or(16);
                self.state = if arg == ALL_LESSONS {
                    State::CompletedAll
                } else {
                    State::Idle
                };
                self.last = None;
            }
            _ => return 0,
        }
        1
    }
    fn zone_matches(&self, position: [f32; 3], flags: u32) -> bool {
        if !position.iter().all(|v| v.is_finite()) {
            return false;
        }
        let zone = crate::tutorial_zone::district(Vec3::from_array(position));
        let matching = zone
            == Some(if self.lesson < 12 {
                "DIST_SkateSchool"
            } else {
                "DIST_StartPark"
            });
        matching || (zone.is_none() && flags & FREE_ROAM != 0)
    }
    fn objective(&self, i: &SvTutorialInput, completed: bool) -> bool {
        if !i.position.iter().all(|v| v.is_finite())
            || ![
                i.speed,
                i.sequence_score,
                i.line_score,
                i.multiplier,
                i.completed_lines,
            ]
            .iter()
            .all(|v| v.is_finite())
        {
            return false;
        }
        if !self.zone_matches(i.position, i.flags) {
            return false;
        }
        if self.lesson == 10 {
            return i.flags & WIPEOUT != 0 && self.last.is_some_and(|p| p.flags & WIPEOUT == 0);
        }
        if i.flags & WIPEOUT != 0 || i.flags & RIDING == 0 {
            return false;
        }
        if self.lesson == 0 {
            return i.speed.is_finite() && i.speed >= 5.;
        }
        if !completed || i.flags & CLEAN == 0 {
            return false;
        }
        let t = tokens(&text(&i.trick_utf8));
        let line = |n| i.line_score.is_finite() && i.line_score >= n;
        let sequence = |n| i.sequence_score.is_finite() && i.sequence_score >= n;
        match self.lesson {
            1 | 12 => has(&t, &["ollie"]),
            2 => has(&t, &["manual", "nosemanual"]),
            3 => flip(&t),
            4 => has(&t, &["shuvit", "shuv", "shove", "popshuvit", "popshuv"]),
            5 | 13 => grind(&t),
            6 => has(
                &t,
                &["180", "fs180", "bs180", "frontside180", "backside180"],
            ),
            7 => grab(&t),
            8 => has(&t, &["air", "pump", "vert"]),
            9 => line(400.) || (i.multiplier.is_finite() && i.multiplier >= 2.),
            11 => sequence(800.),
            14 => flip(&t) && line(300.),
            // A landed grab is an aerial trick, even when its name omits "air".
            15 => grab(&t),
            16 => line(1200.),
            _ => false,
        }
    }
    pub fn step(&mut self, i: SvTutorialInput) -> SvTutorialStatus {
        self.preset = UiPreset::from_raw(i.preset).unwrap_or_default();
        let changed = self.last.is_some_and(|p| {
            p.trick_utf8 != i.trick_utf8
                || p.sequence_score != i.sequence_score
                || p.line_score != i.line_score
                || p.completed_lines != i.completed_lines
        });
        let prev_eligible = self
            .last
            .is_some_and(|p| p.sequence_active != 0 || self.zone_matches(p.position, p.flags));
        let completed = prev_eligible
            && i.sequence_active == 0
            && self.last.is_some_and(|p| p.sequence_active != 0 || changed);
        match self.state {
            State::Briefing => self.state = State::Active,
            State::Active if self.objective(&i, completed) => {
                self.completed_mask |= 1 << self.lesson;
                self.state = State::Passed;
                self.passed_seconds = 0.;
            }
            State::Passed => {
                if i.dt_seconds.is_finite() {
                    self.passed_seconds += i.dt_seconds.clamp(0., 0.1);
                }
                if self.passed_seconds >= 3. {
                    self.next();
                }
            }
            _ => (),
        }
        self.last = Some(i);
        self.status()
    }
    pub fn status(&self) -> SvTutorialStatus {
        let lesson = LESSONS[self.lesson];
        let mut out = SvTutorialStatus {
            state: self.state as u32,
            lesson: self.lesson as u32,
            completed_mask: self.completed_mask,
            completed_count: self.completed_mask.count_ones(),
            preset: self.preset as u32,
            ..Default::default()
        };
        let banner = match self.state {
            State::Idle => String::new(),
            State::CompletedAll => self
                .preset
                .prompt("All 17 lessons", "Tutorial complete.", true),
            _ => self
                .preset
                .prompt(lesson.title, lesson.objective, self.state == State::Passed),
        };
        crate::copy_str(&mut out.banner_utf8, &banner);
        crate::copy_str(&mut out.lesson_id, lesson.id);
        out
    }
}
/// Commands use an sv_create runtime handle. arg is a mask only for RESTORE.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sv_tutorial_command(rt: *mut c_void, cmd: u32, arg: u32) -> u32 {
    crate::guard(0, || {
        let Some(rt) = (unsafe { crate::runtime(rt) }) else {
            return 0;
        };
        rt.tutorial.lock().unwrap().command(cmd, arg)
    })
}
/// Both records must have size >= this version. No worker state is mutated.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sv_tutorial_step(
    rt: *mut c_void,
    input: *const SvTutorialInput,
    out: *mut SvTutorialStatus,
) -> u32 {
    crate::guard(0, || {
        let (Some(rt), Some(i), Some(o)) = (
            unsafe { crate::runtime(rt) },
            unsafe { input.as_ref() },
            unsafe { out.as_mut() },
        ) else {
            return 0;
        };
        if i.size < size_of::<SvTutorialInput>() as u32
            || o.size < size_of::<SvTutorialStatus>() as u32
            || UiPreset::from_raw(i.preset).is_none()
        {
            return 0;
        }
        *o = rt.tutorial.lock().unwrap().step(*i);
        1
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn input(lesson: usize, trick: &str) -> SvTutorialInput {
        let mut i = SvTutorialInput {
            flags: RIDING | CLEAN,
            position: [if lesson < 12 { -4500. } else { -5450. }, -3200., 93.],
            speed: 5.,
            sequence_score: 800.,
            line_score: 1200.,
            multiplier: 2.,
            ..Default::default()
        };
        crate::copy_str(&mut i.trick_utf8, trick);
        i
    }
    fn engine(lesson: usize) -> TutorialEngine {
        let mut e = TutorialEngine {
            lesson,
            ..Default::default()
        };
        e.command(CMD_START, 0);
        e.step(input(lesson, ""));
        e
    }
    const TRICKS: [&str; 17] = [
        "",
        "Ollie",
        "Nose Manual",
        "Kickflip",
        "Pop Shuvit",
        "50-50",
        "FS 180 Ollie",
        "Melon",
        "Vert Air",
        "Ollie",
        "",
        "Ollie",
        "Ollie",
        "Box Grind",
        "Heelflip",
        "Indy",
        "Ollie",
    ];
    macro_rules! lesson_test {
        ($name:ident, $n:expr) => {
            #[test]
            fn $name() {
                let mut e = engine($n);
                let mut i = input($n, TRICKS[$n]);
                if $n == 10 {
                    i.flags = WIPEOUT;
                }
                if $n == 0 {
                    i.speed = 4.99;
                    assert_eq!(e.step(i).state, State::Active as u32);
                    i.speed = 5.;
                }
                assert_eq!(e.step(i).state, State::Passed as u32);
                assert_eq!(e.completed_mask, 1 << $n);
                assert_eq!(e.status().completed_count, 1);
            }
        };
    }
    lesson_test!(push_and_carve, 0);
    lesson_test!(ollie, 1);
    lesson_test!(manual, 2);
    lesson_test!(flip_tricks, 3);
    lesson_test!(pop_shuvit, 4);
    lesson_test!(grinds, 5);
    lesson_test!(spin, 6);
    lesson_test!(grabs, 7);
    lesson_test!(transition, 8);
    lesson_test!(line_sequence, 9);
    lesson_test!(bail, 10);
    lesson_test!(school_final, 11);
    lesson_test!(park_ollie, 12);
    lesson_test!(park_grind, 13);
    lesson_test!(park_flip_line, 14);
    lesson_test!(park_air_grab, 15);
    lesson_test!(park_pro_line, 16);
    #[test]
    fn matching_variants_and_false_positives() {
        for (n, names) in [
            (1, vec!["Switch Ollie"]),
            (2, vec!["Manual", "Nose Manual"]),
            (3, vec!["Heelflip", "Nollie Kickflip"]),
            (4, vec!["Shuvit", "Pop Shuvit", "Pop Shuv-it"]),
            (
                5,
                vec![
                    "50-50",
                    "Grind",
                    "Noseslide",
                    "Slide",
                    "Crooked",
                    "Smith",
                    "Feeble",
                ],
            ),
            (7, vec!["Melon", "Indy", "Mute", "Method", "Grab"]),
            (8, vec!["Air", "Pump", "Vert"]),
        ] {
            for name in names {
                assert_eq!(
                    engine(n).step(input(n, name)).state,
                    State::Passed as u32,
                    "{name}"
                );
            }
        }
        for (n, name) in [
            (1, "Nollie"),
            (2, "Manualistic"),
            (3, "Kickflipper"),
            (5, "500-500"),
            (6, "1800 Ollie"),
            (7, "Indycar"),
            (8, "Chair"),
        ] {
            assert_eq!(engine(n).step(input(n, name)).state, State::Active as u32);
        }
    }
    #[test]
    fn wipeouts_never_pass_non_bail() {
        for n in (0..17).filter(|&n| n != 10) {
            let mut i = input(n, TRICKS[n]);
            i.flags |= WIPEOUT;
            assert_eq!(engine(n).step(i).state, State::Active as u32);
        }
    }
    #[test]
    fn zone_riding_and_free_roam_gates() {
        for n in 0..17 {
            let mut i = input(n, TRICKS[n]);
            if n == 10 {
                i.flags |= WIPEOUT;
            }
            i.position[0] = if n < 12 { -5450. } else { -4500. };
            assert_eq!(engine(n).step(i).state, State::Active as u32);
            i.flags |= FREE_ROAM; // Opt-in never overrides a wrong tutorial district.
            assert_eq!(engine(n).step(i).state, State::Active as u32);
            i.position = [0.; 3];
            assert_eq!(engine(n).step(i).state, State::Passed as u32);
            i.flags &= !FREE_ROAM;
            assert_eq!(engine(n).step(i).state, State::Active as u32);
        }
        let mut i = input(1, "Ollie");
        i.flags = CLEAN;
        assert_eq!(engine(1).step(i).state, State::Active as u32);
        i.flags = RIDING;
        assert_eq!(engine(1).step(i).state, State::Active as u32);
    }
    #[test]
    fn stale_scores_and_active_sequences_do_not_pass() {
        let i = input(1, "Ollie");
        let mut e = TutorialEngine {
            lesson: 1,
            ..Default::default()
        };
        e.command(CMD_START, 0);
        e.step(i);
        for _ in 0..4 {
            assert_eq!(e.step(i).state, State::Active as u32);
        }
        let mut active = i;
        active.sequence_active = 1;
        e.step(active);
        assert_eq!(e.step(i).state, State::Passed as u32);
        e.command(CMD_RETRY, 0);
        e.step(i);
        assert_eq!(e.step(i).state, State::Active as u32);
    }
    #[test]
    fn stale_cross_district_snapshot_does_not_credit() {
        // Complete a trick outside Skate School, then enter Skate School with delayed score delta.
        let mut e = TutorialEngine {
            lesson: 1,
            ..Default::default()
        };
        e.command(CMD_START, 0);
        let mut out_idle = input(1, "");
        out_idle.position = [0., 0., 0.];
        e.step(out_idle);
        let mut out_old = input(1, "");
        out_old.position = [0., 0., 0.];
        e.step(out_old);
        let mut arrive_stale = input(1, "Ollie");
        assert_eq!(e.step(arrive_stale).state, State::Active as u32);
        // Once riding inside the district and landing a fresh in-district trick, it passes.
        arrive_stale.sequence_active = 1;
        e.step(arrive_stale);
        assert_eq!(e.step(input(1, "Ollie")).state, State::Passed as u32);
    }
    #[test]
    fn score_thresholds_and_composite_objectives() {
        for (n, score) in [(9, 400.), (14, 300.), (16, 1200.)] {
            let mut i = input(n, TRICKS[n]);
            i.line_score = score - 1.;
            i.multiplier = 1.;
            assert_eq!(engine(n).step(i).state, State::Active as u32);
            i.line_score = score;
            assert_eq!(engine(n).step(i).state, State::Passed as u32);
        }
        let mut i = input(11, "Ollie");
        i.sequence_score = 799.;
        assert_eq!(engine(11).step(i).state, State::Active as u32);
        i.sequence_score = 800.;
        assert_eq!(engine(11).step(i).state, State::Passed as u32);
        let mut i = input(14, "Ollie");
        assert_eq!(engine(14).step(i).state, State::Active as u32);
        i = input(9, "Ollie");
        i.line_score = 0.;
        assert_eq!(engine(9).step(i).state, State::Passed as u32);
        i.multiplier = f32::NAN;
        i.line_score = f32::INFINITY;
        assert_eq!(engine(9).step(i).state, State::Active as u32);
    }
    #[test]
    fn commands_and_completion() {
        let mut e = engine(0);
        e.step(input(0, ""));
        e.command(CMD_RETRY, 0);
        assert_eq!(e.state, State::Briefing);
        assert_eq!(e.completed_mask, 1);
        e.command(CMD_SKIP, 0);
        assert_eq!(e.lesson, 1);
        assert_eq!(e.completed_mask, 1);
        e.command(CMD_PREV, 0);
        e.command(CMD_PREV, 0);
        assert_eq!(e.lesson, 0);
        e.command(CMD_STOP, 0);
        assert_eq!(e.state, State::Idle);
        assert_eq!(e.command(99, 0), 0);
        assert_eq!(e.command(CMD_RESTORE, 1 << 20), 0);
        e.command(CMD_RESTORE, ALL_LESSONS & !(1 << 16));
        e.command(CMD_START, 0);
        e.step(input(16, ""));
        e.step(input(16, "Ollie"));
        for _ in 0..32 {
            let mut i = input(16, "Ollie");
            i.dt_seconds = 0.1;
            e.step(i);
        }
        assert_eq!(e.state, State::CompletedAll);
        assert_eq!(e.status().completed_count, 17);
        e.command(CMD_START, 0);
        assert_eq!(e.state, State::CompletedAll);
        e.command(CMD_RESTART, 0);
        assert_eq!(e.completed_mask, 0);
        assert_eq!(e.lesson, 0);
        assert_eq!(e.state, State::Briefing);
    }
    #[test]
    fn preset_formats() {
        assert!(
            UiPreset::Sk8v
                .prompt("Ollie", "Land it.", true)
                .contains("Coach Frank: Nailed it!")
        );
        assert_eq!(
            UiPreset::GtaxSkate.prompt("Ollie", "Land it.", true),
            "Objective complete: Ollie"
        );
        assert_eq!(UiPreset::Rp.prompt("Ollie", "Land it.", false), "Land it.");
        for p in [UiPreset::Sk8v, UiPreset::GtaxSkate, UiPreset::Rp] {
            let mut e = engine(1);
            let mut i = input(1, "Ollie");
            i.preset = p as u32;
            let s = e.step(i);
            assert_eq!(
                text(&s.banner_utf8),
                p.prompt(LESSONS[1].title, LESSONS[1].objective, true)
            );
        }
    }
    #[test]
    fn abi_layout_and_null_exports() {
        use std::mem::offset_of;
        assert_eq!(crate::SKATEV_ABI_VERSION, 8);
        assert_eq!(size_of::<SvTutorialInput>(), 148);
        assert_eq!(offset_of!(SvTutorialInput, position), 12);
        assert_eq!(offset_of!(SvTutorialInput, trick_utf8), 48);
        assert_eq!(offset_of!(SvTutorialInput, dt_seconds), 144);
        assert_eq!(size_of::<SvTutorialStatus>(), 304);
        assert_eq!(offset_of!(SvTutorialStatus, banner_utf8), 24);
        assert_eq!(offset_of!(SvTutorialStatus, lesson_id), 280);
        unsafe {
            assert_eq!(sv_tutorial_command(std::ptr::null_mut(), CMD_START, 0), 0);
            assert_eq!(
                sv_tutorial_step(std::ptr::null_mut(), std::ptr::null(), std::ptr::null_mut()),
                0
            );
        }
        for (n, l) in LESSONS.iter().enumerate() {
            assert_eq!(
                l.id,
                format!(
                    "tut_{}_{:02}",
                    if n < 12 { "sksc" } else { "skpk" },
                    if n < 12 { n + 1 } else { n - 11 }
                )
            );
        }
    }
}

const _: () = {
    assert!(size_of::<SvTutorialInput>() == 148);
    assert!(std::mem::offset_of!(SvTutorialInput, trick_utf8) == 48);
    assert!(std::mem::offset_of!(SvTutorialInput, dt_seconds) == 144);
    assert!(size_of::<SvTutorialStatus>() == 304);
    assert!(std::mem::offset_of!(SvTutorialStatus, banner_utf8) == 24);
    assert!(std::mem::offset_of!(SvTutorialStatus, lesson_id) == 280);
    assert!(crate::SKATEV_ABI_VERSION == 8);
};
