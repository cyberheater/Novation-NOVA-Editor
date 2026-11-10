//! 8-track step sequencer, ported from the Launchpad Pro MK3 CoreFW sequencer (lpp-emu/corefw/src/app/sequencer).
//!
//! The engine runs on its own thread, so playback carries on while the editor page is shown. Notes go to the
//! MIDI output the editor is connected to (the Nova), one MIDI channel per track (track n = channel n by default,
//! matching the Multi Ch 1-6 performances).
//!
//! The page is the Launchpad layout: an 8 x 8 grid (top four rows = 32 steps, bottom four rows = 32 notes), the
//! round side buttons, the track row and the bottom row. A connected Launchpad Pro MK3 is put into Programmer mode
//! and drives the same page with real press-and-hold; on screen, the hold buttons (Shift, Clear, Duplicate,
//! Fixed Length, Patterns, Projects, Mute, Solo) latch, and right-click / long-press holds a step for editing.
use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, Ui, vec2};
use midir::{MidiInput, MidiInputConnection, MidiOutput, MidiOutputConnection};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, channel};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

pub const TRACKS: usize = 8;
/// Patterns per project. A pattern holds all 8 tracks (each track keeps its own
/// copy at the same index, and every track is always on the same pattern).
pub const PATTERNS: usize = 16;
pub const STEPS: usize = 64;
pub const MAX_NOTES: usize = 8;
/// Projects (matches the Launchpad firmware).
pub const SLOTS: usize = 32;
/// Testing aid: label each step pad PRxxPNxxSPxx (project, pattern, step).
const DEBUG_STEP_IDS: bool = false;
/// Half clock pulses (48 per quarter note) per step for each step rate.
pub const RATE_HP: [u32; 8] = [192, 48, 32, 24, 16, 12, 8, 6];
pub const RATE_NAMES: [&str; 8] = ["1/1", "1/4", "1/4T", "1/8", "1/8T", "1/16", "1/16T", "1/32"];
pub const DEFAULT_RATE: u8 = 5;
/// Velocity and probability bar values, top row first (as on the Launchpad).
const VEL_BAR: [u8; 8] = [127, 111, 95, 79, 63, 47, 31, 16];
const PROB_BAR: [u8; 8] = [100, 88, 75, 63, 50, 38, 25, 12];
const NOTE_NAMES: [&str; 12] = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];

pub type Out = Arc<Mutex<MidiOutputConnection>>;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

pub fn note_name(n: u8) -> String {
    format!("{}{}", NOTE_NAMES[n as usize % 12], n as i32 / 12 - 2)
}

// ---------------------------------------------------------------------------------------------------------------
// Song data (saved as JSON)

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Step {
    pub on: bool,
    /// 1-8 notes (a chord)
    pub notes: Vec<u8>,
    pub vel: u8,
    /// gate length in eighths of a step, 1-8
    pub gate: u8,
    /// chance the step plays, 1-100 %
    pub prob: u8,
}
impl Default for Step {
    fn default() -> Self {
        Step { on: false, notes: vec![60], vel: 100, gate: 4, prob: 100 }
    }
}
impl Step {
    fn single(note: u8) -> Step {
        Step { on: true, notes: vec![note], ..Step::default() }
    }
    fn toggle_note(&mut self, note: u8) {
        if let Some(i) = self.notes.iter().position(|&n| n == note) {
            if self.notes.len() > 1 {
                self.notes.remove(i);
            }
        } else if self.notes.len() < MAX_NOTES {
            self.notes.push(note);
        } else {
            self.notes[MAX_NOTES - 1] = note;
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Pattern {
    pub steps: Vec<Step>,
    pub length: u8,
}
impl Default for Pattern {
    fn default() -> Self {
        Pattern { steps: vec![Step::default(); STEPS], length: 16 }
    }
}
impl Pattern {
    pub fn has_notes(&self) -> bool {
        self.steps.iter().any(|s| s.on)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Track {
    pub patterns: Vec<Pattern>,
    /// pattern playing / being edited
    pub pattern: u8,
    pub chain_start: u8,
    pub chain_end: u8,
    /// MIDI channel 0-15
    pub channel: u8,
    pub muted: bool,
    pub solo: bool,
    /// note given to newly lit steps
    pub note: u8,
    /// lowest note of the note area
    pub base: u8,
    pub transpose: i8,
    /// index into RATE_HP
    pub rate: u8,
    /// pattern range to switch to at the end of the current pattern
    #[serde(skip)]
    pub queued: Option<(u8, u8)>,
}
impl Default for Track {
    fn default() -> Self {
        Track {
            patterns: vec![Pattern::default(); PATTERNS],
            pattern: 0,
            chain_start: 0,
            chain_end: 0,
            channel: 0,
            muted: false,
            solo: false,
            note: 60,
            base: 48,
            transpose: 0,
            rate: DEFAULT_RATE,
            queued: None,
        }
    }
}
impl Track {
    pub fn pat(&self) -> &Pattern {
        &self.patterns[self.pattern as usize % PATTERNS]
    }
    pub fn pat_mut(&mut self) -> &mut Pattern {
        let p = self.pattern as usize % PATTERNS;
        &mut self.patterns[p]
    }
    /// At the end of a pattern: take a queued pattern / chain, or move along the current chain.
    fn next_pattern(&mut self) {
        if let Some((a, b)) = self.queued.take() {
            self.chain_start = a;
            self.chain_end = b;
            self.pattern = a;
        } else if self.chain_end > self.chain_start {
            self.pattern = if self.pattern < self.chain_start || self.pattern >= self.chain_end { self.chain_start } else { self.pattern + 1 };
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Song {
    pub tracks: Vec<Track>,
    pub bpm: u16,
    /// 50 = straight ... 75
    pub swing: u8,
    /// send MIDI clock / start / stop to the output
    pub send_clock: bool,
    /// which of the 64 projects this song is
    pub project: u8,
}
impl Default for Song {
    fn default() -> Self {
        let tracks = (0..TRACKS).map(|i| Track { channel: i as u8, ..Track::default() }).collect();
        Song { tracks, bpm: 120, swing: 50, send_clock: false, project: 0 }
    }
}
impl Song {
    /// Repairs sizes and ranges after loading a file.
    fn normalize(&mut self) {
        self.tracks.resize_with(TRACKS, Track::default);
        for t in &mut self.tracks {
            t.patterns.resize_with(PATTERNS, Pattern::default);
            for p in &mut t.patterns {
                p.steps.resize_with(STEPS, Step::default);
                p.length = p.length.clamp(1, STEPS as u8);
                for s in &mut p.steps {
                    s.notes.retain(|&n| n < 128);
                    s.notes.truncate(MAX_NOTES);
                    if s.notes.is_empty() {
                        s.notes.push(60);
                    }
                    s.vel = s.vel.clamp(1, 127);
                    s.gate = s.gate.clamp(1, 8);
                    s.prob = s.prob.clamp(1, 100);
                }
            }
            t.pattern %= PATTERNS as u8;
            t.chain_start %= PATTERNS as u8;
            t.chain_end = t.chain_end.clamp(t.chain_start, PATTERNS as u8 - 1);
            t.channel &= 15;
            t.rate = t.rate.min(RATE_HP.len() as u8 - 1);
            t.base = t.base.min(96);
            t.transpose = t.transpose.clamp(-24, 24);
        }
        // patterns are song-wide: every track follows track 1's pattern / chain
        let (pat, a, b) = (self.tracks[0].pattern, self.tracks[0].chain_start, self.tracks[0].chain_end);
        for t in &mut self.tracks {
            (t.pattern, t.chain_start, t.chain_end, t.queued) = (pat, a, b, None);
        }
        self.project = self.project.min(SLOTS as u8 - 1);
        self.bpm = self.bpm.clamp(30, 300);
        self.swing = self.swing.clamp(50, 75);
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Engine

struct Ev {
    due: Instant,
    msg: Vec<u8>,
}

pub struct Engine {
    pub song: Song,
    pub playing: bool,
    /// next step to play, per track
    pos: [u8; TRACKS],
    /// step sounding now (for the playhead), per track
    pub playing_step: [Option<u8>; TRACKS],
    hp_count: u64,
    next_hp: Instant,
    events: Vec<Ev>,
    rng: u32,
    /// per track: when its last step played, and its step length (to place recorded notes)
    last_step_t: [Option<Instant>; TRACKS],
    step_dur: [Duration; TRACKS],
    /// per track: a step just recorded ahead of the playhead, not replayed on this pass (you already heard it)
    skip_once: [Option<u8>; TRACKS],
    /// recent note-ons sent (channel, note, time): the same note arriving straight back is an echo, not playing
    sent: Vec<(u8, u8, Instant)>,
}

impl Engine {
    fn new(song: Song) -> Self {
        let seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.subsec_nanos()).unwrap_or(1) | 1;
        Engine { song, playing: false, pos: [0; TRACKS], playing_step: [None; TRACKS], hp_count: 0, next_hp: Instant::now(), events: vec![], rng: seed, last_step_t: [None; TRACKS], step_dur: [Duration::from_millis(125); TRACKS], skip_once: [None; TRACKS], sent: vec![] }
    }

    fn hp_dur(&self) -> Duration {
        Duration::from_secs_f64(60.0 / (self.song.bpm.max(1) as f64 * 48.0))
    }

    fn random_percent(&mut self) -> u8 {
        // xorshift32
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng = x;
        (x % 100) as u8 + 1
    }

    fn push(&mut self, due: Instant, msg: Vec<u8>) {
        self.events.push(Ev { due, msg });
    }

    pub fn start(&mut self) {
        if self.playing {
            return;
        }
        let now = Instant::now();
        self.playing = true;
        self.hp_count = 0;
        self.next_hp = now + Duration::from_millis(5);
        self.pos = [0; TRACKS];
        for t in &mut self.song.tracks {
            if let Some((a, b)) = t.queued.take() {
                t.chain_start = a;
                t.chain_end = b;
                t.pattern = a;
            }
        }
        if self.song.send_clock {
            self.push(now, vec![0xfa]);
        }
    }

    pub fn stop(&mut self) {
        if !self.playing {
            return;
        }
        self.playing = false;
        let now = Instant::now();
        // sounding notes end now; notes not yet started are dropped
        let offs: Vec<Vec<u8>> = self.events.drain(..).map(|e| e.msg).filter(|m| m[0] & 0xf0 == 0x80).collect();
        for m in offs {
            self.push(now, m);
        }
        self.playing_step = [None; TRACKS];
        self.last_step_t = [None; TRACKS];
        self.skip_once = [None; TRACKS];
        if self.song.send_clock {
            self.push(now, vec![0xfc]);
        }
    }

    /// Plays a note now for `ms` milliseconds (note pad preview).
    pub fn preview(&mut self, channel: u8, note: u8, vel: u8, ms: u64) {
        let now = Instant::now();
        self.push(now, vec![0x90 | channel, note, vel]);
        self.push(now + Duration::from_millis(ms), vec![0x80 | channel, note, 0]);
    }

    fn half_pulse(&mut self, t: Instant) {
        let hp = self.hp_count;
        self.hp_count += 1;
        let hp_dur = self.hp_dur();
        if self.song.send_clock && hp % 2 == 0 {
            self.push(t, vec![0xf8]);
        }
        let any_solo = self.song.tracks.iter().any(|t| t.solo);
        for ti in 0..TRACKS {
            let tr = &self.song.tracks[ti];
            let rate = RATE_HP[tr.rate as usize % RATE_HP.len()];
            if hp % rate as u64 != 0 {
                continue;
            }
            let step_dur = hp_dur * rate;
            let mut pos = self.pos[ti];
            if pos >= self.song.tracks[ti].pat().length {
                pos = 0;
                self.song.tracks[ti].next_pattern();
            }
            let swing = self.song.swing.clamp(50, 75) as u32;
            let delay = if rate == 12 && pos % 2 == 1 { step_dur * 2 * (swing - 50) / 100 } else { Duration::ZERO };
            self.playing_step[ti] = Some(pos);
            self.pos[ti] = pos + 1;
            self.last_step_t[ti] = Some(t + delay);
            self.step_dur[ti] = step_dur;
            if self.skip_once[ti].take() == Some(pos) {
                continue;
            }
            let tr = &self.song.tracks[ti];
            let step = tr.pat().steps[pos as usize].clone();
            let (ch, tp, playable) = (tr.channel & 15, tr.transpose, !tr.muted && (!any_solo || tr.solo));
            if !step.on || !playable || self.random_percent() > step.prob {
                continue;
            }
            let on_t = t + delay;
            let gate = (step_dur * step.gate as u32 / 8).saturating_sub(Duration::from_millis(2)).max(Duration::from_millis(5));
            for &n in &step.notes {
                let nn = (n as i32 + tp as i32).clamp(0, 127) as u8;
                self.push(on_t, vec![0x90 | ch, nn, step.vel.max(1)]);
                self.push(on_t + gate, vec![0x80 | ch, nn, 0]);
            }
        }
    }

    /// Advances the clock to `now` and returns the messages due.
    fn tick(&mut self, now: Instant) -> Vec<Vec<u8>> {
        if self.playing {
            if now > self.next_hp + Duration::from_millis(250) {
                // stalled (sleep, debugger): skip ahead instead of firing a burst
                self.next_hp = now;
            }
            while now >= self.next_hp {
                let t = self.next_hp;
                self.half_pulse(t);
                self.next_hp += self.hp_dur();
            }
        }
        if self.events.is_empty() {
            return vec![];
        }
        self.events.sort_by_key(|e| e.due);
        let n = self.events.iter().take_while(|e| e.due <= now).count();
        let out: Vec<Vec<u8>> = self.events.drain(..n).map(|e| e.msg).collect();
        self.sent.retain(|s| now.duration_since(s.2) < Duration::from_millis(100));
        for m in &out {
            if m[0] & 0xf0 == 0x90 {
                self.sent.push((m[0] & 15, m[1], now));
            }
        }
        out
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Page state shared by the screen and the Launchpad

/// A position on the Launchpad: row -1 = top buttons, 0-7 = grid (0 = top), 8 = track row, 9 = bottom row;
/// col -1 = left buttons, 0-7 = grid columns, 8 = right buttons.
type Pos = (i8, i8);

const SHIFT: Pos = (-1, -1);
const LEFT: Pos = (-1, 0);
const RIGHT: Pos = (-1, 1);
const EDITOR: Pos = (-1, 3);
const SEQ_BTN: Pos = (-1, 6);
const PROJECTS: Pos = (-1, 7);
const UP: Pos = (0, -1);
const DOWN: Pos = (1, -1);
const CLEAR: Pos = (2, -1);
const DUPLICATE: Pos = (3, -1);
const FIXED: Pos = (5, -1);
const PLAY: Pos = (6, -1);
const RECORD: Pos = (7, -1);
const PATTERNS_BTN: Pos = (0, 8);
const MUTE: Pos = (9, 1);
const SOLO: Pos = (9, 2);

fn label(p: Pos) -> &'static str {
    match p {
        SHIFT => "shift",
        LEFT => "\u{25c0}",
        RIGHT => "\u{25b6}",
        (-1, 2) => "session",
        EDITOR => "editor",
        (-1, 4) => "chord",
        (-1, 5) => "custom",
        SEQ_BTN => "seq",
        PROJECTS => "projects",
        UP => "\u{25b2}",
        DOWN => "\u{25bc}",
        CLEAR => "clear",
        DUPLICATE => "dupl.",
        (4, -1) => "quant.",
        FIXED => "fixed len",
        PLAY => "play",
        RECORD => "record",
        PATTERNS_BTN => "patterns",
        (9, 0) => "rec arm",
        MUTE => "mute",
        SOLO => "solo",
        (9, 3) => "volume",
        (9, 4) => "pan",
        (9, 5) => "sends",
        (9, 6) => "device",
        (9, 7) => "stop clip",
        _ => "",
    }
}

/// Launchpad Pro MK3 Programmer-mode index of a position.
fn lp_index(p: Pos) -> Option<u8> {
    match p {
        (-1, -1) => Some(90),
        (-1, c @ 0..=7) => Some(91 + c as u8),
        (r @ 0..=7, -1) => Some((8 - r as u8) * 10),
        (r @ 0..=7, c @ 0..=7) => Some((8 - r as u8) * 10 + c as u8 + 1),
        (r @ 0..=7, 8) => Some((8 - r as u8) * 10 + 9),
        (8, c @ 0..=7) => Some(101 + c as u8),
        (9, c @ 0..=7) => Some(1 + c as u8),
        _ => None,
    }
}

fn pos_of_index(i: u8) -> Option<Pos> {
    match i {
        90 => Some(SHIFT),
        91..=98 => Some((-1, (i - 91) as i8)),
        101..=108 => Some((8, (i - 101) as i8)),
        1..=8 => Some((9, (i - 1) as i8)),
        10..=89 => {
            let (row, col) = (i / 10, i % 10);
            let r = 8 - row as i8;
            match col {
                0 => Some((r, -1)),
                1..=8 => Some((r, col as i8 - 1)),
                9 => Some((r, 8)),
                _ => None,
            }
        }
        _ => None,
    }
}

/// Colour hues shared by the screen and the Launchpad palette (`lp` = palette index of the brightest shade;
/// the next three indices are darker shades).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Hue {
    White,
    Red,
    Orange,
    Yellow,
    Green,
    Cyan,
    Blue,
    Purple,
    Pink,
}
const TRACK_HUE: [Hue; TRACKS] = [Hue::Red, Hue::Orange, Hue::Yellow, Hue::Green, Hue::Cyan, Hue::Blue, Hue::Purple, Hue::Pink];

impl Hue {
    fn rgb(self) -> (u8, u8, u8) {
        match self {
            Hue::White => (255, 255, 255),
            Hue::Red => (255, 60, 50),
            Hue::Orange => (255, 140, 30),
            Hue::Yellow => (250, 225, 40),
            Hue::Green => (60, 230, 80),
            Hue::Cyan => (40, 220, 230),
            Hue::Blue => (60, 110, 255),
            Hue::Purple => (170, 80, 255),
            Hue::Pink => (255, 80, 190),
        }
    }
    fn lp(self) -> u8 {
        match self {
            Hue::White => 0,
            Hue::Red => 4,
            Hue::Orange => 8,
            Hue::Yellow => 12,
            Hue::Green => 20,
            Hue::Cyan => 32,
            Hue::Blue => 44,
            Hue::Purple => 48,
            Hue::Pink => 56,
        }
    }
}

/// A pad colour: hue and level 0 (off) ... 4 (brightest).
#[derive(Clone, Copy, PartialEq, Eq)]
struct Lit(Hue, u8);
const OFF: Lit = Lit(Hue::White, 0);

impl Lit {
    fn color(self) -> Color32 {
        const F: [f32; 5] = [0.0, 0.18, 0.38, 0.7, 1.0];
        let Lit(h, l) = self;
        if l == 0 {
            return Color32::from_rgb(0x26, 0x27, 0x2b);
        }
        let (r, g, b) = h.rgb();
        let f = F[l.min(4) as usize];
        let mix = |c: u8| (0x26 as f32 + (c as f32 - 0x26 as f32) * f) as u8;
        Color32::from_rgb(mix(r), mix(g), mix(b))
    }
    /// Novation palette index (Programmer mode note-on velocity).
    fn palette(self) -> u8 {
        let Lit(h, l) = self;
        match (h, l) {
            (_, 0) => 0,
            (Hue::White, 1) => 1,
            (Hue::White, 2) => 2,
            (Hue::White, _) => 3,
            (h, l) => h.lp() + (4 - l.min(4)),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Arm {
    None,
    Mute,
    Solo,
}

/// Interaction state of the page (the same for the screen and the Launchpad).
struct Page {
    sel: usize,
    page: u8,
    /// step held for editing, with: was lit when pressed / edited during the hold / a note pad was tapped
    held: Option<u8>,
    held_was_on: bool,
    held_edited: bool,
    held_notes_set: bool,
    /// hold buttons: physically held on the Launchpad, or latched on screen
    shift: bool,
    clear: bool,
    dup: bool,
    fixed: bool,
    patterns: bool,
    projects: bool,
    arm: Arm,
    copy_src: Option<u8>,
    /// Patterns view: pattern pressed first (for chaining a range)
    pat_hold: Option<(usize, u8)>,
    slot: usize,
    /// request to show the editor page
    go_editor: bool,
    status: Option<String>,
    /// recording armed (Record): live while playing, step recording while stopped
    rec: bool,
    /// live recording: notes held now (note, step, start), and the last step written (step, time) so notes
    /// played together land on it as a chord
    rec_held: Vec<(u8, u8, Instant)>,
    rec_step: Option<(u8, Instant)>,
    /// step recording: the step the next note goes to, and how many notes are held on it
    cursor: u8,
    cursor_held: u8,
    /// velocity of the Launchpad pad being pressed
    pad_vel: u8,
    /// Step Rec: each note or chord goes on the cursor step, which then moves on
    step_rec: bool,
    /// tap tempo: recent taps
    taps: Vec<Instant>,
}

impl Default for Page {
    fn default() -> Self {
        Page {
            sel: 0,
            page: 0,
            held: None,
            held_was_on: false,
            held_edited: false,
            held_notes_set: false,
            shift: false,
            clear: false,
            dup: false,
            fixed: false,
            patterns: false,
            projects: false,
            arm: Arm::None,
            copy_src: None,
            pat_hold: None,
            slot: 0,
            go_editor: false,
            status: None,
            rec: false,
            rec_held: vec![],
            rec_step: None,
            cursor: 0,
            cursor_held: 0,
            pad_vel: 100,
            step_rec: false,
            taps: vec![],
        }
    }
}

/// Everything behind the one lock: engine, page state, project slots, Launchpad link.
struct Core {
    eng: Engine,
    pg: Page,
    slots: Vec<Option<Song>>,
    slots_dirty: bool,
    lp_out: Option<MidiOutputConnection>,
    lp_rx: Option<Receiver<Vec<u8>>>,
    /// last palette value sent per Launchpad index (None = unknown)
    lp_leds: [Option<u8>; 128],
    lp_last_draw: Instant,
}

// ---------------------------------------------------------------------------------------------------------------
// Page logic

impl Core {
    fn track(&mut self) -> &mut Track {
        let s = self.pg.sel;
        &mut self.eng.song.tracks[s]
    }

    fn held_step(&mut self) -> Option<&mut Step> {
        let h = self.pg.held? as usize;
        Some(&mut self.track().pat_mut().steps[h])
    }

    fn status(&mut self, s: impl Into<String>) {
        self.pg.status = Some(s.into());
    }

    /// A press. `physical` = from the Launchpad (a release follows); otherwise a screen click / tap.
    /// `alt` = screen right-click / long-press.
    fn press(&mut self, p: Pos, physical: bool, alt: bool) {
        let latch = |v: &mut bool| *v = if physical { true } else { !*v };
        // while a step is held, the right column is its velocity / probability bar and the bottom row its gate
        if self.pg.held.is_some() {
            match p {
                (r @ 0..=7, 8) => return self.right_column(r as usize),
                (9, c @ 0..=7) => {
                    if let Some(s) = self.held_step() {
                        s.gate = c as u8 + 1;
                    }
                    self.pg.held_edited = true;
                    return;
                }
                _ => {}
            }
        }
        match p {
            SHIFT => latch(&mut self.pg.shift),
            CLEAR => {
                latch(&mut self.pg.clear);
                if self.pg.clear {
                    self.pg.dup = false;
                    self.pg.fixed = false;
                }
            }
            DUPLICATE => {
                latch(&mut self.pg.dup);
                self.pg.copy_src = None;
                if self.pg.dup {
                    self.pg.clear = false;
                    self.pg.fixed = false;
                }
            }
            FIXED => {
                latch(&mut self.pg.fixed);
                if self.pg.fixed {
                    self.pg.clear = false;
                    self.pg.dup = false;
                }
            }
            PATTERNS_BTN => {
                latch(&mut self.pg.patterns);
                self.pg.pat_hold = None;
                if self.pg.patterns {
                    self.pg.projects = false;
                    self.pg.held = None;
                }
            }
            PROJECTS => {
                latch(&mut self.pg.projects);
                if self.pg.projects {
                    self.pg.patterns = false;
                    self.pg.held = None;
                }
            }
            MUTE | SOLO => {
                let a = if p == MUTE { Arm::Mute } else { Arm::Solo };
                self.pg.arm = if self.pg.arm == a { Arm::None } else { a };
            }
            LEFT => self.pg.page = 0,
            RIGHT => self.pg.page = 1,
            EDITOR => self.pg.go_editor = true,
            UP | DOWN => {
                let shift = self.pg.shift;
                if shift {
                    let d: i32 = if self.pg.fixed { 10 } else { 1 };
                    let bpm = self.eng.song.bpm as i32 + if p == UP { d } else { -d };
                    self.eng.song.bpm = bpm.clamp(30, 300) as u16;
                    let b = self.eng.song.bpm;
                    self.status(format!("Tempo {b} BPM"));
                } else {
                    let t = self.track();
                    t.base = if p == UP { (t.base + 12).min(96) } else { t.base.saturating_sub(12) };
                    let b = t.base;
                    self.status(format!("Notes from {}", note_name(b)));
                }
            }
            PLAY => {
                if self.eng.playing {
                    self.eng.stop()
                } else {
                    self.pg.rec_held.clear();
                    self.pg.rec_step = None;
                    self.eng.start()
                }
            }
            RECORD => {
                if self.eng.playing { self.toggle_record() } else { self.toggle_step_rec() }
            }
            (9, 5) if self.pg.shift => self.tap(),
            (r @ 0..=7, 8) => self.right_column(r as usize),
            (8, c @ 0..=7) => self.track_button(c as usize),
            (r @ 0..=7, c @ 0..=7) => {
                let (r, c) = (r as usize, c as usize);
                if self.pg.patterns {
                    if r < 2 {
                        self.pattern_pad((r * 8 + c) as u8);
                    }
                    if !physical {
                        // a screen tap has no release, so it must not stay held (that would chain the next tap)
                        self.pg.pat_hold = None;
                    }
                } else if self.pg.projects {
                    if r * 8 + c < SLOTS {
                        self.project_slot(r * 8 + c);
                    }
                } else if r < 4 {
                    self.step_press(self.pg.page * 32 + (r * 8 + c) as u8, physical, alt);
                } else {
                    let t = &self.eng.song.tracks[self.pg.sel];
                    let note = (t.base as usize + (7 - r) * 8 + c).min(127) as u8;
                    let vel = if physical { self.pg.pad_vel.max(1) } else { 100 };
                    self.note_press(note, vel);
                    if !physical {
                        // a screen tap has no release
                        self.rec_note_off(note);
                    }
                }
            }
            _ => {}
        }
    }

    fn release(&mut self, p: Pos) {
        match p {
            SHIFT => self.pg.shift = false,
            CLEAR => self.pg.clear = false,
            DUPLICATE => {
                self.pg.dup = false;
                self.pg.copy_src = None;
            }
            FIXED => self.pg.fixed = false,
            PATTERNS_BTN => {
                self.pg.patterns = false;
                self.pg.pat_hold = None;
            }
            PROJECTS => self.pg.projects = false,
            (r @ 0..=3, c @ 0..=7) if !self.pg.patterns && !self.pg.projects => {
                let idx = self.pg.page * 32 + (r as u8) * 8 + c as u8;
                if self.pg.held == Some(idx) {
                    if self.pg.held_was_on && !self.pg.held_edited {
                        self.track().pat_mut().steps[idx as usize].on = false;
                    }
                    self.pg.held = None;
                }
            }
            (r @ 4..=7, c @ 0..=7) if !self.pg.patterns && !self.pg.projects => {
                let t = &self.eng.song.tracks[self.pg.sel];
                let note = (t.base as usize + (7 - r as usize) * 8 + c as usize).min(127) as u8;
                self.rec_note_off(note);
            }
            (r @ 0..=7, c @ 0..=7) if self.pg.patterns => {
                if self.pg.pat_hold.is_some_and(|(_, p)| p as usize == r as usize * 8 + c as usize) {
                    self.pg.pat_hold = None;
                }
            }
            _ => {}
        }
    }

    fn step_press(&mut self, idx: u8, physical: bool, alt: bool) {
        let i = idx as usize;
        let note = self.track().note;
        if self.pg.fixed {
            self.track().pat_mut().length = idx + 1;
            self.status(format!("Pattern length {}", idx + 1));
            return;
        }
        if self.pg.clear {
            self.track().pat_mut().steps[i] = Step::default();
            return;
        }
        if self.pg.dup {
            match self.pg.copy_src {
                None => {
                    if self.track().pat().steps[i].on {
                        self.pg.copy_src = Some(idx);
                        self.status(format!("Copying step {} - tap steps to paste", idx + 1));
                    }
                }
                Some(src) => {
                    let s = self.track().pat().steps[src as usize].clone();
                    self.track().pat_mut().steps[i] = s;
                }
            }
            return;
        }
        if self.pg.step_rec && !alt {
            self.pg.cursor = idx;
            self.pg.cursor_held = 0;
            return;
        }
        if physical {
            // Launchpad: the step is held until released; tap alone turns a lit step off
            let was_on = self.track().pat().steps[i].on;
            if !was_on {
                self.track().pat_mut().steps[i] = Step { notes: vec![note], ..Step::single(note) };
            }
            self.pg.held = Some(idx);
            self.pg.held_was_on = was_on;
            self.pg.held_edited = false;
            self.pg.held_notes_set = false;
            return;
        }
        if alt {
            // screen: right-click / long-press holds (latches) a step for editing
            if self.pg.held == Some(idx) {
                self.pg.held = None;
            } else {
                if !self.track().pat().steps[i].on {
                    self.track().pat_mut().steps[i] = Step::single(note);
                }
                self.pg.held = Some(idx);
                self.pg.held_was_on = false;
                self.pg.held_edited = false;
                self.pg.held_notes_set = false;
            }
            return;
        }
        if self.pg.held.is_some() {
            // another step while one is held on screen: move the hold there
            if self.pg.held == Some(idx) {
                self.pg.held = None;
            } else {
                if !self.track().pat().steps[i].on {
                    self.track().pat_mut().steps[i] = Step::single(note);
                }
                self.pg.held = Some(idx);
                self.pg.held_notes_set = false;
            }
            return;
        }
        let s = &mut self.track().pat_mut().steps[i];
        if s.on {
            s.on = false;
        } else {
            *s = Step::single(note);
        }
    }

    /// Rec: live recording while playing.
    fn toggle_record(&mut self) {
        self.pg.rec = !self.pg.rec;
        self.pg.rec_held.clear();
        self.pg.rec_step = None;
        if self.pg.rec {
            self.pg.step_rec = false;
            self.pg.held = None;
            self.status(if self.eng.playing { "Recording: notes you play are written into the pattern" } else { "Rec armed: recording starts when you press Play" });
        } else {
            self.status("Recording off");
        }
    }

    /// Step Rec: each note or chord goes on the white step, then it moves on.
    fn toggle_step_rec(&mut self) {
        self.pg.step_rec = !self.pg.step_rec;
        self.pg.cursor_held = 0;
        if self.pg.step_rec {
            self.pg.rec = false;
            self.pg.cursor = self.pg.page * 32;
            self.pg.held = None;
            self.status("Step recording: each note (or chord) goes on the white step, then it moves on. Tap a step to move it.");
        } else {
            self.status("Step recording off");
        }
    }

    /// Tap tempo: the average of the last few taps sets the BPM (a 2 s pause starts over).
    fn tap(&mut self) {
        let now = Instant::now();
        if self.pg.taps.last().is_some_and(|t| now.duration_since(*t) > Duration::from_secs(2)) {
            self.pg.taps.clear();
        }
        self.pg.taps.push(now);
        if self.pg.taps.len() > 5 {
            self.pg.taps.remove(0);
        }
        let n = self.pg.taps.len();
        if n >= 2 {
            let span = self.pg.taps[n - 1].duration_since(self.pg.taps[0]).as_secs_f64() / (n - 1) as f64;
            if span > 0.0 {
                self.eng.song.bpm = (60.0 / span).round().clamp(30.0, 300.0) as u16;
            }
            let b = self.eng.song.bpm;
            self.status(format!("Tap tempo: {b} BPM"));
        } else {
            self.status("Tap tempo: keep tapping");
        }
    }

    /// A note played while recording (note pads, or a keyboard on the MIDI input).
    fn rec_note_on(&mut self, note: u8, vel: u8) {
        if !(self.pg.rec || self.pg.step_rec) || self.pg.patterns || self.pg.projects {
            return;
        }
        let t = self.pg.sel;
        let now = Instant::now();
        if !self.pg.step_rec {
            if !self.eng.playing {
                return;
            }
            let (Some(last), Some(cur)) = (self.eng.last_step_t[t], self.eng.playing_step[t]) else { return };
            let dur = self.eng.step_dur[t];
            let len = self.eng.song.tracks[t].pat().length.max(1);
            // nearest step: the one playing now, or the next one (not replayed on this pass: you just heard it)
            let late = now.saturating_duration_since(last) >= dur / 2;
            let step = if late { (cur + 1) % len } else { cur };
            if late {
                self.eng.skip_once[t] = Some(step);
            }
            let chord = self.pg.rec_step.is_some_and(|(st, at)| st == step && now.duration_since(at) < dur);
            let s = &mut self.eng.song.tracks[t].pat_mut().steps[step as usize];
            if chord && s.on {
                if !s.notes.contains(&note) && s.notes.len() < MAX_NOTES {
                    s.notes.push(note);
                }
                s.vel = s.vel.max(vel);
            } else {
                *s = Step { vel, ..Step::single(note) };
            }
            self.pg.rec_step = Some((step, now));
            self.pg.rec_held.retain(|h| h.0 != note);
            self.pg.rec_held.push((note, step, now));
        } else {
            let c = self.pg.cursor.min(STEPS as u8 - 1) as usize;
            let first = self.pg.cursor_held == 0;
            let s = &mut self.eng.song.tracks[t].pat_mut().steps[c];
            if first {
                *s = Step { vel, ..Step::single(note) };
            } else if !s.notes.contains(&note) && s.notes.len() < MAX_NOTES {
                s.notes.push(note);
            }
            self.pg.cursor_held += 1;
        }
    }

    fn rec_note_off(&mut self, note: u8) {
        if !(self.pg.rec || self.pg.step_rec) {
            return;
        }
        let t = self.pg.sel;
        if !self.pg.step_rec {
            let Some(i) = self.pg.rec_held.iter().position(|h| h.0 == note) else { return };
            let (_, step, start) = self.pg.rec_held.remove(i);
            let dur = self.eng.step_dur[t].as_secs_f64().max(0.001);
            let gate = (start.elapsed().as_secs_f64() / dur * 8.0).round().clamp(1.0, 8.0) as u8;
            let s = &mut self.eng.song.tracks[t].pat_mut().steps[step as usize];
            if s.on && s.notes.contains(&note) {
                s.gate = gate;
            }
        } else if self.pg.cursor_held > 0 {
            self.pg.cursor_held -= 1;
            if self.pg.cursor_held == 0 {
                self.pg.cursor += 1;
                let len = self.eng.song.tracks[t].pat().length;
                if self.pg.cursor >= len {
                    self.pg.step_rec = false;
                    self.pg.cursor = 0;
                    self.status("Step recording finished (end of the pattern)");
                } else {
                    self.pg.page = self.pg.cursor / 32;
                }
            }
        }
    }

    /// Notes from the MIDI input (a keyboard): recorded when Record is on.
    fn midi_in(&mut self, status: u8, d1: u8, d2: u8) {
        let kind = status & 0xf0;
        let ch = status & 15;
        if kind == 0x90 && d2 > 0 {
            let now = Instant::now();
            if self.eng.sent.iter().any(|s| s.0 == ch && s.1 == d1 && now.duration_since(s.2) < Duration::from_millis(60)) {
                return; // our own note coming back (an instrument with MIDI thru)
            }
            self.rec_note_on(d1, d2);
        } else if kind == 0x80 || kind == 0x90 {
            self.rec_note_off(d1);
        }
    }

    fn note_press(&mut self, note: u8, vel: u8) {
        let (ch, tp) = {
            let t = self.track();
            t.note = note;
            (t.channel, t.transpose)
        };
        let pn = (note as i32 + tp as i32).clamp(0, 127) as u8;
        self.eng.preview(ch, pn, 100, 250);
        if self.pg.held.is_none() {
            self.rec_note_on(note, vel);
        }
        if self.pg.held.is_some() {
            let first = !self.pg.held_notes_set;
            if let Some(s) = self.held_step() {
                if first {
                    s.notes = vec![note];
                } else {
                    s.toggle_note(note);
                }
            }
            self.pg.held_notes_set = true;
            self.pg.held_edited = true;
        }
    }

    fn right_column(&mut self, r: usize) {
        if self.pg.held.is_some() {
            let shift = self.pg.shift;
            if let Some(s) = self.held_step() {
                if shift {
                    s.prob = PROB_BAR[r];
                } else {
                    s.vel = VEL_BAR[r];
                }
            }
            self.pg.held_edited = true;
        }
    }

    fn track_button(&mut self, t: usize) {
        if self.pg.projects {
            return;
        }
        if self.pg.clear {
            let tr = &mut self.eng.song.tracks[t];
            let len = tr.pat().length;
            *tr.pat_mut() = Pattern { length: len, ..Pattern::default() };
            self.status(format!("Track {} pattern cleared", t + 1));
            return;
        }
        match self.pg.arm {
            Arm::Mute => {
                let tr = &mut self.eng.song.tracks[t];
                tr.muted = !tr.muted;
            }
            Arm::Solo => {
                let tr = &mut self.eng.song.tracks[t];
                tr.solo = !tr.solo;
            }
            Arm::None => {
                self.pg.sel = t;
                self.pg.held = None;
            }
        }
    }

    /// Tap pattern `p` (0-15). Patterns are song-wide, so every track switches together.
    fn pattern_pad(&mut self, p: u8) {
        let p = p.min(PATTERNS as u8 - 1);
        let tracks = &mut self.eng.song.tracks;
        if self.pg.clear {
            for tr in tracks.iter_mut() {
                let len = tr.patterns[p as usize].length;
                tr.patterns[p as usize] = Pattern { length: len, ..Pattern::default() };
            }
            // one go only, so the next tap selects again
            self.pg.clear = false;
            self.status(format!("Pattern {} cleared", p + 1));
            return;
        }
        if self.pg.dup {
            let from = tracks[0].pattern;
            for tr in tracks.iter_mut() {
                let src = tr.pat().clone();
                tr.patterns[p as usize] = src;
            }
            self.pg.dup = false;
            self.status(format!("Pattern {} copied to {}", from + 1, p + 1));
            return;
        }
        // a second pattern pressed while the first is held (or, on screen, with "Chain to it"): chain the range
        let first = match self.pg.pat_hold {
            Some((_, hp)) if hp != p => Some(hp),
            _ if self.pg.shift => Some(tracks[0].pattern),
            _ => None,
        };
        let (a, b) = match first {
            Some(f) => (f.min(p), f.max(p)),
            None => (p, p),
        };
        if first.is_none() {
            self.pg.pat_hold = Some((0, p));
        }
        let playing = self.eng.playing;
        for tr in tracks.iter_mut() {
            // an empty pattern takes the length of the current one
            let cur_len = tr.pat().length;
            if !tr.patterns[p as usize].has_notes() {
                tr.patterns[p as usize].length = cur_len;
            }
            if playing {
                tr.queued = Some((a, b));
            } else {
                tr.chain_start = a;
                tr.chain_end = b;
                tr.pattern = a;
            }
        }
        let msg = if a == b { format!("Pattern {}", a + 1) } else { format!("Chain patterns {}-{}", a + 1, b + 1) };
        self.status(msg);
    }

    /// Tap a project: the current song is kept in its own project, then project
    /// `s` opens (a blank song if it has never been used).
    fn project_slot(&mut self, s: usize) {
        let s = s.min(SLOTS - 1);
        if s == self.pg.slot {
            self.status(format!("Project {} is open", s + 1));
            return;
        }
        let cur = self.pg.slot;
        self.slots[cur] = Some(self.eng.song.clone());
        let mut song = self.slots[s].clone().unwrap_or_default();
        song.project = s as u8;
        song.normalize();
        let playing = self.eng.playing;
        self.eng.stop();
        self.eng.song = song;
        if playing {
            self.eng.start();
        }
        self.slots_dirty = true;
        self.pg.slot = s;
        self.pg.held = None;
        self.pg.pat_hold = None;
        let new = if self.slots[s].is_some() { "" } else { " (new)" };
        self.status(format!("Project {} kept, project {}{} open", cur + 1, s + 1, new));
    }

    /// Colour of project `i`: open = bright white, used = dim white.
    fn slot_lit(&self, i: usize) -> Lit {
        if self.pg.slot == i {
            Lit(Hue::White, 4)
        } else if self.slots[i].is_some() {
            Lit(Hue::White, 1)
        } else {
            OFF
        }
    }

    /// Colour of a pad.
    fn lit(&self, p: Pos) -> Lit {
        let pg = &self.pg;
        let song = &self.eng.song;
        let tr = &song.tracks[pg.sel];
        let hue = TRACK_HUE[pg.sel];
        let on = |b: bool, h: Hue| if b { Lit(h, 4) } else { Lit(Hue::White, 1) };
        match p {
            SHIFT => on(pg.shift, Hue::White),
            LEFT => {
                let other = self.eng.playing_step[pg.sel].is_some_and(|s| s < 32) && pg.page == 1;
                if pg.page == 0 { Lit(Hue::White, 4) } else if other { Lit(Hue::Green, 3) } else { Lit(Hue::White, 1) }
            }
            RIGHT => {
                let other = self.eng.playing_step[pg.sel].is_some_and(|s| s >= 32) && pg.page == 0;
                if pg.page == 1 { Lit(Hue::White, 4) } else if other { Lit(Hue::Green, 3) } else if tr.pat().length > 32 { Lit(Hue::White, 2) } else { Lit(Hue::White, 1) }
            }
            EDITOR => Lit(Hue::Blue, 4),
            SEQ_BTN => {
                if self.eng.playing { Lit(Hue::Green, 4) } else { Lit(Hue::White, 3) }
            }
            PROJECTS => on(pg.projects, Hue::Cyan),
            UP | DOWN => Lit(Hue::White, 2),
            CLEAR => on(pg.clear, Hue::Red),
            DUPLICATE => on(pg.dup, Hue::Cyan),
            FIXED => on(pg.fixed, Hue::Orange),
            PLAY => {
                if self.eng.playing { Lit(Hue::Green, 4) } else { Lit(Hue::Green, 1) }
            }
            RECORD => {
                if pg.rec || pg.step_rec { Lit(Hue::Red, 4) } else { Lit(Hue::Red, 1) }
            }
            MUTE => if pg.arm == Arm::Mute { Lit(Hue::Yellow, 4) } else { Lit(Hue::Yellow, 1) },
            SOLO => if pg.arm == Arm::Solo { Lit(Hue::Blue, 4) } else { Lit(Hue::Blue, 1) },
            (r @ 0..=7, 8) => {
                let r = r as usize;
                if let Some(h) = pg.held {
                    let s = &tr.pat().steps[h as usize];
                    if pg.shift {
                        if s.prob >= PROB_BAR[r] { Lit(Hue::Purple, 4) } else { Lit(Hue::White, 1) }
                    } else if s.vel >= VEL_BAR[r] {
                        Lit(Hue::Red, 3)
                    } else {
                        Lit(Hue::White, 1)
                    }
                } else if r == 0 {
                    on(pg.patterns, Hue::Orange)
                } else {
                    OFF
                }
            }
            (8, c @ 0..=7) => {
                let c = c as usize;
                if pg.projects {
                    return OFF;
                }
                let t = &song.tracks[c];
                let any_solo = song.tracks.iter().any(|t| t.solo);
                let h = TRACK_HUE[c];
                match pg.arm {
                    Arm::Mute => if t.muted { Lit(Hue::White, 1) } else { Lit(h, 3) },
                    Arm::Solo => if t.solo { Lit(h, 4) } else { Lit(Hue::White, 1) },
                    Arm::None => {
                        let silent = t.muted || (any_solo && !t.solo);
                        if c == pg.sel { Lit(h, 4) } else if silent { Lit(Hue::White, 1) } else { Lit(h, 2) }
                    }
                }
            }
            (9, c @ 0..=7) => {
                if let Some(h) = pg.held {
                    let g = tr.pat().steps[h as usize].gate;
                    if (c as u8) < g { Lit(Hue::Green, 3) } else { Lit(Hue::White, 1) }
                } else {
                    OFF
                }
            }
            (r @ 0..=7, c @ 0..=7) => {
                let (r, c) = (r as usize, c as usize);
                if pg.patterns {
                    if r >= 2 {
                        return OFF;
                    }
                    let p = (r * 8 + c) as u8;
                    let t = &song.tracks[0];
                    return if t.queued.is_some_and(|(a, b)| p >= a && p <= b) {
                        Lit(Hue::White, 3)
                    } else if p == t.pattern {
                        Lit(Hue::White, 4)
                    } else if p >= t.chain_start && p <= t.chain_end && t.chain_end > t.chain_start {
                        Lit(Hue::Orange, 3)
                    } else if song.tracks.iter().any(|t| t.patterns[p as usize].has_notes()) {
                        Lit(Hue::Orange, 1)
                    } else {
                        OFF
                    };
                }
                if pg.projects {
                    return if r * 8 + c < SLOTS { self.slot_lit(r * 8 + c) } else { OFF };
                }
                if r < 4 {
                    let idx = pg.page * 32 + (r * 8 + c) as u8;
                    let pat = tr.pat();
                    if idx >= pat.length {
                        return OFF;
                    }
                    if pg.step_rec && pg.cursor == idx {
                        return Lit(Hue::White, 3);
                    }
                    if self.eng.playing_step[pg.sel] == Some(idx) {
                        return Lit(Hue::White, 4);
                    }
                    if pg.held == Some(idx) || pg.copy_src == Some(idx) {
                        return Lit(Hue::Cyan, 4);
                    }
                    let s = &pat.steps[idx as usize];
                    if s.on {
                        Lit(hue, if s.vel >= 100 { 4 } else if s.vel >= 60 { 3 } else { 2 })
                    } else {
                        Lit(Hue::White, 1)
                    }
                } else {
                    let note = (tr.base as usize + (7 - r) * 8 + c).min(127) as u8;
                    let in_held = pg.held.is_some_and(|h| tr.pat().steps[h as usize].notes.contains(&note));
                    if in_held {
                        Lit(hue, 4)
                    } else if note == tr.note {
                        Lit(hue, 3)
                    } else if note % 12 == 0 {
                        Lit(Hue::White, 2)
                    } else if [1, 3, 6, 8, 10].contains(&(note % 12)) {
                        OFF
                    } else {
                        Lit(Hue::White, 1)
                    }
                }
            }
            _ => OFF,
        }
    }

    /// Sends changed pad colours to the Launchpad.
    fn draw_lp(&mut self, force: bool) {
        if self.lp_out.is_none() {
            return;
        }
        let mut msgs: Vec<[u8; 3]> = vec![];
        for row in -1..=9i8 {
            for col in -1..=8i8 {
                let p = (row, col);
                let Some(i) = lp_index(p) else { continue };
                let v = self.lit(p).palette();
                if force || self.lp_leds[i as usize] != Some(v) {
                    self.lp_leds[i as usize] = Some(v);
                    msgs.push([0x90, i, v]);
                }
            }
        }
        if let Some(out) = self.lp_out.as_mut() {
            for m in msgs {
                if out.send(&m).is_err() {
                    break;
                }
            }
        }
    }

    /// Pad presses and releases from the Launchpad.
    fn poll_lp(&mut self) {
        let msgs: Vec<Vec<u8>> = match &self.lp_rx {
            Some(rx) => rx.try_iter().collect(),
            None => return,
        };
        for m in msgs {
            if m.len() < 3 {
                continue;
            }
            let kind = m[0] & 0xf0;
            if kind != 0x90 && kind != 0x80 && kind != 0xb0 {
                continue;
            }
            let Some(p) = pos_of_index(m[1]) else { continue };
            if kind != 0x80 && m[2] > 0 {
                self.pg.pad_vel = m[2];
                self.press(p, true, false);
            } else {
                self.release(p);
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Front end: owns the thread, the files and the Launchpad connection

const LP_PROGRAMMER: [u8; 9] = [0xf0, 0x00, 0x20, 0x29, 0x02, 0x0e, 0x0e, 0x01, 0xf7];
const LP_LIVE: [u8; 9] = [0xf0, 0x00, 0x20, 0x29, 0x02, 0x0e, 0x0e, 0x00, 0xf7];

fn is_lp_port(name: &str) -> bool {
    let n = name.to_lowercase();
    (n.contains("launchpad pro mk3") || n.contains("lppromk3")) && !n.contains("daw") && !n.contains("din") && !n.contains("emu")
}

fn seq_dir() -> Option<std::path::PathBuf> {
    crate::files::config_dir().map(|d| d.join("novaedit"))
}

/// Button symbols, drawn as shapes so they do not depend on the font having arrow, square or circle glyphs.
#[derive(Clone, Copy)]
enum Icon {
    Play,
    Stop,
    Rec,
    Left,
    Right,
    Up,
    Down,
}

fn paint_icon(p: &egui::Painter, c: Pos2, s: f32, icon: Icon, col: Color32) {
    let h = s / 2.0;
    let tri = |pts: [(f32, f32); 3]| egui::Shape::convex_polygon(pts.iter().map(|&(x, y)| c + vec2(x * h, y * h)).collect(), col, Stroke::NONE);
    match icon {
        Icon::Play | Icon::Right => {
            p.add(tri([(-0.8, -1.0), (-0.8, 1.0), (1.0, 0.0)]));
        }
        Icon::Left => {
            p.add(tri([(0.8, -1.0), (0.8, 1.0), (-1.0, 0.0)]));
        }
        Icon::Up => {
            p.add(tri([(-1.0, 0.8), (1.0, 0.8), (0.0, -1.0)]));
        }
        Icon::Down => {
            p.add(tri([(-1.0, -0.8), (1.0, -0.8), (0.0, 1.0)]));
        }
        Icon::Stop => {
            p.rect_filled(Rect::from_center_size(c, vec2(s * 0.85, s * 0.85)), 1.5, col);
        }
        Icon::Rec => {
            p.circle_filled(c, h, col);
        }
    }
}

/// A button with an optional symbol before its text (both centred together).
fn icon_button(ui: &mut Ui, size: [f32; 2], text: &str, icon: Option<Icon>, font: f32, fg: Color32, fill: Color32, stroke: Stroke) -> egui::Response {
    icon_button_c(ui, size, text, icon, None, font, fg, fill, stroke)
}

/// `icon_col`: the symbol's colour when it should differ from the text.
fn icon_button_c(ui: &mut Ui, size: [f32; 2], text: &str, icon: Option<Icon>, icon_col: Option<Color32>, font: f32, fg: Color32, fill: Color32, stroke: Stroke) -> egui::Response {
    let r = ui.add_sized(size, egui::Button::new("").fill(fill).stroke(stroke).corner_radius(6.0));
    let p = ui.painter();
    let galley = p.layout_no_wrap(text.to_string(), FontId::proportional(font), fg);
    let isz = font * 0.75;
    let gap = if text.is_empty() || icon.is_none() { 0.0 } else { font * 0.5 };
    let iw = if icon.is_some() { isz } else { 0.0 };
    let total = iw + gap + galley.size().x;
    let x0 = r.rect.center().x - total / 2.0;
    if let Some(i) = icon {
        paint_icon(p, Pos2::new(x0 + isz / 2.0, r.rect.center().y), isz, i, icon_col.unwrap_or(fg));
    }
    let gp = Pos2::new(x0 + iw + gap, r.rect.center().y - galley.size().y / 2.0);
    p.galley(gp, galley, fg);
    r
}

pub struct Seq {
    core: Arc<Mutex<Core>>,
    /// output shared with the editor (the Nova)
    out: Arc<Mutex<Option<Out>>>,
    quit: Arc<AtomicBool>,
    lp_in: Option<MidiInputConnection<()>>,
    pub lp_name: Option<String>,
    lp_checked: Option<Instant>,
    saved: Song,
    save_check: Instant,
}

impl Seq {
    pub fn new() -> Seq {
        let song = seq_dir()
            .and_then(|d| std::fs::read_to_string(d.join("seq_current.json")).ok())
            .and_then(|s| serde_json::from_str::<Song>(&s).ok())
            .map(|mut s| {
                s.normalize();
                s
            })
            .unwrap_or_default();
        let slots: Vec<Option<Song>> = seq_dir()
            .and_then(|d| std::fs::read_to_string(d.join("seq_projects.json")).ok())
            .and_then(|s| serde_json::from_str::<Vec<Option<Song>>>(&s).ok())
            .map(|mut v| {
                v.resize(SLOTS, None);
                for s in v.iter_mut().flatten() {
                    s.normalize();
                }
                v
            })
            .unwrap_or_else(|| vec![None; SLOTS]);
        let core = Arc::new(Mutex::new(Core {
            eng: Engine::new(song.clone()),
            pg: Page { slot: song.project as usize, ..Page::default() },
            slots,
            slots_dirty: false,
            lp_out: None,
            lp_rx: None,
            lp_leds: [None; 128],
            lp_last_draw: Instant::now(),
        }));
        let out: Arc<Mutex<Option<Out>>> = Arc::new(Mutex::new(None));
        let quit = Arc::new(AtomicBool::new(false));
        {
            let (core, out, quit) = (core.clone(), out.clone(), quit.clone());
            let _ = std::thread::Builder::new().name("sequencer".into()).spawn(move || {
                while !quit.load(Ordering::Relaxed) {
                    let now = Instant::now();
                    let msgs = {
                        let mut c = lock(&core);
                        c.poll_lp();
                        let msgs = c.eng.tick(now);
                        if now.duration_since(c.lp_last_draw) >= Duration::from_millis(30) {
                            c.lp_last_draw = now;
                            c.draw_lp(false);
                        }
                        msgs
                    };
                    if !msgs.is_empty() {
                        let o = lock(&out).clone();
                        if let Some(o) = o {
                            let mut conn = lock(&o);
                            for m in msgs {
                                let _ = conn.send(&m);
                            }
                        }
                    }
                    std::thread::sleep(Duration::from_micros(500));
                }
            });
        }
        Seq { core, out, quit, lp_in: None, lp_name: None, lp_checked: None, saved: song, save_check: Instant::now() }
    }

    /// A channel message from the editor's MIDI input (keyboard notes are recorded when Record is on).
    pub fn midi_in(&self, status: u8, d1: u8, d2: u8) {
        lock(&self.core).midi_in(status, d1, d2);
    }

    pub fn is_playing(&self) -> bool {
        lock(&self.core).eng.playing
    }

    /// Tells the engine which output the editor is connected to (call every frame).
    pub fn set_out(&self, o: Option<Out>) {
        let mut cur = lock(&self.out);
        let same = match (&*cur, &o) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        };
        if !same {
            *cur = o;
        }
    }

    /// Connects a Launchpad Pro MK3 when one appears (checked every 2 s), and saves changes. Call every frame.
    pub fn tick(&mut self, ctx: &egui::Context) {
        if self.lp_checked.is_none_or(|t| t.elapsed() > Duration::from_secs(2)) {
            self.lp_checked = Some(Instant::now());
            self.lp_watch(ctx);
        }
        if self.save_check.elapsed() > Duration::from_secs(2) {
            self.save_check = Instant::now();
            self.autosave();
        }
        if self.is_playing() || self.lp_in.is_some() {
            ctx.request_repaint_after(Duration::from_millis(40));
        }
    }

    fn autosave(&mut self) {
        let (song, slots) = {
            let mut c = lock(&self.core);
            let slots = if c.slots_dirty {
                c.slots_dirty = false;
                Some(c.slots.clone())
            } else {
                None
            };
            (c.eng.song.clone(), slots)
        };
        let Some(dir) = seq_dir() else { return };
        let _ = std::fs::create_dir_all(&dir);
        if song != self.saved {
            if let Ok(s) = serde_json::to_string(&song) {
                let _ = std::fs::write(dir.join("seq_current.json"), s);
            }
            self.saved = song;
        }
        if let Some(slots) = slots {
            if let Ok(s) = serde_json::to_string(&slots) {
                let _ = std::fs::write(dir.join("seq_projects.json"), s);
            }
        }
    }

    fn lp_watch(&mut self, ctx: &egui::Context) {
        let outs: Vec<String> = MidiOutput::new("novaedit-lp-list").map(|m| m.ports().iter().filter_map(|p| m.port_name(p).ok()).collect()).unwrap_or_default();
        let ins: Vec<String> = MidiInput::new("novaedit-lp-list").map(|m| m.ports().iter().filter_map(|p| m.port_name(p).ok()).collect()).unwrap_or_default();
        if let Some(name) = &self.lp_name {
            if !outs.iter().any(|n| n == name) {
                // unplugged
                self.lp_in = None;
                self.lp_name = None;
                let mut c = lock(&self.core);
                c.lp_out = None;
                c.lp_rx = None;
                c.status("Launchpad disconnected");
            }
            return;
        }
        let (Some(oi), Some(ii)) = (outs.iter().position(|n| is_lp_port(n)), ins.iter().position(|n| is_lp_port(n))) else { return };
        let Ok(mo) = MidiOutput::new("novaedit-lp-out") else { return };
        let Some(port) = mo.ports().get(oi).cloned() else { return };
        let Ok(mut oc) = mo.connect(&port, "novaedit-lp-out") else { return };
        let Ok(mut mi) = MidiInput::new("novaedit-lp-in") else { return };
        mi.ignore(midir::Ignore::All);
        let Some(iport) = mi.ports().get(ii).cloned() else { return };
        let (tx, rx) = channel::<Vec<u8>>();
        let ctx2 = ctx.clone();
        let Ok(ic) = mi.connect(
            &iport,
            "novaedit-lp-in",
            move |_t, data, _| {
                let _ = tx.send(data.to_vec());
                ctx2.request_repaint();
            },
            (),
        ) else {
            return;
        };
        let _ = oc.send(&LP_PROGRAMMER);
        let mut c = lock(&self.core);
        c.lp_out = Some(oc);
        c.lp_rx = Some(rx);
        c.lp_leds = [None; 128];
        c.draw_lp(true);
        c.status("Launchpad Pro MK3 connected (Programmer mode)");
        drop(c);
        self.lp_in = Some(ic);
        self.lp_name = Some(outs[oi].clone());
    }

    /// Draws the sequencer page: the 8 x 8 grid with the track row and mute / solo under it, and every other
    /// control in the side panel. Returns true when the editor page should be shown again.
    pub fn ui(&mut self, ui: &mut Ui, out_name: Option<&str>) -> bool {
        let avail = ui.available_rect_before_wrap();
        ui.painter().rect_filled(avail, 0.0, Color32::from_rgb(0x12, 0x13, 0x16));
        let side_w = (avail.width() * 0.32).clamp(280.0, 440.0);
        let area = Rect::from_min_max(avail.min + vec2(16.0, 16.0), Pos2::new(avail.max.x - side_w - 8.0, avail.max.y - 16.0));
        // 8 grid rows, then a gap, the track row (0.55 of a pad) and the mute / solo row (0.4)
        let cell = (area.height() / 9.4).min(area.width() / 8.0);
        let origin = Pos2::new(area.center().x - cell * 4.0, area.min.y + (area.height() - cell * 9.4).max(0.0) / 2.0);
        let mut clicks: Vec<(Pos, bool)> = vec![];
        let mut mute_solo: Vec<(usize, bool)> = vec![];
        {
            let c = lock(&self.core);
            let p = ui.painter().clone();
            let font = FontId::proportional((cell * 0.2).clamp(10.0, 16.0));
            let pad = |r: f32, col: usize, h: f32| Rect::from_min_size(origin + vec2(col as f32 * cell + cell * 0.05, r * cell + cell * 0.05), vec2(cell * 0.9, h));
            for r in 0..10i8 {
                for col in 0..8i8 {
                    let p_ = (r, col);
                    let rect = match r {
                        0..=7 => pad(r as f32, col as usize, cell * 0.9),
                        8 => pad(8.25, col as usize, cell * 0.5),
                        _ => continue,
                    };
                    let resp = ui.interact(rect, ui.id().with(("seqpad", r, col)), Sense::click());
                    let mut fill = c.lit(p_).color();
                    if resp.hovered() {
                        fill = fill.gamma_multiply(1.15);
                    }
                    p.rect_filled(rect, cell * 0.07, fill);
                    p.rect_stroke(rect, cell * 0.07, Stroke::new(1.0, Color32::from_black_alpha(160)), egui::StrokeKind::Inside);
                    let mut text = String::new();
                    if r == 8 {
                        text = if c.pg.projects { String::new() } else { format!("{}", col + 1) };
                    } else if (r as usize * 8 + col as usize) < SLOTS && c.pg.projects {
                        text = format!("PR{:02}", r as usize * 8 + col as usize + 1);
                    } else if r >= 4 && !c.pg.patterns && !c.pg.projects {
                        let tr = &c.eng.song.tracks[c.pg.sel];
                        let note = (tr.base as usize + (7 - r as usize) * 8 + col as usize).min(127) as u8;
                        let held = c.pg.held.is_some_and(|h| tr.pat().steps[h as usize].notes.contains(&note));
                        if note % 12 == 0 || held {
                            text = note_name(note);
                        }
                    } else if c.pg.patterns && r < 2 && !DEBUG_STEP_IDS {
                        text = (r as usize * 8 + col as usize + 1).to_string();
                    }
                    if !text.is_empty() {
                        let [r_, g, b, _] = fill.to_array();
                        let tc = if r_ as u32 + g as u32 + b as u32 > 420 { Color32::BLACK } else { Color32::from_rgb(0xe0, 0xe2, 0xe8) };
                        p.text(rect.center(), Align2::CENTER_CENTER, text, font.clone(), tc);
                    }
                    // DEBUG: project / pattern / step id on each step pad
                    if DEBUG_STEP_IDS && r < 4 && !c.pg.patterns && !c.pg.projects {
                        let tr = &c.eng.song.tracks[c.pg.sel];
                        let step = c.pg.page as usize * 32 + r as usize * 8 + col as usize;
                        let id = format!("PR{:02}\nPN{:02}\nST{:02}\nTR{:02}", c.pg.slot + 1, tr.pattern as usize + 1, step + 1, c.pg.sel + 1);
                        // four lines of 4 chars, as big as the pad allows
                        let size = (rect.width() * 0.85 / (4.0 * 0.62)).min(rect.height() * 0.88 / 4.7);
                        let [r_, g, b, _] = fill.to_array();
                        let tc = if r_ as u32 + g as u32 + b as u32 > 420 { Color32::BLACK } else { Color32::from_rgb(0xd0, 0xd2, 0xd8) };
                        p.text(rect.center(), Align2::CENTER_CENTER, id, FontId::monospace(size), tc);
                    }
                    // DEBUG: project / pattern id on each pattern pad (row = pattern, column = track)
                    if DEBUG_STEP_IDS && r < 2 && c.pg.patterns {
                        let id = format!("PR{:02}\nPN{:02}", c.pg.slot + 1, r as usize * 8 + col as usize + 1);
                        let size = (rect.width() * 0.85 / (4.0 * 0.62)).min(rect.height() * 0.85 / 3.6);
                        let [r_, g, b, _] = fill.to_array();
                        let tc = if r_ as u32 + g as u32 + b as u32 > 420 { Color32::BLACK } else { Color32::from_rgb(0xd0, 0xd2, 0xd8) };
                        p.text(rect.center(), Align2::CENTER_CENTER, id, FontId::monospace(size), tc);
                    }
                    if resp.clicked() {
                        clicks.push((p_, false));
                    } else if resp.secondary_clicked() || resp.long_touched() {
                        clicks.push((p_, true));
                    }
                }
            }
            // mute / solo under each track
            for t in 0..TRACKS {
                let tr = &c.eng.song.tracks[t];
                for (k, (on, txt, col)) in [(tr.muted, "M", Color32::from_rgb(0xc8, 0xa8, 0x20)), (tr.solo, "S", Color32::from_rgb(0x3c, 0x6e, 0xff))].into_iter().enumerate() {
                    let w = cell * 0.43;
                    let rect = Rect::from_min_size(origin + vec2(t as f32 * cell + cell * 0.05 + k as f32 * (w + cell * 0.04), 8.95 * cell), vec2(w, cell * 0.38));
                    let resp = ui.interact(rect, ui.id().with(("seqms", t, k)), Sense::click());
                    let fill = if on { col } else if resp.hovered() { Color32::from_rgb(0x34, 0x36, 0x3c) } else { Color32::from_rgb(0x26, 0x27, 0x2b) };
                    p.rect_filled(rect, 4.0, fill);
                    p.rect_stroke(rect, 4.0, Stroke::new(1.0, Color32::from_rgb(0x4a, 0x4c, 0x55)), egui::StrokeKind::Inside);
                    p.text(rect.center(), Align2::CENTER_CENTER, txt, font.clone(), if on { Color32::BLACK } else { Color32::from_rgb(0xb0, 0xb2, 0xba) });
                    if resp.clicked() {
                        mute_solo.push((t, k == 1));
                    }
                }
            }
        }
        {
            let mut c = lock(&self.core);
            for (p_, alt) in clicks {
                c.press(p_, false, alt);
            }
            for (t, solo) in mute_solo {
                let tr = &mut c.eng.song.tracks[t];
                if solo { tr.solo = !tr.solo } else { tr.muted = !tr.muted }
            }
        }
        let side = Rect::from_min_max(Pos2::new(avail.max.x - side_w, avail.min.y + 12.0), avail.max - vec2(14.0, 8.0));
        let mut back = false;
        ui.scope_builder(egui::UiBuilder::new().max_rect(side), |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                back = self.side_ui(ui, out_name);
            });
        });
        let mut c = lock(&self.core);
        back | std::mem::take(&mut c.pg.go_editor)
    }

    fn side_ui(&mut self, ui: &mut Ui, out_name: Option<&str>) -> bool {
        /// A row of choices; returns the one clicked.
        fn seg(ui: &mut Ui, opts: &[&str], cur: usize) -> Option<usize> {
            let mut hit = None;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                let w = (ui.available_width() - 4.0 * (opts.len() as f32 - 1.0)) / opts.len() as f32;
                for (i, o) in opts.iter().enumerate() {
                    let on = i == cur;
                    let b = egui::Button::new(egui::RichText::new(*o).size(15.0).color(if on { Color32::WHITE } else { Color32::from_rgb(0xc8, 0xca, 0xd2) }))
                        .fill(if on { Color32::from_rgb(0x3a, 0x5b, 0xd9) } else { Color32::from_rgb(0x26, 0x27, 0x2b) });
                    if ui.add_sized([w, 36.0], b).clicked() {
                        hit = Some(i);
                    }
                }
            });
            hit
        }
        let label = |ui: &mut Ui, t: &str| {
            ui.add_space(6.0);
            ui.label(egui::RichText::new(t).size(13.0).color(Color32::from_rgb(0x8a, 0x8c, 0x95)));
        };
        let mut back = false;
        let ed = Color32::from_rgb(0x2d, 0x4a, 0xa8);
        if icon_button(ui, [ui.available_width(), 44.0], "Editor", Some(Icon::Left), 20.0, Color32::WHITE, ed, Stroke::new(1.0, ed)).clicked() {
            back = true;
        }
        ui.add_space(6.0);
        let mut c = lock(&self.core);
        let playing = c.eng.playing;
        /// One big button of the 3-column grid; lit = filled with `col`.
        fn big_i(ui: &mut Ui, w: f32, text: &str, icon: Option<Icon>, lit: bool, col: Color32) -> bool {
            big_ic(ui, w, text, icon, None, lit, col)
        }
        /// `icon_col`: symbol colour while not lit (lit buttons show it white on the fill).
        fn big_ic(ui: &mut Ui, w: f32, text: &str, icon: Option<Icon>, icon_col: Option<Color32>, lit: bool, col: Color32) -> bool {
            let fg = if lit { Color32::WHITE } else { Color32::from_rgb(0xd0, 0xd2, 0xda) };
            let fill = if lit { col } else { Color32::from_rgb(0x22, 0x23, 0x28) };
            let stroke = Stroke::new(1.5, if lit { col } else { Color32::from_rgb(0x4e, 0x52, 0x60) });
            let ic = if lit { None } else { icon_col };
            icon_button_c(ui, [w, 50.0], text, icon, ic, 17.0, fg, fill, stroke).clicked()
        }
        fn big(ui: &mut Ui, w: f32, text: &str, lit: bool, col: Color32) -> bool {
            big_i(ui, w, text, None, lit, col)
        }
        let (green, red, blue, amber) = (
            Color32::from_rgb(0x2e, 0x9a, 0x50),
            Color32::from_rgb(0xc8, 0x2a, 0x22),
            Color32::from_rgb(0x3a, 0x5b, 0xd9),
            Color32::from_rgb(0xc0, 0x7a, 0x14),
        );
        let gap = 6.0;
        let w = (ui.available_width() - gap * 2.0) / 3.0;
        let view = if c.pg.patterns { 1 } else if c.pg.projects { 2 } else { 0 };
        ui.spacing_mut().item_spacing = vec2(gap, gap);
        ui.horizontal(|ui| {
            if big_ic(ui, w, if playing { "Stop" } else { "Play" }, Some(if playing { Icon::Stop } else { Icon::Play }), Some(Color32::from_rgb(0x3c, 0xe0, 0x5a)), playing, green) {
                c.press(PLAY, false, false);
            }
            if big_ic(ui, w, "Rec", Some(Icon::Rec), Some(Color32::from_rgb(0xf0, 0x3a, 0x2e)), c.pg.rec, red) {
                c.toggle_record();
            }
            if big(ui, w, "Step Rec", c.pg.step_rec, red) {
                c.toggle_step_rec();
            }
        });
        ui.horizontal(|ui| {
            for (i, name) in ["Steps", "Pattern", "Proj"].into_iter().enumerate() {
                if big(ui, w, name, view == i, blue) && view != i {
                    c.pg.patterns = i == 1;
                    c.pg.projects = i == 2;
                    // Clear / Copy / Length don't carry over to another view
                    c.pg.clear = false;
                    c.pg.dup = false;
                    c.pg.fixed = false;
                    c.pg.copy_src = None;
                    c.pg.shift = false;
                    c.pg.held = None;
                    c.pg.pat_hold = None;
                }
            }
        });
        ui.horizontal(|ui| {
            let cur = if c.pg.clear { 1 } else if c.pg.dup { 2 } else if c.pg.fixed { 3 } else { 0 };
            for (i, name) in [(1, "Clear"), (2, "Copy"), (3, "Length")] {
                if big(ui, w, name, cur == i, amber) {
                    // tap again to go back to plain editing
                    let on = cur != i;
                    c.pg.clear = on && i == 1;
                    c.pg.dup = on && i == 2;
                    c.pg.fixed = on && i == 3;
                    c.pg.copy_src = None;
                }
            }
        });
        ui.horizontal(|ui| {
            let bpm = c.eng.song.bpm;
            ui.scope(|ui| {
                ui.style_mut().override_font_id = Some(FontId::proportional(17.0));
                ui.add_sized([w, 50.0], egui::DragValue::new(&mut c.eng.song.bpm).range(30..=300).prefix("BPM ").speed(0.3));
            });
            let hw = (w - gap) / 2.0;
            if big_i(ui, hw, "", Some(Icon::Left), false, blue) {
                c.eng.song.bpm = bpm.saturating_sub(1).max(30);
            }
            if big_i(ui, hw, "", Some(Icon::Right), false, blue) {
                c.eng.song.bpm = (bpm + 1).min(300);
            }
            if big(ui, w, "Tap Tempo", false, blue) {
                c.tap();
            }
        });
        ui.spacing_mut().item_spacing = vec2(8.0, 3.0);
        match view {
            1 => {
                label(ui, "Tap a pattern to");
                if let Some(v) = seg(ui, &["Switch", "Chain to it"], c.pg.shift as usize) {
                    c.pg.shift = v == 1;
                }
            }
            2 => {
                label(ui, &format!("Project {} open", c.pg.slot + 1));
                ui.weak("Tap a project to switch to it. Your work stays in its own project.");
            }
            _ => {}
        }
        c.eng.song.swing = stepper(ui, "Swing (1/16 tracks)", c.eng.song.swing as i32, 50, 75, &|v| format!("{v}%")) as u8;
        ui.horizontal(|ui| {
            let w = (ui.available_width() - 8.0) / 2.0;
            ui.vertical(|ui| {
                ui.set_width(w);
                label(ui, "Steps");
                let page = c.pg.page as usize;
                if let Some(v) = seg(ui, &["1-32", "33-64"], page) {
                    c.pg.page = v as u8;
                }
            });
            ui.vertical(|ui| {
                ui.set_width(w);
                let base = c.eng.song.tracks[c.pg.sel].base;
                label(ui, &format!("Notes from {}", note_name(base)));
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    let bw = (ui.available_width() - 4.0) / 2.0;
                    let (fg, fill, st) = (Color32::from_rgb(0xc8, 0xca, 0xd2), Color32::from_rgb(0x26, 0x27, 0x2b), Stroke::NONE);
                    if icon_button(ui, [bw, 36.0], "octave", Some(Icon::Up), 15.0, fg, fill, st).clicked() {
                        let t = c.track();
                        t.base = (t.base + 12).min(96);
                    }
                    if icon_button(ui, [bw, 36.0], "", Some(Icon::Down), 15.0, fg, fill, st).clicked() {
                        let t = c.track();
                        t.base = t.base.saturating_sub(12);
                    }
                });
            });
        });

        ui.add_space(8.0);
        ui.separator();
        let sel = c.pg.sel;
        ui.label(egui::RichText::new(format!("Track {}", sel + 1)).size(17.0).strong());
        /// Touch-friendly number control: big -/+ buttons around a slider, with the value shown large.
        fn stepper(ui: &mut Ui, name: &str, v: i32, lo: i32, hi: i32, show: &dyn Fn(i32) -> String) -> i32 {
            let mut out = v;
            ui.add_space(4.0);
            ui.label(egui::RichText::new(name).size(14.0).color(Color32::from_rgb(0x8a, 0x8c, 0x95)));
            ui.horizontal(|ui| {
                let bw = 46.0;
                let pm = |t: &str| egui::Button::new(egui::RichText::new(t).size(22.0).strong()).fill(Color32::from_rgb(0x26, 0x27, 0x2b));
                if ui.add_sized([bw, 40.0], pm("\u{2212}")).clicked() {
                    out = (out - 1).max(lo);
                }
                let sp = ui.spacing().item_spacing.x;
                ui.spacing_mut().slider_width = (ui.available_width() - bw - 72.0 - sp * 3.0).max(60.0);
                ui.spacing_mut().interact_size.y = 40.0;
                let mut f = out;
                ui.add(egui::Slider::new(&mut f, lo..=hi).show_value(false));
                out = f;
                ui.add_sized([72.0, 40.0], egui::Label::new(egui::RichText::new(show(out)).size(18.0).strong()));
                if ui.add_sized([bw, 40.0], pm("+")).clicked() {
                    out = (out + 1).min(hi);
                }
            });
            out
        }
        {
            let t = &mut c.eng.song.tracks[sel];
            t.channel = (stepper(ui, "MIDI channel", t.channel as i32 + 1, 1, 16, &|v| v.to_string()) - 1) as u8;
            ui.add_space(4.0);
            ui.label(egui::RichText::new("Step rate").size(14.0).color(Color32::from_rgb(0x8a, 0x8c, 0x95)));
            let r = t.rate as usize;
            if let Some(i) = seg(ui, &RATE_NAMES[..4], if r < 4 { r } else { 9 }) {
                t.rate = i as u8;
            }
            if let Some(i) = seg(ui, &RATE_NAMES[4..], if r >= 4 { r - 4 } else { 9 }) {
                t.rate = i as u8 + 4;
            }
            t.transpose = stepper(ui, "Transpose", t.transpose as i32, -24, 24, &|v| if v > 0 { format!("+{v}") } else { v.to_string() }) as i8;
            let len = stepper(ui, "Pattern length (steps)", t.pat().length as i32, 1, 64, &|v| v.to_string()) as u8;
            t.pat_mut().length = len;
            ui.add_space(4.0);
            let chain = if t.chain_end > t.chain_start { format!("   chain {}-{}", t.chain_start + 1, t.chain_end + 1) } else { String::new() };
            ui.label(egui::RichText::new(format!("Pattern{chain}")).size(14.0).color(Color32::from_rgb(0x8a, 0x8c, 0x95)));
        }
        let cur_pat = c.eng.song.tracks[sel].pattern as usize;
        for half in 0..2 {
            let names: Vec<String> = (half * 8 + 1..=half * 8 + 8).map(|n| n.to_string()).collect();
            let names: Vec<&str> = names.iter().map(|n| n.as_str()).collect();
            let cur = if cur_pat / 8 == half { cur_pat % 8 } else { 99 };
            if let Some(p) = seg(ui, &names, cur) {
                let shift = c.pg.shift;
                c.pg.shift = false;
                c.pattern_pad((half * 8 + p) as u8);
                c.pg.shift = shift;
                c.pg.pat_hold = None;
            }
        }
        if let Some(h) = c.pg.held {
            ui.add_space(6.0);
            ui.separator();
            let s = &mut c.eng.song.tracks[sel].pat_mut().steps[h as usize];
            let notes = s.notes.iter().map(|&n| note_name(n)).collect::<Vec<_>>().join(" ");
            ui.label(egui::RichText::new(format!("Step {}  \u{00b7}  {notes}", h + 1)).size(17.0).strong());
            ui.weak("Tap note pads to set the chord");
            s.vel = stepper(ui, "Velocity", s.vel as i32, 1, 127, &|v| v.to_string()) as u8;
            s.gate = stepper(ui, "Gate (eighths of a step)", s.gate as i32, 1, 8, &|v| format!("{v}/8")) as u8;
            s.prob = stepper(ui, "Probability", s.prob as i32, 1, 100, &|v| format!("{v}%")) as u8;
            ui.add_space(4.0);
            if ui.add_sized([ui.available_width(), 40.0], egui::Button::new(egui::RichText::new("Done").size(17.0))).clicked() {
                c.pg.held = None;
            }
        }
        ui.add_space(4.0);
        ui.separator();
        if let Some(s) = &c.pg.status {
            ui.label(s);
        }
        ui.checkbox(&mut c.eng.song.send_clock, "Send MIDI clock (start / stop / F8)");
        match out_name {
            Some(n) => ui.weak(format!("Plays to: {n}")),
            None => ui.colored_label(Color32::from_rgb(0xf3, 0xa2, 0x4b), "No MIDI output connected - use the midi button in the editor"),
        };
        match &self.lp_name {
            Some(n) => ui.weak(format!("Launchpad: {n}")),
            None => ui.weak("Launchpad Pro MK3: not connected"),
        };
        ui.collapsing("How to use", |ui| {
            ui.label("Top four rows: steps. Bottom four rows: notes (tap one to hear it and to choose the note new steps get).");
            ui.label("Click a step to turn it on or off. Right-click a step to edit it: tap note pads to set its chord, and use the sliders here for velocity, gate and probability.");
            ui.label("Rec: while playing, notes you play on the note pads (or a keyboard on the MIDI input) are written into the pattern at the nearest step, with their velocity and length. While stopped, it is step recording: each note or chord goes on the white step and the cursor moves on; tap a step to move the cursor.");
            ui.label("Tools: Clear empties the steps you tap (or a track's pattern when you tap its track button). Copy: tap a lit step, then the steps to paste it to. Length: tap the step that should be the last.");
            ui.label("Patterns view: columns are tracks, rows are patterns A-H. While playing, a new pattern starts when the current one ends.");
            ui.label("On the Launchpad, every button works as on the hardware sequencer, with press-and-hold.");
        });
        back
    }
}

impl Drop for Seq {
    fn drop(&mut self) {
        self.autosave();
        let mut c = lock(&self.core);
        c.eng.stop();
        let offs = c.eng.tick(Instant::now() + Duration::from_secs(1));
        if let Some(o) = lock(&self.out).clone() {
            let mut conn = lock(&o);
            for m in offs {
                let _ = conn.send(&m);
            }
        }
        if let Some(out) = c.lp_out.as_mut() {
            let _ = out.send(&LP_LIVE);
        }
        self.quit.store(true, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lp_index_roundtrip() {
        for r in -1..=9i8 {
            for c in -1..=8i8 {
                if let Some(i) = lp_index((r, c)) {
                    assert_eq!(pos_of_index(i), Some((r, c)), "index {i}");
                }
            }
        }
        assert_eq!(lp_index((0, 0)), Some(81));
        assert_eq!(lp_index((7, 7)), Some(18));
        assert_eq!(lp_index(PLAY), Some(20));
        assert_eq!(lp_index(PATTERNS_BTN), Some(89));
        assert_eq!(lp_index(PROJECTS), Some(98));
    }

    #[test]
    fn engine_plays_steps_on_track_channel() {
        let mut song = Song::default();
        song.tracks[2].patterns[0].steps[0] = Step::single(64);
        song.tracks[2].patterns[0].length = 4;
        let mut e = Engine::new(song);
        e.start();
        let t0 = e.next_hp;
        let mut all = vec![];
        for i in 0..400 {
            all.extend(e.tick(t0 + Duration::from_millis(i * 5)));
        }
        let ons: Vec<_> = all.iter().filter(|m| m[0] == 0x92 && m[1] == 64).collect();
        // 2 s at 120 BPM = 16 sixteenths = 4 loops of a 4-step pattern
        assert_eq!(ons.len(), 4);
        assert!(all.iter().any(|m| m[0] == 0x82 && m[1] == 64));
        e.stop();
        assert!(e.playing_step.iter().all(|s| s.is_none()));
    }

    fn core() -> Core {
        Core { eng: Engine::new(Song::default()), pg: Page::default(), slots: vec![None; SLOTS], slots_dirty: false, lp_out: None, lp_rx: None, lp_leds: [None; 128], lp_last_draw: Instant::now() }
    }

    #[test]
    fn patterns_are_song_wide_and_projects_switch() {
        let mut c = core();
        c.pattern_pad(10);
        c.pg.pat_hold = None;
        assert!(c.eng.song.tracks.iter().all(|t| t.pattern == 10));
        c.eng.song.tracks[2].patterns[10].steps[0].on = true;
        c.project_slot(7);
        assert_eq!((c.pg.slot, c.eng.song.project), (7, 7));
        assert_eq!(c.eng.song.tracks[0].pattern, 0, "a new project starts blank");
        assert!(c.slots[0].is_some());
        c.project_slot(0);
        assert_eq!(c.eng.song.tracks[0].pattern, 10);
        assert!(c.eng.song.tracks[2].patterns[10].steps[0].on);
        assert!(c.slots[7].is_some());
    }

    #[test]
    fn step_recording_writes_chords_and_advances() {
        let mut c = core();
        c.press(RECORD, false, false);
        assert!(c.pg.step_rec && c.pg.cursor == 0);
        c.midi_in(0x90, 60, 90);
        c.midi_in(0x90, 64, 80);
        c.midi_in(0x80, 60, 0);
        assert_eq!(c.pg.cursor, 0);
        c.midi_in(0x90, 64, 0);
        assert_eq!(c.pg.cursor, 1);
        c.midi_in(0x90, 67, 100);
        c.midi_in(0x80, 67, 0);
        let p = c.eng.song.tracks[0].pat();
        assert!(p.steps[0].on && p.steps[0].notes == vec![60, 64] && p.steps[0].vel == 90);
        assert!(p.steps[1].on && p.steps[1].notes == vec![67]);
        assert_eq!(c.pg.cursor, 2);
        // tapping a step moves the cursor instead of toggling it
        c.press((0, 5), false, false);
        assert_eq!(c.pg.cursor, 5);
        assert!(!c.eng.song.tracks[0].pat().steps[5].on);
    }

    #[test]
    fn live_recording_lands_on_playing_step() {
        let mut c = core();
        c.eng.start();
        let t0 = c.eng.next_hp;
        c.eng.tick(t0);
        c.press(RECORD, false, false);
        // right at the step: lands on it
        c.midi_in(0x90, 62, 100);
        c.midi_in(0x80, 62, 0);
        let p = c.eng.song.tracks[0].pat();
        assert!(p.steps[0].on && p.steps[0].notes == vec![62]);
        assert_eq!(p.steps[0].gate, 1);
        // the sequencer's own note coming back is ignored
        c.eng.sent.push((0, 70, Instant::now()));
        c.midi_in(0x90, 70, 100);
        assert!(!c.eng.song.tracks[0].pat().steps[0].notes.contains(&70));
    }

    #[test]
    fn pattern_chain_advances() {
        let mut t = Track { chain_start: 1, chain_end: 3, pattern: 1, ..Track::default() };
        t.next_pattern();
        assert_eq!(t.pattern, 2);
        t.next_pattern();
        t.next_pattern();
        assert_eq!(t.pattern, 1);
        t.queued = Some((5, 5));
        t.next_pattern();
        assert_eq!((t.pattern, t.chain_start, t.chain_end), (5, 5, 5));
    }

    #[test]
    fn song_json_roundtrip_normalizes() {
        let s = Song::default();
        let j = serde_json::to_string(&s).unwrap();
        let mut back: Song = serde_json::from_str(&j).unwrap();
        back.normalize();
        assert_eq!(back, s);
        let mut bad: Song = serde_json::from_str(r#"{"bpm": 999, "tracks": []}"#).unwrap();
        bad.normalize();
        assert_eq!(bad.bpm, 300);
        assert_eq!(bad.tracks.len(), TRACKS);
    }
}
