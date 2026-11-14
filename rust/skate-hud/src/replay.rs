//! Skate 3's original Clip / Replay Editor APT HUD (`replay_editor2` +
//! `button_menu` + `button_item2` + Xbox 360 button glyphs).
use crate::apt_movie::Movie;
use crate::apt_vm::{Host, ObjectKind, Value, Vm};
use crate::layout::Layout;
use crate::player::{Assets, DrawCmd, Frame, Vertex, flatten};
use crate::apt_scene;

const BUTTON_MENU_CHAR: i32 = 15;
const BUTTON_ITEM_CHAR: i32 = 24;
const BUTTON_RENDER_CHAR: i32 = 3002;
const SOLID_WHITE_RGBA: &str = "buttons/solid_white.rgba";

// Custom timeline area (`mCustomRender` at `(228.5, 53.0)`, size `817 x 30`).
const TIMELINE_X0: f32 = 228.5;
const TIMELINE_Y0: f32 = 53.0;
const TIMELINE_W: f32 = 817.0;
const TIMELINE_H: f32 = 30.0;

#[allow(dead_code)]
mod state {
    pub const SCRUB_MODE: i32 = 0;
    pub const FOLLOW_EDIT: i32 = 1;
    pub const TRIPOD_EDIT: i32 = 2;
    pub const TAKE_PHOTO: i32 = 3;
    pub const EFFECTS_MODE: i32 = 4;
    pub const REFRESH_EFFECTS: i32 = 5;
    pub const UPDATE_EFFECTS: i32 = 6;
    pub const FREE_CAM: i32 = 7;
    pub const ADVANCED_MODE: i32 = 8;
    pub const MAIN_EDIT: i32 = 9;
    pub const DEFAULT_CAM: i32 = 10;
    pub const UPDATE_ZOOM: i32 = 11;
    pub const START_PLAYBACK: i32 = 18;
    pub const STOP_PLAYBACK: i32 = 19;
    pub const SHOW_TOGGLE: i32 = 21;
    pub const HIDE_TOGGLE: i32 = 22;
    pub const MOVE_PLAYHEAD: i32 = 999;
    pub const MOVE_START_MARKER: i32 = 1000;
    pub const MOVE_END_MARKER: i32 = 1001;
    pub const SHOW_EDITOR: i32 = 1002;
    pub const HIDE_EDITOR: i32 = 1003;
    pub const SHOW_CURSOR: i32 = 1004;
    pub const HIDE_CURSOR: i32 = 1005;
    pub const OVER_CLIP_MARKER: i32 = 1006;
    pub const OVER_CLIP: i32 = 1007;
    pub const HIDE_CLIP_KEY: i32 = 1008;
    pub const OVER_KEY_MARKER: i32 = 1009;
}

/// Active sub-mode inside the Replay Editor (`SvReplayHudState::mode`).
#[repr(u32)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ReplayHudMode {
    #[default]
    Instant = 0,
    Main = 1,
    FollowCam = 2,
    TripodCam = 3,
    FreeCam = 4,
    GameCam = 5,
    Effects = 6,
}

impl ReplayHudMode {
    pub fn from_u32(v: u32) -> Self {
        match v {
            1 => Self::Main,
            2 => Self::FollowCam,
            3 => Self::TripodCam,
            4 => Self::FreeCam,
            5 => Self::GameCam,
            6 => Self::Effects,
            _ => Self::Instant,
        }
    }

    fn state_id(self) -> i32 {
        match self {
            Self::Instant => state::SCRUB_MODE,
            Self::Main => state::MAIN_EDIT,
            Self::FollowCam => state::FOLLOW_EDIT,
            Self::TripodCam => state::TRIPOD_EDIT,
            Self::FreeCam => state::FREE_CAM,
            Self::GameCam => state::DEFAULT_CAM,
            Self::Effects => state::EFFECTS_MODE,
        }
    }
}

/// One edited clip span along the normalized timeline `[0..1]`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ReplayClipSpan {
    pub start: f32,
    pub end: f32,
    pub camera_kind: u32,
    pub selected: bool,
}

/// Host snapshot driving `replay_editor2` and `mCustomRender`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ReplayHudData {
    pub mode: ReplayHudMode,
    pub playing: bool,
    pub hidden: bool,
    pub over_key: bool,
    pub over_clip_marker: bool,
    pub can_show_hide: bool,
    pub selected_clip: i32,
    pub playhead: f32,
    pub trim_in: f32,
    pub trim_out: f32,
    pub encoded_time: String,
    pub zoom_text: String,
    pub title_text: String,
    pub shake_type: u32,
    pub speed_pct: i32,
    pub clips: Vec<ReplayClipSpan>,
    pub keys: Vec<(f32, bool)>,
    pub markers: Vec<f32>,
}

struct ReplayBindings {
    movie: Movie,
    data: ReplayHudData,
    /// Active normalized `[0..1]` value read by `FEReplayMenu.GetPlayheadPosition`
    /// during `kMovePlayHead`, `kMoveStartMarker`, and `kMoveEndMarker`.
    query_pos: f32,
}

impl ReplayBindings {
    fn sync_text_height(movie: &Movie, vm: &mut Vm, object: usize) -> Result<(), String> {
        if let Some(ch) = movie
            .instances
            .get(&object)
            .and_then(|i| movie.characters.get(&i.character))
        {
            if let Some(text) = &ch.text {
                let h = text["font_height"].as_f64().unwrap_or(16.0);
                vm.set(object, "textHeight", Value::Number(h))?;
            }
        }
        Ok(())
    }

    fn bind_controls(&mut self, vm: &mut Vm) -> Result<(), String> {
        let targets: Vec<_> = self
            .movie
            .instances
            .iter()
            .filter_map(|(&id, inst)| {
                if self.movie.characters.get(&inst.character).is_some_and(|c| c.text.is_some()) {
                    Some((id, "", false))
                } else {
                    match inst.character {
                        BUTTON_MENU_CHAR => Some((id, "button_menu", false)),
                        BUTTON_ITEM_CHAR => Some((id, "button_item2", true)),
                        _ => None,
                    }
                }
            })
            .collect();
        for (id, class_name, is_button_item) in targets {
            if class_name.is_empty() {
                if !matches!(vm.get(id, "textHeight"), Value::Number(_)) {
                    Self::sync_text_height(&self.movie, vm, id)?;
                }
                continue;
            }
            if vm.objects.get(id).is_some_and(|o| o.prototype.is_some()) {
                continue;
            }
            if is_button_item {
                vm.set(id, "_width", Value::Number(32.0))?;
                vm.set(id, "_height", Value::Number(32.0))?;
            }
            if let Value::Object(class) = vm.get(vm.global, class_name) {
                if let Value::Object(proto) = vm.get(class, "prototype") {
                    vm.objects[id].prototype = Some(proto);
                }
                vm.set(id, "__init_ctor", Value::Object(class))?;
                let res = vm.call_method(id, "__init_ctor", Vec::new(), self);
                vm.objects[id].fields.remove("__init_ctor");
                res?;
            }
        }
        Ok(())
    }

    fn sync_button_render(&mut self, vm: &mut Vm, object: usize) -> Result<(), String> {
        if self.movie.instances.get(&object).is_some_and(|i| i.character == BUTTON_RENDER_CHAR) {
            let label = vm.get(object, "_test");
            if let Value::Text(name) = label {
                if !name.is_empty() {
                    let _ = self.movie.method(vm, object, "gotoAndStop", &[Value::Text(name)]);
                }
            }
        }
        Ok(())
    }
}

impl Host for ReplayBindings {
    fn property_changed(&mut self, vm: &mut Vm, object: usize, key: &str) -> Result<(), String> {
        if key == "text" || key == "autoSize" {
            self.movie.text_changed(vm, object)?;
            Self::sync_text_height(&self.movie, vm, object)?;
        } else if key == "_test" {
            self.sync_button_render(vm, object)?;
        }
        Ok(())
    }

    fn call(&mut self, vm: &mut Vm, object: usize, method: &str, args: Vec<Value>) -> Result<Value, String> {
        match self.movie.method(vm, object, method, &args) {
            Ok(true) => {
                if matches!(method, "gotoAndPlay" | "gotoAndStop") {
                    self.bind_controls(vm)?;
                }
                return Ok(Value::Undefined);
            }
            Ok(false) => {}
            Err(e) if e.contains("lacks label") => return Ok(Value::Undefined),
            Err(e) => return Err(e),
        }
        let native = match vm.objects.get(object).map(|o| &o.kind) {
            Some(ObjectKind::Native(name)) => name.as_str(),
            _ => "global",
        };
        let arg = |i: usize| args.get(i).cloned().unwrap_or_default();
        let norm = |v: f32| if v.is_finite() { v.clamp(0.0, 1.0) as f64 } else { 0.0 };
        match (native, method) {
            ("Math", "floor") => Ok(Value::Number(arg(0).number().floor())),
            ("Math", "round") => Ok(Value::Number(arg(0).number().round())),
            ("Math", "ceil") => Ok(Value::Number(arg(0).number().ceil())),
            ("Math", "abs") => Ok(Value::Number(arg(0).number().abs())),
            ("Math", "min") => Ok(Value::Number(arg(0).number().min(arg(1).number()))),
            ("Math", "max") => Ok(Value::Number(arg(0).number().max(arg(1).number()))),
            ("global", "ASSetPropFlags") | ("Object", "registerClass") => Ok(Value::Undefined),
            ("FELanguage", "GetLanguage") => Ok(Value::Text("english".into())),
            ("Game", "IsDemo") => Ok(Value::Bool(false)),
            ("ScreenManager", "PlatformCurrent") => Ok(Value::Number(0.0)), // Xbox 360
            ("ScreenManager", "RegisterScreen" | "OnLoaded" | "IntroComplete" | "OutroComplete") => {
                Ok(Value::Undefined)
            }
            ("FEReplayMenu", "CanSaveMedia" | "IsEncodedLengthValid") => Ok(Value::Bool(true)),
            ("FEReplayMenu", "CanDisplayShowHide") => Ok(Value::Bool(self.data.can_show_hide)),
            ("FEReplayMenu", "IsInstantReplayMode") => {
                Ok(Value::Bool(self.data.mode == ReplayHudMode::Instant))
            }
            ("FEReplayMenu", "GetPlayheadPosition") => Ok(Value::Number(norm(self.query_pos))),
            ("FEReplayMenu", "GetCursorPosition") => Ok(Value::Number(norm(self.data.playhead))),
            ("FEReplayMenu", "GetStartHeadPosition") => Ok(Value::Number(norm(self.data.trim_in))),
            ("FEReplayMenu", "GetEndHeadPosition") => Ok(Value::Number(norm(self.data.trim_out))),
            ("FEReplayMenu", "GetEncodedLength") => {
                Ok(Value::Text(self.data.encoded_time.clone()))
            }
            ("FEReplayMenu", "GetCameraZoomString") => {
                Ok(Value::Text(self.data.zoom_text.clone()))
            }
            ("FEReplayMenu", "GetShakeCamType") => Ok(Value::Number(self.data.shake_type as f64)),
            ("FEReplayMenu", "GetSimpleSpeedSetting") => {
                Ok(Value::Number(self.data.speed_pct as f64))
            }
            ("FEReplayMenu", "GetCurrentCameraEditIndex" | "GetEditIndex") => {
                Ok(Value::Number(self.data.mode as u32 as f64))
            }
            (
                "FEReplayMenu",
                "Ready" | "InstantDone" | "UpdateReplayEditorReady" | "RefreshCursor",
            ) => Ok(Value::Undefined),
            ("movie:72" | "movie:76" | "movie:81", _) => Ok(Value::Undefined),
            (_, "setTextFormat" | "Enable" | "Destroy") => Ok(Value::Undefined),
            _ => Err(format!("Unimplemented Replay HUD binding {native}.{method}")),
        }
    }
}

/// The running Skate 3 Replay Editor HUD (`replay_editor2`).
pub struct ReplayHudPlayer {
    vm: Vm,
    bindings: ReplayBindings,
    controller: usize,
    applied: Option<ReplayHudData>,
}

impl ReplayHudPlayer {
    pub fn new(assets: &Assets) -> Result<Self, String> {
        let mut vm = Vm::new();
        for name in [
            "MovieClip",
            "FELanguage",
            "Math",
            "Object",
            "TextFormat",
            "ScreenManager",
            "Game",
            "FEReplayMenu",
        ] {
            let object = vm.object(ObjectKind::Native(name.into()));
            let prototype = vm.object(ObjectKind::Plain);
            vm.set(object, "prototype", Value::Object(prototype))?;
            vm.set(vm.global, name, Value::Object(object))?;
        }
        if let Value::Object(sm) = vm.get(vm.global, "ScreenManager") {
            vm.set(sm, "Platform_PS3", Value::Number(1.0))?;
            vm.set(sm, "PS3_UseCircleEnter", Value::Bool(false))?;
        }
        let mut bindings = ReplayBindings {
            movie: Movie::load(assets.source())?,
            data: ReplayHudData::default(),
            query_pos: 0.0,
        };
        let initial: Vec<_> = bindings
            .movie
            .characters
            .values()
            .flat_map(|c| &c.frames)
            .flat_map(|f| &f.controls)
            .filter(|c| c.type_name == "do_init_action")
            .map(|c| c.actions_offset)
            .collect();
        for offset in initial {
            let code = bindings.movie.actions[&offset.to_string()].clone();
            vm.run(&code, &mut bindings)?;
        }
        vm.begin_update();
        bindings.movie.initialize(&mut vm)?;
        let mut player = Self {
            vm,
            bindings,
            controller: usize::MAX,
            applied: None,
        };
        player.bind_registered_controls()?;
        player.drain()?;
        let Value::Object(controller) = player.vm.get(player.bindings.movie.root, "screen") else {
            return Err("Replay HUD controller was not constructed".into());
        };
        player.controller = controller;
        // Skip root fade-in frames (0..14) so mReplay starts fully opaque.
        let root = player.bindings.movie.root;
        player.bindings.movie.seek(&mut player.vm, root, 14, 0)?;
        player.drain()?;
        player.hide_unused_panels();
        Ok(player)
    }

    fn bind_registered_controls(&mut self) -> Result<(), String> {
        self.bindings.bind_controls(&mut self.vm)
    }

    fn drain(&mut self) -> Result<(), String> {
        let mut calls = 0;
        while let Some((object, offset)) = self.bindings.movie.pending.pop_front() {
            calls += 1;
            if calls > 4096 {
                return Err("Replay HUD frame action limit".into());
            }
            if !self.bindings.movie.instances.contains_key(&object) {
                continue;
            }
            let code = self
                .bindings
                .movie
                .actions
                .get(&offset.to_string())
                .ok_or("Missing Replay HUD action block")?
                .clone();
            self.vm.run_on(object, &code, &mut self.bindings)?;
            self.bind_registered_controls()?;
        }
        Ok(())
    }

    fn call_state(&mut self, state_id: i32) -> Result<(), String> {
        self.vm.call_method(
            self.controller,
            "SetState",
            vec![Value::Number(state_id as f64)],
            &mut self.bindings,
        )?;
        self.drain()?;
        Ok(())
    }

    fn hide_unused_panels(&mut self) {
        if let Value::Object(mreplay) = self.vm.get(self.bindings.movie.root, "mReplay") {
            for field in ["mBlack", "panelEdit", "panelHelp", "mEffects"] {
                if let Value::Object(id) = self.vm.get(mreplay, field) {
                    let _ = self.vm.set(id, "_visible", Value::Bool(false));
                }
            }
        }
    }

    pub fn update(&mut self, data: &ReplayHudData) -> Result<(), String> {
        self.vm.begin_update();
        let prev = self.applied.clone();
        self.bindings.data = data.clone();

        let mode_changed = prev.as_ref().is_none_or(|p| p.mode != data.mode);
        let hidden_changed = prev.as_ref().is_none_or(|p| p.hidden != data.hidden);
        let playing_changed = prev.as_ref().is_none_or(|p| p.playing != data.playing);
        let trim_in_changed = prev.as_ref().is_none_or(|p| (p.trim_in - data.trim_in).abs() > 1e-4);
        let trim_out_changed = prev.as_ref().is_none_or(|p| (p.trim_out - data.trim_out).abs() > 1e-4);
        let playhead_changed = prev.as_ref().is_none_or(|p| (p.playhead - data.playhead).abs() > 1e-4);
        let zoom_changed = prev.as_ref().is_none_or(|p| p.zoom_text != data.zoom_text);

        if mode_changed {
            self.call_state(data.mode.state_id())?;
        }
        if hidden_changed {
            self.call_state(if data.hidden { state::HIDE_EDITOR } else { state::SHOW_EDITOR })?;
            self.vm.call_method(
                self.controller,
                "SetShowHideText",
                vec![Value::Text(if data.hidden {
                    "ID_REPLAY_EDITOR_SHOW".into()
                } else {
                    "ID_REPLAY_EDITOR_HIDE".into()
                })],
                &mut self.bindings,
            )?;
        }
        if playing_changed {
            self.call_state(if data.playing {
                state::START_PLAYBACK
            } else {
                state::STOP_PLAYBACK
            })?;
        }
        if trim_in_changed || mode_changed {
            self.bindings.query_pos = data.trim_in;
            self.call_state(state::MOVE_START_MARKER)?;
        }
        if trim_out_changed || mode_changed {
            self.bindings.query_pos = data.trim_out;
            self.call_state(state::MOVE_END_MARKER)?;
        }
        if playhead_changed || mode_changed {
            self.bindings.query_pos = data.playhead;
            self.call_state(state::MOVE_PLAYHEAD)?;
        }
        if zoom_changed || mode_changed {
            self.call_state(state::UPDATE_ZOOM)?;
        }
        if !data.title_text.is_empty() {
            if let Value::Object(mreplay) = self.vm.get(self.bindings.movie.root, "mReplay") {
                if let Value::Object(t) = self.vm.get(mreplay, "mTextReplay") {
                    let _ = self.vm.set(t, "text", Value::Text(data.title_text.clone()));
                    let _ = self.bindings.movie.text_changed(&mut self.vm, t);
                }
            }
        }
        if prev.as_ref().is_none_or(|p| p.encoded_time != data.encoded_time) || mode_changed {
            self.vm.call_method(self.controller, "RefreshEncodedLength", Vec::new(), &mut self.bindings)?;
            self.drain()?;
        }

        self.hide_unused_panels();
        self.bindings.movie.advance(&mut self.vm)?;
        self.drain()?;
        self.bind_registered_controls()?;
        self.vm.collect(self.bindings.movie.instances.keys().copied())?;
        self.applied = Some(data.clone());
        Ok(())
    }

    pub fn frame(&self, assets: &Assets, layout: &Layout) -> Result<Frame, String> {
        let mut frame = Frame {
            layout: Some(*layout),
            draws: Vec::new(),
            vertices: Vec::new(),
        };
        if !self.bindings.data.hidden {
            append_timeline_quads(&mut frame, assets, layout, &self.bindings.data);
        }
        let draws = apt_scene::draw(&self.bindings.movie, &self.vm, assets.shapes())?;
        let movie_frame = flatten(draws, assets, layout);
        let vbase = frame.vertices.len() as u32;
        frame.vertices.extend_from_slice(&movie_frame.vertices);
        frame.draws.extend(movie_frame.draws.into_iter().map(|d| DrawCmd {
            first: d.first + vbase,
            ..d
        }));
        Ok(frame)
    }

    pub fn visible_text(&self) -> Vec<String> {
        crate::movie::visible_text(&self.bindings.movie, &self.vm)
    }
}

fn camera_colour(kind: u32) -> [f32; 4] {
    match kind {
        1 => [0.22, 0.56, 0.96, 0.90], // Follow
        2 => [0.30, 0.80, 0.35, 0.90], // Tripod
        3 => [0.98, 0.60, 0.14, 0.90], // Free
        4 => [0.55, 0.75, 1.00, 0.90],
        5 => [0.65, 1.00, 0.65, 0.90],
        6 => [1.00, 0.82, 0.45, 0.90],
        _ => [0.55, 0.55, 0.58, 0.85], // Game / default
    }
}

fn push_solid_rect(
    frame: &mut Frame,
    texture: u32,
    layout: &Layout,
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    rgba: [f32; 4],
) {
    if !x0.is_finite() || !y0.is_finite() || !x1.is_finite() || !y1.is_finite() || x1 <= x0 || y1 <= y0 {
        return;
    }
    let p0 = layout.to_pixels([x0, y0]);
    let p1 = layout.to_pixels([x1, y0]);
    let p2 = layout.to_pixels([x1, y1]);
    let p3 = layout.to_pixels([x0, y1]);
    let first = frame.vertices.len() as u32;
    frame.vertices.extend_from_slice(&[
        Vertex { x: p0[0], y: p0[1], u: 0.5, v: 0.5 },
        Vertex { x: p1[0], y: p1[1], u: 0.5, v: 0.5 },
        Vertex { x: p2[0], y: p2[1], u: 0.5, v: 0.5 },
        Vertex { x: p0[0], y: p0[1], u: 0.5, v: 0.5 },
        Vertex { x: p2[0], y: p2[1], u: 0.5, v: 0.5 },
        Vertex { x: p3[0], y: p3[1], u: 0.5, v: 0.5 },
    ]);
    frame.draws.push(DrawCmd {
        texture,
        first,
        count: 6,
        multiply: rgba,
        add: [0.0; 4],
    });
}

fn append_timeline_quads(frame: &mut Frame, assets: &Assets, layout: &Layout, data: &ReplayHudData) {
    let Some(tex) = assets.texture_index(SOLID_WHITE_RGBA) else {
        return;
    };
    let tx = |t: f32| TIMELINE_X0 + if t.is_finite() { t.clamp(0.0, 1.0) } else { 0.0 } * TIMELINE_W;
    // Track background + trimmed out-of-range shade.
    push_solid_rect(
        frame,
        tex,
        layout,
        TIMELINE_X0,
        TIMELINE_Y0,
        TIMELINE_X0 + TIMELINE_W,
        TIMELINE_Y0 + TIMELINE_H,
        [0.08, 0.08, 0.10, 0.78],
    );
    if data.mode == ReplayHudMode::Instant {
        push_solid_rect(
            frame,
            tex,
            layout,
            TIMELINE_X0,
            TIMELINE_Y0 + 3.0,
            tx(data.playhead),
            TIMELINE_Y0 + TIMELINE_H - 3.0,
            [0.85, 0.68, 0.18, 0.90],
        );
    } else {
        for clip in &data.clips {
            let x0 = tx(clip.start);
            let x1 = tx(clip.end);
            if clip.selected {
                push_solid_rect(
                    frame,
                    tex,
                    layout,
                    x0,
                    TIMELINE_Y0 + 1.0,
                    x1,
                    TIMELINE_Y0 + TIMELINE_H - 1.0,
                    [1.0, 1.0, 1.0, 0.92],
                );
            }
            push_solid_rect(
                frame,
                tex,
                layout,
                x0 + 1.0,
                TIMELINE_Y0 + 4.0,
                (x1 - 1.0).max(x0 + 1.5),
                TIMELINE_Y0 + TIMELINE_H - 4.0,
                camera_colour(clip.camera_kind),
            );
        }
        // Shade outside [trim_in..trim_out].
        push_solid_rect(
            frame,
            tex,
            layout,
            TIMELINE_X0,
            TIMELINE_Y0,
            tx(data.trim_in),
            TIMELINE_Y0 + TIMELINE_H,
            [0.0, 0.0, 0.0, 0.55],
        );
        push_solid_rect(
            frame,
            tex,
            layout,
            tx(data.trim_out),
            TIMELINE_Y0,
            TIMELINE_X0 + TIMELINE_W,
            TIMELINE_Y0 + TIMELINE_H,
            [0.0, 0.0, 0.0, 0.55],
        );
        for &(k, selected) in &data.keys {
            let x = tx(k);
            let rgba = if selected {
                [1.0, 0.95, 0.35, 1.0]
            } else {
                [1.0, 0.45, 0.12, 0.95]
            };
            push_solid_rect(frame, tex, layout, x - 2.0, TIMELINE_Y0 + 2.0, x + 2.0, TIMELINE_Y0 + TIMELINE_H - 2.0, rgba);
        }
        for &m in &data.markers {
            let x = tx(m);
            push_solid_rect(frame, tex, layout, x - 1.5, TIMELINE_Y0 + 1.0, x + 1.5, TIMELINE_Y0 + 8.0, [1.0, 1.0, 1.0, 0.95]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mode_maps_to_expected_apt_states() {
        assert_eq!(ReplayHudMode::from_u32(0).state_id(), state::SCRUB_MODE);
        assert_eq!(ReplayHudMode::from_u32(1).state_id(), state::MAIN_EDIT);
        assert_eq!(ReplayHudMode::from_u32(2).state_id(), state::FOLLOW_EDIT);
        assert_eq!(ReplayHudMode::from_u32(3).state_id(), state::TRIPOD_EDIT);
        assert_eq!(ReplayHudMode::from_u32(4).state_id(), state::FREE_CAM);
        assert_eq!(ReplayHudMode::from_u32(5).state_id(), state::DEFAULT_CAM);
        assert_eq!(ReplayHudMode::from_u32(6).state_id(), state::EFFECTS_MODE);
    }

    #[test]
    #[ignore = "requires local Skate 3 prepared assets; set GTAXSKATE_TEST_ASSETS"]
    fn replay_movie_loads_and_renders_all_modes() {
        let dir = std::env::var_os("GTAXSKATE_TEST_ASSETS")
            .map(std::path::PathBuf::from)
            .expect("set GTAXSKATE_TEST_ASSETS to Skate 3 assets folder");
        let assets = Assets::load_replay(&Assets::replay_dir_for(&dir)).expect("replay HUD assets");
        let mut player = ReplayHudPlayer::new(&assets).expect("init replay_editor2");
        let layout = Layout::compute(1920, 1080, 1.0, 0.0);
        for (i, mode) in [
            ReplayHudMode::Instant,
            ReplayHudMode::Main,
            ReplayHudMode::FollowCam,
            ReplayHudMode::TripodCam,
            ReplayHudMode::FreeCam,
            ReplayHudMode::GameCam,
            ReplayHudMode::Effects,
        ]
        .into_iter()
        .enumerate()
        {
            let data = ReplayHudData {
                mode,
                playing: i % 2 == 0,
                hidden: false,
                can_show_hide: true,
                playhead: 0.35,
                trim_in: 0.10,
                trim_out: 0.90,
                encoded_time: "0:03.5 / 0:12.0".into(),
                zoom_text: "35mm".into(),
                clips: vec![ReplayClipSpan { start: 0.0, end: 1.0, camera_kind: 1, selected: true }],
                keys: vec![(0.35, true)],
                markers: vec![0.5],
                ..Default::default()
            };
            for _ in 0..16 {
                player.update(&data).expect("update mode");
            }
            let frame = player.frame(&assets, &layout).expect("flatten frame");
            assert!(!frame.draws.is_empty(), "mode {mode:?} produced empty draws");
            let tex_names: std::collections::BTreeSet<&str> = frame
                .draws
                .iter()
                .map(|d| assets.textures[d.texture as usize].name.as_str())
                .collect();
            let btns: Vec<_> = tex_names.iter().filter(|n| n.starts_with("buttons/")).copied().collect();
            eprintln!("MODE={mode:?} draws={} btns={:?} texts={:?}", frame.draws.len(), btns, player.visible_text());
        }
    }
}
