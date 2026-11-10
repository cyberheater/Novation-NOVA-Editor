//! Front panel drawing and control bindings (port of the web editor's panel view).
use crate::params::{self, N, Param};
use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, Ui, vec2};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const PW: f32 = 1878.0;
/// Panel caption/title size multiplier: larger on the iPad (1.25 is the most before neighbouring captions touch).
#[cfg(target_os = "ios")]
const TEXT_BOOST: f32 = 1.25;
#[cfg(not(target_os = "ios"))]
const TEXT_BOOST: f32 = 1.0;
pub const PH: f32 = 940.0;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct Assign {
    pub off: usize,
    #[serde(default)]
    pub min: i32,
    #[serde(default = "d127")]
    pub max: i32,
}
fn d127() -> i32 {
    127
}

pub struct Target {
    /// true: offset into the performance body instead of the program
    pub perf: bool,
    pub off: usize,
    pub min: i32,
    pub max: i32,
    pub name: String,
}

#[derive(Clone, Copy, PartialEq)]
pub enum Kind {
    Plain,
    Osc(usize),
    Wave(&'static str),
    Tune(&'static str),
    Dest(&'static str),
    Fx(&'static str),
    Env(usize),
    Lfo(usize),
    FDest(&'static str),
    FSrc(&'static str),
    MSrc(&'static str),
    Part(usize),
    PPage(&'static str),
    Toggle(u8),
    Slope(u8),
    FType(u8),
    LWave(u8),
    /// arpeggiator on/off, latch, mute
    Arp(u8),
}

// ---- pure mapping helpers (program byte <-> button state) ----
/// Filter slope byte: 0 = 12dB, 1 = 18dB, 2 = 24dB.
pub const FILTER_SLOPE: usize = 0xd0;
/// Filter type byte: 3 Low, 4 Band, 5 High, 6..=14 special types.
pub const FILTER_TYPE: usize = 0xcf;
/// LFO waveform bytes: LFO 1 at 0xa1 (16..19), LFO 2 at 0xab (20..23); order square, saw, triangle, s/h.
pub const LFO_WAVE: [usize; 2] = [0xa1, 0xab];
pub const LFO_WAVE_BASE: [u8; 2] = [16, 20];
/// Button argument that stands for "special" filter types.
pub const FTYPE_SPECIAL: u8 = 255;
pub const SPECIAL_TYPES: [&str; 9] = [
    "Res LPF", "Res BPF", "Res HPF", "Notch", "LPF+LPF", "BPF+BPF", "HPF+HPF", "LPF+BPF", "BPF+HPF",
];

pub fn slope_lit(byte: u8, btn: u8) -> bool {
    byte == btn
}
pub fn ftype_lit(byte: u8, btn: u8) -> bool {
    if btn == FTYPE_SPECIAL { byte >= 6 } else { byte == btn }
}
/// New filter type byte after pressing a type button (special keeps an existing special type).
pub fn ftype_click(byte: u8, btn: u8) -> u8 {
    if btn == FTYPE_SPECIAL { if byte >= 6 { byte } else { 6 } } else { btn }
}
pub fn lfo_wave_lit(byte: u8, lfo: usize, w: u8) -> bool {
    byte == LFO_WAVE_BASE[lfo.min(1)] + w
}
pub fn lfo_wave_value(lfo: usize, w: u8) -> u8 {
    LFO_WAVE_BASE[lfo.min(1)] + w
}
pub fn ftype_name(byte: u8) -> String {
    match byte {
        3 => "Low".into(),
        4 => "Band".into(),
        5 => "High".into(),
        6..=14 => format!("special: {}", SPECIAL_TYPES[(byte - 6) as usize]),
        _ => format!("raw {byte}"),
    }
}

pub struct Kn {
    pub id: &'static str,
    pub x: f32,
    pub y: f32,
    pub cap: &'static str,
}
pub struct Bt {
    pub id: &'static str,
    pub x: f32,
    pub y: f32,
    pub cap: &'static str,
    pub kind: Kind,
}
const fn k(id: &'static str, x: f32, y: f32, cap: &'static str) -> Kn {
    Kn { id, x, y, cap }
}
const fn b(id: &'static str, x: f32, y: f32, cap: &'static str) -> Bt {
    Bt { id, x, y, cap, kind: Kind::Plain }
}
const fn bk(id: &'static str, x: f32, y: f32, cap: &'static str, kind: Kind) -> Bt {
    Bt { id, x, y, cap, kind }
}

pub const KN: [Kn; 20] = [
    k("portamento", 211.0, 613.0, "portamento"),
    k("tune", 360.0, 613.0, "tune"),
    k("level", 578.0, 470.0, "level"),
    k("moddepth", 578.0, 613.0, "mod depth"),
    k("f_freq", 798.0, 490.0, "frequency"),
    k("f_res", 942.0, 490.0, "resonance"),
    k("f_over", 798.0, 613.0, "overdrive"),
    k("f_mod", 1058.0, 613.0, "mod depth"),
    k("l_speed", 1568.0, 362.0, "speed"),
    k("l_delay", 1678.0, 362.0, "delay"),
    k("e_att", 1348.0, 620.0, "attack"),
    k("e_dec", 1458.0, 620.0, "decay"),
    k("e_sus", 1568.0, 620.0, "sustain"),
    k("e_rel", 1678.0, 620.0, "release"),
    k("mastervol", 170.0, 796.0, "master volume"),
    k("arp_speed", 498.0, 796.0, "speed"),
    k("part_level", 1148.0, 796.0, "level"),
    k("effect", 1660.0, 796.0, "effect"),
    k("fast1", 1272.0, 90.0, "fast data"),
    k("fast2", 1272.0, 226.0, ""),
];

pub fn buttons() -> Vec<Bt> {
    use Kind::*;
    vec![
        b("perf", 170.0, 206.0, "perf"), b("prog", 252.0, 206.0, "prog"), b("write", 488.0, 206.0, "write"),
        b("midi", 1580.0, 56.0, "midi"), b("dump", 1580.0, 116.0, "dump"), b("list", 1580.0, 176.0, "list"), b("seq", 1580.0, 236.0, "seq"),
        b("pageU", 590.0, 135.0, "page \u{25b2}"), b("pageD", 590.0, 190.0, "page \u{25bc}"),
        b("bankU", 1370.0, 78.0, "bank \u{25b2}"), b("bankD", 1370.0, 132.0, "bank \u{25bc}"),
        b("progU", 1370.0, 186.0, "prog \u{25b2}"), b("progD", 1370.0, 240.0, "prog \u{25bc}"),
        b("o_menu", 170.0, 330.0, "menu"), b("o_solo", 170.0, 394.0, "solo"), b("o_copy", 170.0, 458.0, "copy"),
        bk("osc1", 252.0, 330.0, "osc 1", Osc(0)), bk("osc2", 324.0, 330.0, "osc 2", Osc(1)), bk("osc3", 396.0, 330.0, "osc 3", Osc(2)),
        bk("noise", 252.0, 394.0, "noise", Osc(3)), bk("m13", 324.0, 394.0, "1*3", Osc(4)), bk("m23", 396.0, 394.0, "2*3", Osc(5)),
        bk("w_square", 252.0, 458.0, "square", Wave("square")), bk("w_saw", 324.0, 458.0, "saw", Wave("saw")),
        bk("w_special", 396.0, 458.0, "special", Wave("special")),
        bk("t_oct", 252.0, 522.0, "octave", Tune("octave")), bk("t_semi", 324.0, 522.0, "semitone", Tune("semitone")),
        bk("t_fine", 396.0, 522.0, "fine", Tune("fine")),
        bk("d_mix", 488.0, 330.0, "mix", Dest("mix")), bk("d_pitch", 488.0, 394.0, "pitch", Dest("pitch")),
        bk("d_width", 488.0, 458.0, "width", Dest("width")), bk("d_sync", 488.0, 522.0, "sync", Dest("sync")),
        bk("d_soften", 488.0, 586.0, "soften", Dest("soften")),
        bk("s_lfo1", 668.0, 330.0, "lfo 1", MSrc("lfo1")), bk("s_lfo2", 668.0, 394.0, "lfo 2", MSrc("lfo2")), bk("s_env2", 668.0, 458.0, "env 2", MSrc("env2")),
        bk("s_env3", 668.0, 522.0, "env 3", MSrc("env3")), bk("s_wheel", 668.0, 586.0, "wheel", MSrc("wheel")),
        b("f_menu", 762.0, 330.0, "menu"), bk("f12", 834.0, 330.0, "12dB", Slope(0)), bk("f18", 906.0, 330.0, "18dB", Slope(1)),
        bk("f24", 978.0, 330.0, "24dB", Slope(2)), bk("f_sp", 762.0, 394.0, "special", FType(255)), bk("f_lo", 834.0, 394.0, "low", FType(3)),
        bk("f_bd", 906.0, 394.0, "band", FType(4)), bk("f_hi", 978.0, 394.0, "high", FType(5)),
        bk("fd_freq", 1058.0, 330.0, "frequency", FDest("freq")), bk("fd_res", 1058.0, 394.0, "resonance", FDest("res")),
        bk("fs_lfo1", 1148.0, 330.0, "lfo 1", FSrc("lfo1")), bk("fs_lfo2", 1148.0, 394.0, "lfo 2", FSrc("lfo2")), bk("fs_env2", 1148.0, 458.0, "env 2", FSrc("env2")),
        bk("fs_env3", 1148.0, 522.0, "env 3", FSrc("env3")), bk("fs_wheel", 1148.0, 586.0, "wheel", FSrc("wheel")),
        b("l_menu", 1242.0, 330.0, "menu"), bk("l_1", 1314.0, 330.0, "lfo 1", Lfo(0)), bk("l_2", 1386.0, 330.0, "lfo 2", Lfo(1)),
        bk("l_sq", 1242.0, 394.0, "square", LWave(0)), bk("l_saw", 1314.0, 394.0, "saw", LWave(1)), bk("l_tri", 1386.0, 394.0, "tri", LWave(2)),
        bk("l_sh", 1458.0, 394.0, "s/h", LWave(3)),
        b("e_menu", 1242.0, 520.0, "menu"), b("e_trig", 1314.0, 520.0, "trigger"), bk("e_1", 1386.0, 520.0, "env 1", Env(0)),
        bk("e_2", 1458.0, 520.0, "env 2", Env(1)), bk("e_3", 1530.0, 520.0, "env 3", Env(2)),
        b("a_menu", 322.0, 764.0, "menu"), bk("a_onoff", 394.0, 764.0, "on/off", Arp(0)), bk("a_latch", 322.0, 828.0, "latch", Arp(1)),
        bk("a_mute", 394.0, 828.0, "mute", Arp(2)),
        bk("p1", 616.0, 764.0, "1", Part(0)), bk("p2", 688.0, 764.0, "2", Part(1)), bk("p3", 760.0, 764.0, "3", Part(2)), bk("p4", 832.0, 764.0, "4", Part(3)),
        bk("p5", 904.0, 764.0, "5", Part(4)), bk("p6", 976.0, 764.0, "6", Part(5)), bk("p_solo", 1048.0, 764.0, "solo", Toggle(2)),
        bk("p_tune", 616.0, 828.0, "tune", PPage("tune")), bk("p_vel", 688.0, 828.0, "velocity", PPage("vel")), bk("p_io", 760.0, 828.0, "in/out", PPage("io")),
        bk("p_midi", 832.0, 828.0, "midi", PPage("midi")), bk("p_poly", 904.0, 828.0, "poly", PPage("poly")), bk("p_range", 976.0, 828.0, "range", PPage("range")),
        bk("p_mute", 1048.0, 828.0, "mute", Toggle(1)),
        b("x_menu", 1268.0, 764.0, "menu"), bk("x_delay", 1352.0, 764.0, "delay", Fx("delay")),
        bk("x_reverb", 1436.0, 764.0, "reverb", Fx("reverb")), bk("x_chorus", 1520.0, 764.0, "chorus..", Fx("chorus")),
        b("x_copy", 1268.0, 828.0, "copy"), bk("x_dist", 1352.0, 828.0, "dist/EQ", Fx("dist")),
        bk("x_pan", 1436.0, 828.0, "pan..", Fx("pan")), bk("x_voc", 1520.0, 828.0, "vocoder", Fx("vocoder")),
    ]
}

/// label, x, y (heading, left/center), band x, y, w, h
const SECT: [(&str, f32, f32, f32, f32, f32, f32); 7] = [
    ("Oscillators", 0.0, 0.0, 128.0, 278.0, 582.0, 420.0),
    ("Filters", 0.0, 0.0, 720.0, 278.0, 470.0, 420.0),
    ("LFOs", 0.0, 0.0, 1200.0, 278.0, 544.0, 176.0),
    ("Envelopes", 0.0, 0.0, 1200.0, 462.0, 544.0, 236.0),
    ("Arpeggiator", 0.0, 0.0, 280.0, 712.0, 284.0, 184.0),
    ("Part Edit", 0.0, 0.0, 574.0, 712.0, 640.0, 184.0),
    ("Effects", 0.0, 0.0, 1224.0, 712.0, 520.0, 184.0),
];

pub struct PanelState {
    pub osc: usize,
    pub tune: &'static str,
    pub dest: &'static str,
    pub fx: &'static str,
    pub env: usize,
    pub lfo: usize,
    pub fdest: &'static str,
    pub fsrc: &'static str,
    pub msrc: &'static str,
    pub part: Option<usize>,
    pub ppage: &'static str,
    pub perf_mode: bool,
    /// Some(part) when effect bytes live in the performance part block (Effects used = Part)
    pub fx_part: Option<usize>,
    drag: Option<(String, f32, i32)>,
    scroll_acc: f32,
    /// lower Fast Data knob: drag/scroll accumulator and its pointer angle (radians)
    fast_acc: f32,
    fast_turn: f32,
}
impl Default for PanelState {
    fn default() -> Self {
        PanelState { osc: 0, tune: "octave", dest: "pitch", fx: "chorus", env: 0, lfo: 0, fdest: "freq", fsrc: "lfo1", msrc: "lfo1", part: None, ppage: "tune", perf_mode: false, fx_part: None, drag: None, scroll_acc: 0.0, fast_acc: 0.0, fast_turn: 0.0 }
    }
}
impl PanelState {
    fn osc_base(&self) -> usize {
        0x16 + 0x22 * self.osc
    }
}

fn tgt(off: usize, min: i32, max: i32, name: impl Into<String>) -> Option<Target> {
    Some(Target { perf: false, off, min, max, name: name.into() })
}

fn src_label(s: &str) -> &'static str {
    match s {
        "lfo1" => "LFO 1",
        "lfo2" => "LFO 2",
        "env2" => "Env 2",
        "env3" => "Env 3",
        _ => "Wh",
    }
}

pub fn target_of(
    id: &str,
    st: &PanelState,
    assign: &BTreeMap<String, Assign>,
    params: &[Param],
) -> Option<Target> {
    if let Some(a) = assign.get(id) {
        return tgt(a.off, a.min, a.max, format!("byte 0x{:02x} (assigned)", a.off));
    }
    let nm = |s: &str| -> Option<Target> {
        let o = params::find_name(params, s)?;
        tgt(o, 0, 127, params[o].name.clone())
    };
    let first = |v: Vec<String>| v.iter().find_map(|s| nm(s));
    let n = st.osc as i32;
    let on = st.osc + 1;
    let b = 0x16 + 0x22 * st.osc;
    match id {
        "tune" if st.osc < 3 => match st.tune {
            "octave" => tgt(b + 1, 30 + 5 * n, 34 + 5 * n, format!("Osc {on} octave")),
            "semitone" => tgt(b + 2, 45 + 25 * n, 69 + 25 * n, format!("Osc {on} semitone")),
            _ => nm(&format!("Osc {on} Fine Tune")),
        },
        "portamento" => nm("Portamento Time"),
        "part_level" if st.perf_mode => {
            let p = st.part?;
            Some(Target {
                perf: true,
                off: crate::perf::part_off(p) + crate::perf::rel::LEVEL,
                min: 0,
                max: 127,
                name: format!("Part {} level", p + 1),
            })
        }
        "level" => {
            let src = match st.osc {
                0..=2 => format!("Osc {on}"),
                3 => "Noise".to_string(),
                4 => "Ring Modulator 1 * 3".to_string(),
                _ => "Ring Modulator 2 * 3".to_string(),
            };
            match st.dest {
                "mix" => nm(&format!("{src} Mix Level")),
                "width" if st.osc < 3 => nm(&format!("Osc {on} Pulse Width")),
                "sync" if st.osc < 3 => nm(&format!("Osc {on} Sync")),
                "soften" if st.osc < 3 => nm(&format!("Osc {on} Soften")),
                "soften" if st.osc == 3 => nm("Noise Soften"),
                _ => None,
            }
        }
        "moddepth" => {
            let (dcap, dlow) = match st.dest {
                "mix" => ("Mix", "mix"),
                "pitch" => ("Pitch", "pitch"),
                "width" => ("Width", "width"),
                "sync" => ("Sync", "sync"),
                _ => ("Soften", "hardness"),
            };
            let ms = src_label(st.msrc);
            let mixsoft = st.dest == "mix" || st.dest == "soften";
            first(match st.osc {
                0..=2 => vec![format!("Osc {on} {dcap} {ms}"), format!("Osc {on} Mod knob {ms} {dlow}")],
                3 if mixsoft => vec![format!("Noise {dcap} {ms}"), format!("Noise Mod knob {ms} {dlow}")],
                4 if st.dest == "mix" => vec![format!("1*3 Mod knob {ms} mix")],
                5 if st.dest == "mix" => vec![format!("2*3 Mod knob {ms} mix")],
                _ => vec![],
            })
        }
        "f_freq" => nm("Filter Cutoff Freq"),
        "f_res" => nm("Filter Resonance"),
        "f_over" => nm("Filter Overdrive"),
        "f_mod" => {
            let (pre, wh) = if st.fdest == "freq" {
                ("Filter Freq", "Filter Mod knob Wh frequency")
            } else {
                ("Filter Res", "Filter Mod knob Wh resonance")
            };
            if st.fsrc == "wheel" { nm(wh) } else { nm(&format!("{pre} {}", src_label(st.fsrc))) }
        }
        "l_speed" => nm(&format!("LFO {} Speed", st.lfo + 1)),
        "l_delay" => nm(&format!("LFO {} Delay", st.lfo + 1)),
        "e_att" | "e_dec" | "e_sus" | "e_rel" => {
            let w = match id {
                "e_att" => "Attack",
                "e_dec" => "Decay",
                "e_sus" => "Sustain",
                _ => "Release",
            };
            nm(&format!("Env {} {w}", st.env + 1))
        }
        "effect" => {
            let g = crate::effects::group(st.fx)?;
            let (name, off) = g.params[0];
            match st.fx_part {
                Some(p) if !g.program_only => Some(Target {
                    perf: true,
                    off: crate::effects::part_index(p, off),
                    min: 0,
                    max: 127,
                    name: format!("{} {} (Part {} effects)", g.title, name, p + 1),
                }),
                _ => tgt(off, 0, 127, format!("{} {} (program)", g.title, name)),
            }
        }
        _ => None,
    }
}

const CONTEXT_KNOBS: [&str; 14] = [
    "tune", "portamento", "level", "moddepth", "f_freq", "f_res", "f_over", "f_mod", "l_speed", "l_delay",
    "e_att", "e_dec", "e_sus", "e_rel",
];

fn unbound_text(id: &str) -> &'static str {
    if id == "arp_speed" {
        return "global clock rate, not in the program dump";
    }
    if CONTEXT_KNOBS.contains(&id) || id == "effect" {
        "not in the program dump for the current selection"
    } else if ["mastervol", "arp_speed", "part_level", "fast1", "fast2"].contains(&id) {
        "not in the program dump (global / part data)"
    } else {
        "not mapped yet"
    }
}

fn caption_of(id: &str) -> String {
    KN.iter()
        .find(|k| k.id == id)
        .map(|k| k.cap.to_string())
        .or_else(|| buttons().iter().find(|b| b.id == id).map(|b| b.cap.to_string()))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| id.to_string())
}

pub fn info_text(
    id: &str,
    st: &PanelState,
    assign: &BTreeMap<String, Assign>,
    params: &[Param],
    prog: &[u8],
    perf_body: &[u8],
) -> String {
    if let Some(t) = action_text(id) {
        return t.to_string();
    }
    match target_of(id, st, assign, params) {
        None => format!("{} - {}. Right-click to assign a program byte.", caption_of(id), unbound_text(id)),
        Some(t) => {
            let mut s = format!(
                "{} = {}   ({}byte 0x{:02x}, range {}-{})",
                t.name, if t.perf { perf_body[t.off] } else { prog[t.off] }, if t.perf { "performance " } else { "" }, t.off, t.min, t.max
            );
            if let Some(p) = params.get(t.off) {
                if !p.sources.is_empty() {
                    s.push_str("   |   ");
                    s.push_str(&p.sources.iter().take(3).cloned().collect::<Vec<_>>().join(", "));
                }
            }
            s
        }
    }
}

#[derive(Default, Clone, Copy)]
pub struct Flags {
    pub prog: bool,
    pub perf: bool,
    pub midi: bool,
    pub list: bool,
    /// sequencer playing (seq button lit)
    pub seq: bool,
    /// MIDI input and output both connected (green lamp)
    pub midi_ok: bool,
    pub dump: bool,
    /// mute mode button lit
    pub mute_mode: bool,
    /// effects copy mode (x_copy lit)
    pub copy_mode: bool,
    /// envelope triggering window open (e_trig lit)
    pub trig_open: bool,
    /// solo active
    pub solo: bool,
    /// parts displayed as muted (stored mask, or everything but the soloed part)
    pub muted: u8,
}

pub const ACTIONS: &[&str] = &["write", "midi", "dump", "list", "seq", "prog", "perf", "pageU", "pageD", "x_menu", "x_copy", "a_menu", "o_menu", "f_menu", "l_menu", "e_menu", "e_trig"];

fn action_text(id: &str) -> Option<&'static str> {
    Some(match id {
        "write" => "write: exports this program as a type-2 .syx file (same as Export .syx).",
        "midi" => "midi: opens the MIDI connection window (ports, test, live edit, mirror).",
        "list" => "list: shows or hides the program / performance list on the left.",
        "seq" => "seq: the 8-track step sequencer (Launchpad layout). It keeps playing when you come back here.",
        "dump" => "dump: retrieve from / send to the Nova, and save or load files on this computer.",
        "prog" => "prog: switch to Program mode (and scroll the list to the current program).",
        "o_menu" => "oscillator menu: opens a window with all parameters of the selected oscillator.",
        "f_menu" => "filter menu: opens a window with all filter parameters.",
        "l_menu" => "LFO menu: opens a window with all parameters of the selected LFO.",
        "e_menu" => "envelope menu: opens a window with all parameters of the selected envelope.",
        "e_trig" => "envelope trigger: opens the Envelope triggering window (single / multiple per envelope).",
        "x_menu" => "effects menu: opens the Effects window for the selected effect.",
        "x_copy" => "effects copy (Performance mode): press, then an effect button to copy that effect from the part's program into the part.",
        "a_menu" => "arpeggiator menu: opens the Arpeggiator window.",
        "perf" => "perf: switch to Performance mode (performance list, part editing).",
        "pageU" => "page up: switch to the Panel tab.",
        "pageD" => "page down: switch to the All bytes tab.",
        _ => return None,
    })
}

#[derive(Default)]
pub struct PanelOut {
    /// mode/navigation button pressed (one of ACTIONS)
    pub action: Option<&'static str>,
    /// a Part button (0..5) was pressed in Performance mode
    pub part_selected: Option<usize>,
    /// a Part button was pressed while mute mode is lit
    pub mute_toggle: Option<usize>,
    /// an effect button (delay/reverb/chorus/dist/pan/vocoder) was clicked
    pub fx_clicked: Option<&'static str>,
    /// control id whose readout should be shown
    pub info: Option<String>,
    pub right_click: Option<String>,
    pub step_prog: Option<i32>,
    /// text for the readout when a selector button was pressed
    pub select_msg: Option<String>,
}

fn dimc(c: Color32, dim: bool) -> Color32 {
    if dim { c.gamma_multiply(0.55) } else { c }
}

/// Vertical colour gradient (flat corners).
fn vgrad(p: &egui::Painter, r: Rect, top: Color32, bot: Color32) {
    let mut m = egui::epaint::Mesh::default();
    m.colored_vertex(r.left_top(), top);
    m.colored_vertex(r.right_top(), top);
    m.colored_vertex(r.right_bottom(), bot);
    m.colored_vertex(r.left_bottom(), bot);
    m.add_triangle(0, 1, 2);
    m.add_triangle(0, 2, 3);
    p.add(egui::Shape::mesh(m));
}

/// Horizontal three-stop gradient (edge, centre, edge).
fn hsheen(p: &egui::Painter, r: Rect, edge: Color32, mid: Color32) {
    let mut m = egui::epaint::Mesh::default();
    let (x0, x1, x2) = (r.left(), r.center().x, r.right());
    for (x, c) in [(x0, edge), (x1, mid), (x2, edge)] {
        m.colored_vertex(egui::pos2(x, r.top()), c);
        m.colored_vertex(egui::pos2(x, r.bottom()), c);
    }
    m.add_triangle(0, 1, 2);
    m.add_triangle(1, 2, 3);
    m.add_triangle(2, 3, 4);
    m.add_triangle(3, 4, 5);
    p.add(egui::Shape::mesh(m));
}

/// Metal end cheek with horizontal ribs.
fn cheek(p: &egui::Painter, r: Rect, scale: f32) {
    vgrad(p, r, Color32::from_rgb(0x1a, 0x25, 0x6e), Color32::from_rgb(0x0a, 0x10, 0x3c));
    let step = (9.0 * scale).max(3.0);
    let mut y = r.top() + step;
    while y < r.bottom() - step * 0.5 {
        p.line_segment([egui::pos2(r.left() + 5.0 * scale, y), egui::pos2(r.right() - 5.0 * scale, y)], Stroke::new((2.0 * scale).max(1.0), Color32::from_rgba_unmultiplied(0, 0, 20, 190)));
        p.line_segment([egui::pos2(r.left() + 5.0 * scale, y + 2.2 * scale), egui::pos2(r.right() - 5.0 * scale, y + 2.2 * scale)], Stroke::new((1.0 * scale).max(0.7), Color32::from_rgba_unmultiplied(150, 170, 255, 55)));
        y += step;
    }
}

/// Knob with shadow, chrome rim, knurling, domed black cap and pointer.
fn draw_knob(p: &egui::Painter, c: Pos2, rad: f32, scale: f32, dim: bool, bound: bool, dir: egui::Vec2) {
    p.circle_filled(c, rad * 1.05, Color32::from_black_alpha(90));
    p.circle_filled(c, rad, dimc(Color32::from_rgb(0x1c, 0x1f, 0x29), dim));
    // knurled edge
    let n = 40;
    for i in 0..n {
        let a = i as f32 / n as f32 * std::f32::consts::TAU;
        let d = vec2(a.sin(), -a.cos());
        p.line_segment([c + d * rad * 0.9, c + d * rad * 0.995], Stroke::new((1.6 * scale).max(0.8), dimc(Color32::from_rgb(0x4a, 0x50, 0x60), dim)));
    }
    // chrome rim and dome
    p.circle_stroke(c, rad * 0.99, Stroke::new((2.0 * scale).max(1.0), dimc(Color32::from_rgb(0x8a, 0x92, 0xa6), dim)));
    p.circle_filled(c, rad * 0.78, dimc(Color32::from_rgb(0x0d, 0x0f, 0x15), dim));
    p.circle_filled(c, rad * 0.58, dimc(Color32::from_rgb(0x22, 0x26, 0x31), dim));
    p.circle_filled(c, rad * 0.3, dimc(Color32::from_rgb(0x2c, 0x31, 0x3e), dim));
    let ring = if bound { Color32::from_rgb(0x6b, 0x9b, 0xff) } else { Color32::from_rgb(0x05, 0x06, 0x0a) };
    p.circle_stroke(c, rad * 0.78, Stroke::new(if bound { 3.0 } else { 1.5 } * scale.max(0.6), dimc(ring, dim)));
    p.line_segment([c + dir * rad * 0.3, c + dir * rad * 0.9], Stroke::new(4.5 * scale.max(0.5), dimc(Color32::from_rgb(0xf4, 0xf6, 0xff), dim)));
}

/// Rubber button with a bevel and a red LED when lit.
fn draw_button(p: &egui::Painter, c: Pos2, rad: f32, scale: f32, on: bool, dim: bool) {
    p.circle_filled(c, rad * 1.1, Color32::from_black_alpha(90));
    p.circle_filled(c, rad * 1.06, dimc(Color32::from_rgb(0x4c, 0x55, 0x70), dim));
    if on {
        p.circle_filled(c, rad * 1.9, Color32::from_rgba_unmultiplied(255, 70, 40, 34));
        p.circle_filled(c, rad * 1.45, Color32::from_rgba_unmultiplied(255, 80, 50, 60));
        p.circle_filled(c, rad, Color32::from_rgb(0xe0, 0x2e, 0x22));
        p.circle_filled(c, rad * 0.5, Color32::from_rgb(0xff, 0xa8, 0x88));
    } else {
        p.circle_filled(c, rad, dimc(Color32::from_rgb(0x0c, 0x0e, 0x15), dim));
        p.circle_filled(c, rad * 0.72, dimc(Color32::from_rgb(0x1c, 0x20, 0x2b), dim));
        // unlit LED lens
        p.circle_filled(c, rad * 0.18, dimc(Color32::from_rgb(0x4a, 0x12, 0x12), dim));
    }
}

pub fn draw(
    ui: &mut Ui,
    st: &mut PanelState,
    prog: &mut [u8],
    assign: &BTreeMap<String, Assign>,
    params: &[Param],
    perf_body: &mut [u8],
    lcd: (String, String),
    flags: Flags,
) -> PanelOut {
    let mut out = PanelOut::default();
    let avail = ui.available_size();
    // stretch the layout to fill the area; controls and text use the smaller factor so they stay round
    let sx = (avail.x / PW).max(0.2);
    let sy = (avail.y / PH).max(0.2);
    let scale = sx.min(sy);
    // take the whole area and centre the panel in it
    let (full, _) = ui.allocate_exact_size(avail, Sense::hover());
    let rect = Rect::from_center_size(full.center(), vec2(PW * sx, PH * sy));
    let p = ui.painter_at(rect);
    let pt = |x: f32, y: f32| -> Pos2 { rect.min + vec2(x * sx, y * sy) };
    let rr = |x: f32, y: f32, w: f32, h: f32| Rect::from_min_size(pt(x, y), vec2(w * sx, h * sy));

    // background: dark bevelled frame, royal-blue metallic plate, ribbed end cheeks, black head strip
    let corner = (14.0 * scale) as u8;
    p.rect_filled(rect, 14.0 * scale, Color32::from_rgb(0x07, 0x0a, 0x24));
    let plate = rect.shrink(8.0 * scale);
    let head_h = PH * 0.295 * sy;
    let body = Rect::from_min_max(egui::pos2(plate.left(), rect.top() + head_h), plate.max);
    vgrad(&p, body, Color32::from_rgb(0x2b, 0x4c, 0xc8), Color32::from_rgb(0x16, 0x28, 0x8c));
    hsheen(&p, body, Color32::from_rgba_unmultiplied(255, 255, 255, 0), Color32::from_rgba_unmultiplied(170, 200, 255, 26));
    let head = Rect::from_min_max(plate.min, egui::pos2(plate.right(), rect.top() + head_h));
    vgrad(&p, head, Color32::from_rgb(0x15, 0x18, 0x26), Color32::from_rgb(0x05, 0x06, 0x0c));
    p.line_segment([body.left_top(), body.right_top()], Stroke::new(2.0 * scale, Color32::from_rgb(0x9a, 0xa8, 0xd8)));
    // end cheeks
    let cw = 56.0 * scale;
    cheek(&p, Rect::from_min_max(plate.min, egui::pos2(plate.left() + cw, plate.bottom())), scale);
    cheek(&p, Rect::from_min_max(egui::pos2(plate.right() - cw, plate.top()), plate.max), scale);
    // frame: rounded outer edge and a soft inner highlight
    p.rect_stroke(rect, 14.0 * scale, Stroke::new(8.0 * scale, Color32::from_rgb(0x07, 0x0a, 0x24)), egui::StrokeKind::Inside);
    p.rect_stroke(plate, 4.0 * scale, Stroke::new(1.2 * scale, Color32::from_rgba_unmultiplied(160, 180, 255, 70)), egui::StrokeKind::Inside);
    let _ = corner;
    p.text(pt(PW * 0.055, PH * 0.065), Align2::LEFT_TOP, "novation NOVA", FontId::proportional(48.0 * scale), Color32::WHITE);
    p.text(
        pt(PW * 0.055, PH * 0.065 + 58.0),
        Align2::LEFT_TOP,
        "P O L Y P H O N I C   S Y N T H E S I S E R",
        FontId::proportional(14.0 * scale),
        Color32::from_rgb(0x9f, 0xb2, 0xd8),
    );
    // LCD: bezel, glass, glowing text
    let lcd_rect = rr(PW * 0.373, PH * 0.106, PW * 0.25, PH * 0.136);
    let bez = lcd_rect.expand(7.0 * scale);
    p.rect_filled(bez.translate(vec2(0.0, 2.0 * scale)), 6.0 * scale, Color32::from_black_alpha(120));
    p.rect_filled(bez, 6.0 * scale, Color32::from_rgb(0x2b, 0x2f, 0x3d));
    p.rect_stroke(bez, 6.0 * scale, Stroke::new(1.5 * scale, Color32::from_rgb(0x6a, 0x72, 0x8c)), egui::StrokeKind::Inside);
    vgrad(&p, lcd_rect, Color32::from_rgb(0x08, 0x1c, 0x42), Color32::from_rgb(0x03, 0x0b, 0x22));
    p.rect_stroke(lcd_rect, 3.0 * scale, Stroke::new(1.5 * scale, Color32::from_black_alpha(200)), egui::StrokeKind::Inside);
    // 2 x 16 character display: fit the cells to the glass, draw faint unlit cells, then the text
    let probe = p.layout_no_wrap("M".into(), FontId::monospace(100.0), Color32::WHITE);
    let (cw100, ch100) = (probe.size().x, probe.size().y);
    let fs = (lcd_rect.width() * 0.9 / 16.0 / (cw100 / 100.0)).min(lcd_rect.height() * 0.42 / (ch100 / 100.0));
    let lf = FontId::monospace(fs);
    let (cw, chh) = (cw100 * fs / 100.0, ch100 * fs / 100.0);
    let x0 = lcd_rect.center().x - cw * 8.0;
    let gap = lcd_rect.height() * 0.06;
    let y0 = lcd_rect.center().y - chh - gap / 2.0;
    let lc = Color32::from_rgb(0x7c, 0xd0, 0xff);
    for (i, txt) in [lcd.0, lcd.1].into_iter().enumerate() {
        let y = y0 + i as f32 * (chh + gap);
        for col in 0..16 {
            let cell = Rect::from_min_size(egui::pos2(x0 + col as f32 * cw + cw * 0.06, y + chh * 0.12), vec2(cw * 0.88, chh * 0.78));
            p.rect_filled(cell, 1.0, Color32::from_rgba_unmultiplied(70, 130, 255, 16));
        }
        let txt: String = format!("{:<16}", txt).chars().take(16).collect();
        let pos = egui::pos2(x0, y);
        p.text(pos + vec2(0.0, 1.0 * scale), Align2::LEFT_TOP, txt.clone(), lf.clone(), Color32::from_rgba_unmultiplied(80, 170, 255, 60));
        p.text(pos, Align2::LEFT_TOP, txt, lf.clone(), lc);
    }
    vgrad(&p, Rect::from_min_size(lcd_rect.min, vec2(lcd_rect.width(), lcd_rect.height() * 0.42)), Color32::from_rgba_unmultiplied(255, 255, 255, 16), Color32::from_rgba_unmultiplied(255, 255, 255, 0));

    // sections
    for (t, x, y, bx, by, w, h) in SECT {
        // outline starts below the head strip edge and runs past the lowest captions; the title is a
        // printed legend that interrupts the top line, so nothing is drawn across the text
        let top = by + 10.0;
        let sr = rr(bx, top, w, h - 2.0);
        p.rect_filled(sr, 6.0 * scale, Color32::from_rgba_unmultiplied(6, 14, 70, 70));
        p.rect_stroke(sr, 6.0 * scale, Stroke::new(1.4 * scale.max(0.6), Color32::from_rgba_unmultiplied(225, 235, 255, 120)), egui::StrokeKind::Inside);
        let _ = (x, y);
        let tf = FontId::proportional(21.0 * scale * TEXT_BOOST);
        let g = p.layout_no_wrap(t.to_string(), tf.clone(), Color32::from_rgb(0xe8, 0xee, 0xfc));
        let tp = pt(bx + 14.0, top);
        let fy = ((tp.y - body.top()) / body.height()).clamp(0.0, 1.0);
        let lerp = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * fy) as u8;
        let plate_col = Color32::from_rgb(lerp(0x2b, 0x16), lerp(0x4c, 0x28), lerp(0xc8, 0x8c));
        let back = Rect::from_min_size(egui::pos2(tp.x - 5.0 * scale, tp.y - g.size().y / 2.0), vec2(g.size().x + 10.0 * scale, g.size().y));
        p.rect_filled(back, 3.0 * scale, plate_col);
        p.galley(egui::pos2(tp.x, tp.y - g.size().y / 2.0), g, Color32::from_rgb(0xe8, 0xee, 0xfc));
    }

    let capfont = FontId::proportional((PW * scale * 0.0085 * TEXT_BOOST).max(9.0));
    let mut caps: Vec<(Pos2, Align2, String, Color32)> = Vec::new();
    let capcol = Color32::from_rgb(0xe3, 0xea, 0xfc);
    // column geometry shared by knobs and buttons: returns (graphic centre, caption top, hit rect)
    let geom = |x: f32, y: f32, d: f32, cap: &str, ui: &Ui| {
        // (x, y) is the centre of the knob/button graphic; the caption hangs 3 units below it
        let center = pt(x, y);
        let cap_top = center + vec2(0.0, (d / 2.0 + 3.0) * scale);
        let cw = if cap.is_empty() {
            0.0
        } else {
            ui.painter().layout_no_wrap(cap.to_string(), capfont.clone(), capcol).size().x
        };
        let w = (d * scale).max(cw);
        let hit = Rect::from_min_max(center - vec2(w / 2.0, d / 2.0 * scale), center + vec2(w / 2.0, (d / 2.0 + 3.0 + 20.0) * scale));
        (center, cap_top, hit)
    };

    // knobs
    for kn in KN.iter() {
        let d = 84.0;
        let (c, cap_top, hit) = geom(kn.x, kn.y, d, kn.cap, ui);
        let r = ui.interact(hit, ui.id().with(("kn", kn.id)), Sense::click_and_drag());
        if kn.id == "fast2" {
            // lower Fast Data knob: an endless encoder that selects programs, like Prog up/down
            const NOTCH: f32 = 14.0;
            if r.dragged() {
                st.fast_acc -= r.drag_delta().y;
            }
            if r.hovered() {
                st.fast_acc += ui.input(|i| i.smooth_scroll_delta.y);
            }
            let mut steps = 0i32;
            while st.fast_acc >= NOTCH {
                st.fast_acc -= NOTCH;
                steps += 1;
            }
            while st.fast_acc <= -NOTCH {
                st.fast_acc += NOTCH;
                steps -= 1;
            }
            if steps != 0 {
                out.step_prog = Some(steps);
                st.fast_turn += steps as f32 * 0.35;
            }
            r.clone().on_hover_text("Turn (drag up/down or scroll) to select programs; with a Part selected in Perf mode, that Part's program.");
            let a = st.fast_turn;
            draw_knob(&p, c, d / 2.0 * scale, scale, false, false, vec2(a.sin(), -a.cos()));
            continue;
        }
        let t = target_of(kn.id, st, assign, params);
        if let Some(t) = &t {
            let buf: &mut [u8] = if t.perf { &mut *perf_body } else { &mut *prog };
            if r.drag_started() {
                st.drag = Some((kn.id.to_string(), 0.0, buf[t.off] as i32));
            }
            if r.dragged() {
                let dy = -r.drag_delta().y;
                if let Some((did, acc, start)) = st.drag.as_mut() {
                    if did == kn.id {
                        *acc += dy;
                        let range = (t.max - t.min).max(1) as f32;
                        let dv = (*acc / 180.0 * range).round() as i32;
                        buf[t.off] = (*start + dv).clamp(t.min, t.max).clamp(0, 127) as u8;
                    }
                }
            }
            if r.hovered() {
                st.scroll_acc += ui.input(|i| i.smooth_scroll_delta.y);
                let step = 12.0;
                while st.scroll_acc >= step {
                    st.scroll_acc -= step;
                    buf[t.off] = (buf[t.off] as i32 + 1).clamp(t.min, t.max).clamp(0, 127) as u8;
                }
                while st.scroll_acc <= -step {
                    st.scroll_acc += step;
                    buf[t.off] = (buf[t.off] as i32 - 1).clamp(t.min, t.max).clamp(0, 127) as u8;
                }
            }
        }
        if r.drag_stopped() {
            st.drag = None;
        }
        if r.clicked() || r.dragged() || r.drag_started() {
            out.info = Some(kn.id.to_string());
        }
        if r.secondary_clicked() || r.long_touched() {
            out.right_click = Some(kn.id.to_string());
        }
        if !r.hovered() && st.drag.is_none() {
            // keep accumulator from carrying across controls
        }
        if t.is_none() {
            r.clone().on_hover_text(unbound_text(kn.id));
        }
        let dim = t.is_none();
        let rad = d / 2.0 * scale;
        let bound = assign.contains_key(kn.id);
        let f = t
            .as_ref()
            .map(|t| (((if t.perf { perf_body[t.off] } else { prog[t.off] }) as i32 - t.min) as f32 / (t.max - t.min).max(1) as f32).clamp(0.0, 1.0))
            .unwrap_or(0.5);
        let a = (-135.0 + 270.0 * f).to_radians();
        draw_knob(&p, c, rad, scale, dim, bound, vec2(a.sin(), -a.cos()));
        if !kn.cap.is_empty() {
            let col = if r.hovered() { Color32::WHITE } else { capcol };
            caps.push((cap_top, Align2::CENTER_TOP, kn.cap.to_string(), col));
        }
    }

    // buttons
    for bt in buttons().iter() {
        let d = 36.0;
        let side = ["bankU", "bankD", "progU", "progD", "pageU", "pageD"].contains(&bt.id);
        let (mut c, mut cap_top, mut hit) = geom(bt.x, bt.y, d, bt.cap, ui);
        if side {
            c = pt(bt.x, bt.y);
            cap_top = c + vec2(d / 2.0 * scale + 6.0, 0.0);
            let cw = ui.painter().layout_no_wrap(bt.cap.to_string(), capfont.clone(), capcol).size().x;
            hit = Rect::from_min_max(c - vec2(d / 2.0 * scale, d / 2.0 * scale), c + vec2(d / 2.0 * scale + 6.0 + cw, d / 2.0 * scale));
        }
        let r = ui.interact(hit, ui.id().with(("bt", bt.id)), Sense::click());
        match bt.kind {
            Kind::Plain if bt.id == "x_copy" && !flags.perf => {
                r.clone().on_hover_text("Performance mode only");
            }
            Kind::Part(_) | Kind::PPage(_) if !flags.perf => {
                r.clone().on_hover_text("Performance mode only (press perf)");
            }
            _ => {}
        }
        if r.secondary_clicked() || r.long_touched() {
            out.right_click = Some(bt.id.to_string());
        }
        if r.clicked() {
            let id = bt.id;
            match bt.kind {
                Kind::Osc(n) => st.osc = n,
                Kind::Part(n) => {
                    if flags.perf {
                        if flags.mute_mode {
                            out.mute_toggle = Some(n);
                        } else {
                            st.part = Some(n);
                            out.part_selected = Some(n);
                        }
                    }
                }
                Kind::PPage(x) => {
                    if flags.perf {
                        st.ppage = x;
                    }
                }
                Kind::Slope(n) => prog[FILTER_SLOPE] = n,
                Kind::FType(n) => prog[FILTER_TYPE] = ftype_click(prog[FILTER_TYPE], n),
                Kind::LWave(w) => prog[LFO_WAVE[st.lfo.min(1)]] = lfo_wave_value(st.lfo, w),
                Kind::Toggle(bit) => out.action = Some(if bit == 1 { "mute" } else { "solo" }),
                Kind::Env(n) => st.env = n,
                Kind::Lfo(n) => st.lfo = n,
                Kind::FDest(x) => st.fdest = x,
                Kind::FSrc(x) => st.fsrc = x,
                Kind::MSrc(x) => st.msrc = x,
                Kind::Tune(s) => st.tune = s,
                Kind::Dest(s) => st.dest = s,
                Kind::Fx(s) => {
                    st.fx = s;
                    out.fx_clicked = Some(s);
                }
                Kind::Arp(w) => {
                    let (i, f): (usize, fn(u8) -> u8) = match w {
                        0 => (crate::arp::ON, crate::arp::on_toggle),
                        1 => (crate::arp::LATCH, crate::arp::latch_toggle),
                        _ => (crate::arp::MUTE, crate::arp::mute_toggle),
                    };
                    prog[i] = f(prog[i]);
                }
                Kind::Wave(_) if st.osc >= 3 => {}
                Kind::Wave(w) => {
                    let lo = match w {
                        "square" => 0,
                        "saw" => 1,
                        _ => 2,
                    };
                    prog[st.osc_base()] = (10 * st.osc + lo) as u8;
                }
                Kind::Plain => {
                    if let Some(a) = ACTIONS.iter().find(|a| **a == id).filter(|a| **a != "pageU" && **a != "pageD") {
                        out.action = Some(a);
                    } else if id == "progU" {
                        out.step_prog = Some(1);
                    } else if id == "progD" {
                        out.step_prog = Some(-1);
                    } else if id == "bankU" {
                        out.step_prog = Some(128);
                    } else if id == "bankD" {
                        out.step_prog = Some(-128);
                    } else if let Some(a) = assign.get(id) {
                        let cur = prog[a.off] as i32;
                        prog[a.off] = (if cur == a.max { a.min } else { a.max }).clamp(0, 127) as u8;
                    }
                }
            }
            if id == "pageU" || id == "pageD" {
                out.action = ACTIONS.iter().find(|a| **a == id).copied();
            }
            if matches!(bt.kind, Kind::Slope(_) | Kind::FType(_) | Kind::LWave(_) | Kind::Arp(_)) {
                out.select_msg = Some(match bt.kind {
                    Kind::Slope(_) => format!(
                        "filter slope = {}dB (byte 0xd0 = {})",
                        [12, 18, 24][(prog[FILTER_SLOPE] as usize).min(2)],
                        prog[FILTER_SLOPE]
                    ),
                    Kind::Arp(w) => format!(
                        "arpeggiator {} = {}",
                        ["on/off", "latch", "mute"][w as usize],
                        match w {
                            0 => if crate::arp::on_lit(prog[crate::arp::ON]) { "on" } else { "off" },
                            1 => if crate::arp::latch_lit(prog[crate::arp::LATCH]) { "latched" } else { "off" },
                            _ => if crate::arp::mute_lit(prog[crate::arp::MUTE]) { "muted" } else { "not muted" },
                        }
                    ),
                    Kind::FType(_) => format!("filter type = {} (byte 0xcf = {})", ftype_name(prog[FILTER_TYPE]), prog[FILTER_TYPE]),
                    _ => format!(
                        "LFO {} waveform = {} (byte 0x{:02x} = {})",
                        st.lfo + 1,
                        ["square", "saw", "triangle", "s/h"][(prog[LFO_WAVE[st.lfo.min(1)]].wrapping_sub(LFO_WAVE_BASE[st.lfo.min(1)]) as usize).min(3)],
                        LFO_WAVE[st.lfo.min(1)],
                        prog[LFO_WAVE[st.lfo.min(1)]]
                    ),
                });
            } else if matches!(bt.kind, Kind::Part(_) | Kind::PPage(_) | Kind::Toggle(_)) {
                out.select_msg = Some(if !flags.perf {
                    "Part Edit works in Performance mode - press perf first.".to_string()
                } else if let Kind::Toggle(bit) = bt.kind {
                    if bit == 1 {
                        "mute: mute mode - Part buttons now toggle that part's mute (flashing = muted).".to_string()
                    } else {
                        "solo: soloes the selected part (others flash as muted); press again to cancel. Not stored.".to_string()
                    }
                } else if flags.mute_mode && matches!(bt.kind, Kind::Part(_)) {
                    "Part mute toggled (mute mode is lit; the selected part is unchanged).".to_string()
                } else {
                    format!(
                        "Part {} / page {}: knobs edit that part's program; see the Part parameters strip.",
                        st.part.map(|p| (p + 1).to_string()).unwrap_or("-".into()),
                        st.ppage
                    )
                });
            } else if !matches!(bt.kind, Kind::Plain | Kind::Wave(_)) {
                out.select_msg = Some(format!(
                    "Selected {} - knobs now edit: source {}, tune {}, destination {}, mod source {}, envelope {}, LFO {}, filter mod {} from {}, effect {}.",
                    bt.cap,
                    ["osc 1", "osc 2", "osc 3", "noise", "1*3", "2*3"][st.osc],
                    st.tune,
                    st.dest,
                    src_label(st.msrc),
                    st.env + 1,
                    st.lfo + 1,
                    st.fdest,
                    src_label(st.fsrc),
                    st.fx
                ));
            } else {
                out.info = Some(id.to_string());
            }
        }
        // lit / live state
        let (mut on, mut live) = (false, false);
        match bt.kind {
            Kind::Osc(n) => (on, live) = (st.osc == n, true),
            Kind::Tune(s) => (on, live) = (st.tune == s, true),
            Kind::Dest(s) => (on, live) = (st.dest == s, true),
            Kind::Fx(s) => (on, live) = (st.fx == s, true),
            Kind::Part(n) => {
                on = flags.perf && st.part == Some(n);
                live = flags.perf;
                if flags.perf && flags.muted & (1 << n) != 0 {
                    ui.ctx().request_repaint_after(std::time::Duration::from_millis(400));
                    on = (ui.input(|i| i.time) * 2.5) as i64 % 2 == 0;
                }
            }
            Kind::PPage(x) => (on, live) = (flags.perf && st.ppage == x, flags.perf),
            Kind::Arp(w) => {
                live = true;
                on = match w {
                    0 => crate::arp::on_lit(prog[crate::arp::ON]),
                    1 => crate::arp::latch_lit(prog[crate::arp::LATCH]),
                    _ => crate::arp::mute_lit(prog[crate::arp::MUTE]),
                };
            }
            Kind::Slope(n) => (on, live) = (slope_lit(prog[FILTER_SLOPE], n), true),
            Kind::FType(n) => (on, live) = (ftype_lit(prog[FILTER_TYPE], n), true),
            Kind::LWave(w) => (on, live) = (lfo_wave_lit(prog[LFO_WAVE[st.lfo.min(1)]], st.lfo, w), true),
            Kind::Toggle(bit) => (on, live) = (if bit == 1 { flags.mute_mode } else { flags.solo }, flags.perf),
            Kind::Env(n) => (on, live) = (st.env == n, true),
            Kind::Lfo(n) => (on, live) = (st.lfo == n, true),
            Kind::FDest(x) => (on, live) = (st.fdest == x, true),
            Kind::FSrc(x) => (on, live) = (st.fsrc == x, true),
            Kind::MSrc(x) => (on, live) = (st.msrc == x, true),
            Kind::Wave(w) => {
                let v = if st.osc < 3 { prog[st.osc_base()] as i32 - 10 * st.osc as i32 } else { -99 };
                on = if w == "special" { v >= 2 } else { v == if w == "square" { 0 } else { 1 } };
                live = st.osc < 3;
            }
            Kind::Plain => {
                if ["progU", "progD", "bankU", "bankD", "write", "pageU", "pageD", "x_menu", "a_menu", "o_menu", "f_menu", "l_menu", "e_menu"].contains(&bt.id) {
                    live = true;
                } else if bt.id == "e_trig" {
                    live = true;
                    on = flags.trig_open;
                } else if bt.id == "x_copy" {
                    live = flags.perf;
                    on = flags.copy_mode;
                } else if ["prog", "perf", "midi", "dump", "list", "seq"].contains(&bt.id) {
                    live = true;
                    on = match bt.id {
                        "prog" => flags.prog,
                        "midi" => flags.midi,
                        "dump" => flags.dump,
                        "list" => flags.list,
                        "seq" => flags.seq,
                        _ => flags.perf,
                    };
                } else if let Some(a) = assign.get(bt.id) {
                    on = prog[a.off] as i32 == a.max;
                    live = true;
                }
            }
        }
        let dim = !live;
        let rad = d / 2.0 * scale;
        draw_button(&p, c, rad, scale, on, dim);
        let col = if r.hovered() { Color32::WHITE } else { capcol };
        caps.push((cap_top, if side { Align2::LEFT_CENTER } else { Align2::CENTER_TOP }, bt.cap.to_string(), col));
    }
    // MIDI link lamp beside the midi button: green = input and output connected, red = not
    {
        let c = pt(1624.0, 56.0);
        let rad = 7.0 * scale;
        let (core, glow) = if flags.midi_ok {
            (Color32::from_rgb(0x39, 0xe0, 0x6a), Color32::from_rgba_unmultiplied(60, 255, 110, 60))
        } else {
            (Color32::from_rgb(0xe0, 0x2e, 0x22), Color32::from_rgba_unmultiplied(255, 70, 40, 50))
        };
        p.circle_filled(c, rad * 2.2, glow);
        p.circle_filled(c, rad * 1.25, Color32::from_rgb(0x20, 0x26, 0x38));
        p.circle_filled(c, rad, core);
        p.circle_filled(c, rad * 0.4, Color32::from_rgba_unmultiplied(255, 255, 255, 120));
        p.text(
            c + vec2(rad * 2.0, 0.0),
            Align2::LEFT_CENTER,
            if flags.midi_ok { "connected" } else { "disconnected" },
            capfont.clone(),
            Color32::WHITE,
        );
        ui.interact(Rect::from_center_size(c, vec2(rad * 3.0, rad * 3.0)), ui.id().with("midi_lamp"), Sense::hover())
            .on_hover_text(if flags.midi_ok { "MIDI connected (input and output)" } else { "MIDI not connected" });
    }
    for (pos, al, txt, col) in caps {
        // a trailing up/down arrow is drawn as a triangle (not every font has the glyphs)
        let arrow = match txt.chars().last() {
            Some('\u{25b2}') => Some(true),
            Some('\u{25bc}') => Some(false),
            _ => None,
        };
        let txt = if arrow.is_some() { txt.trim_end_matches(['\u{25b2}', '\u{25bc}']).trim_end().to_string() } else { txt };
        let g = p.layout_no_wrap(txt, capfont.clone(), col);
        let h = g.size().y;
        let extra = if arrow.is_some() { h * 0.85 } else { 0.0 };
        let r = al.anchor_size(pos, g.size() + vec2(extra, 0.0));
        p.rect_filled(r.expand2(vec2(3.0, 1.0)), 3.0, Color32::from_rgba_unmultiplied(10, 20, 50, 150));
        if let Some(up) = arrow {
            let cx = r.right() - h * 0.32;
            let cy = r.center().y;
            let (w2, h2) = (h * 0.26, h * 0.22);
            let pts = if up {
                vec![egui::pos2(cx - w2, cy + h2), egui::pos2(cx + w2, cy + h2), egui::pos2(cx, cy - h2)]
            } else {
                vec![egui::pos2(cx - w2, cy - h2), egui::pos2(cx + w2, cy - h2), egui::pos2(cx, cy + h2)]
            };
            p.add(egui::Shape::convex_polygon(pts, col, Stroke::NONE));
        }
        p.galley(r.min, g, col);
    }
    let _ = N;
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sysex;

    #[test]
    fn button_mappings() {
        // type 4 lights only f_bd, type 9 only f_sp
        let btns = [3u8, 4, 5, FTYPE_SPECIAL];
        let lit = |byte: u8| btns.iter().filter(|&&b| ftype_lit(byte, b)).copied().collect::<Vec<_>>();
        assert_eq!(lit(4), vec![4]);
        assert_eq!(lit(9), vec![FTYPE_SPECIAL]);
        assert_eq!(ftype_click(3, FTYPE_SPECIAL), 6);
        assert_eq!(ftype_click(9, FTYPE_SPECIAL), 9);
        assert_eq!(ftype_click(9, 5), 5);
        assert_eq!(ftype_name(9), "special: Notch");
        assert!(slope_lit(2, 2) && !slope_lit(2, 1));
        // LFO 2 byte 22 lights triangle only when LFO 2 selected
        assert!(lfo_wave_lit(22, 1, 2));
        assert!(!lfo_wave_lit(22, 0, 2));
        assert_eq!(lfo_wave_value(0, 3), 19);
        assert_eq!(lfo_wave_value(1, 0), 20);
    }

    #[test]
    fn factory_slope_and_type() {
        let (progs, _) = sysex::parse_all(include_bytes!("../assets/total.mid"));
        let get = |b: u8, i: u8| progs.iter().find(|p| p.bank == Some(b) && p.idx == i).unwrap();
        let a53 = get(5, 53);
        assert_eq!(a53.name(), "Sonar (C3)  M-Wh");
        assert_eq!((a53.body[FILTER_SLOPE], a53.body[FILTER_TYPE]), (2, 4));
        let a7 = get(5, 7);
        assert_eq!(a7.name(), "FM EP");
        assert_eq!((a7.body[FILTER_SLOPE], a7.body[FILTER_TYPE]), (0, 3));
    }
}
