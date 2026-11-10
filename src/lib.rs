mod arp;
mod files;
mod dump_ui;
mod effects;
mod extras_ui;
mod live;
mod menus;
mod mirror;
mod midi;
mod panel;
mod params;
mod perf;
mod slots;
mod sysex;

use eframe::egui::{self, Color32, RichText};
use params::{CATS, N, Param};
use std::collections::BTreeMap;
use std::path::PathBuf;

pub const TOTAL_MID: &[u8] = include_bytes!("../assets/total.mid");

#[derive(PartialEq, Clone, Copy)]
enum Mode {
    Program,
    Performance,
}

#[derive(PartialEq, Clone, Copy)]
enum Tab {
    Panel,
    Bytes,
    Arp,
    Globals,
}

struct AssignWin {
    id: String,
    off: String,
    min: String,
    max: String,
}

pub struct App {
    params: Vec<Param>,
    programs: Vec<sysex::Program>,
    cur: Option<usize>,
    prog: Vec<u8>,
    orig: Vec<u8>,
    label: String,
    bank: u8,
    idx: u8,
    part: u8,
    filter: String,
    tab: Tab,
    pstate: panel::PanelState,
    assign: BTreeMap<String, panel::Assign>,
    info_id: Option<String>,
    readout_msg: Option<String>,
    status: (String, Option<bool>),
    name_buf: String,
    midi: midi::Midi,
    sel_in: Option<usize>,
    sel_out: Option<usize>,
    show_midi: bool,
    /// library changed (since) and not yet saved to the iPad's library file
    lib_dirty: Option<std::time::Instant>,
    show_list: bool,
    midi_manual_off: bool,
    midi_check: std::time::Instant,
    show_dump: bool,
    dump_bank: u8,
    bank_send: Option<dump_ui::BankSend>,
    bank_send_ack: bool,
    confirm_send: bool,
    assign_win: Option<AssignWin>,
    learn: Option<String>,
    last_rx: Option<Vec<u8>>,
    scroll_to_sel: bool,
    stash: Option<Vec<u8>>,
    mode: Mode,
    perfs: Vec<sysex::Performance>,
    perf_cur: Option<usize>,
    perf_body: Vec<u8>,
    perf_orig: Vec<u8>,
    perf_stash: Option<Vec<u8>>,
    perf_bank: u8,
    perf_idx: u8,
    mute_mode: bool,
    solo: Option<usize>,
    /// embedded part programs of a "Nova buffer" performance (working copy / original / compare stash)
    perf_bundle: Option<sysex::Bundle>,
    perf_bundle_orig: Option<sysex::Bundle>,
    bundle_stash: Option<sysex::Bundle>,
    /// program last loaded from / written to the bundle (to detect panel edits)
    embedded_base: Option<Vec<u8>>,
    /// recently received part dumps waiting for a type-5
    rx_parts: [Option<(std::time::Instant, Vec<u8>)>; 8],
    last_bundle: Option<(usize, std::time::Instant)>,
    copy_mode: bool,
    show_fx: bool,
    show_arp: bool,
    /// open menu windows: "osc", "filter", "lfo", "env"
    menus_open: Vec<&'static str>,
    show_trig: bool,
    /// sending is off at every start (not persisted)
    send_enabled: bool,
    send_chan: u8,
    send_job: Option<midi::SendJob>,
    patterns: Vec<sysex::ArpPattern>,
    patterns_orig: Vec<sysex::ArpPattern>,
    pat_cur: Option<usize>,
    pat_filter: String,
    globals: Option<sysex::Globals>,
    globals_orig: Option<sysex::Globals>,
    globals_labels: std::collections::BTreeMap<usize, String>,
    globals_cmp: Option<(String, Vec<u8>)>,
    ports_restored: bool,
    ports_cfg: midi::PortsConfig,
    ports_saved: midi::PortsConfig,
    mirror_on: bool,
    mirror_chan: mirror::ChanMode,
    mirror: mirror::Engine,
    mirror_log: std::collections::VecDeque<String>,
    /// last Bank Select (CC32) value seen per MIDI channel
    rx_bank: [Option<u8>; 16],
    /// live edit: send program edits to the Nova as CC/NRPN (off at every start)
    live_on: bool,
    /// waiting (since) for the Nova's global settings before Live edit starts
    live_wait: Option<std::time::Instant>,
    /// global settings received from the Nova this session
    globals_live: bool,
    live_snap: Vec<u8>,
    live_key: (bool, Option<usize>, bool, Option<usize>),
    live_sel: (bool, Option<usize>),
    live_sent: u64,
    confirm_unlock: bool,
    rx_collect: bool,
    dump_all: Option<DumpAll>,
    rx_prog_n: usize,
    rx_perf_n: usize,
    unlock_ack: bool,
    /// target slot (bank byte, index) for "save part edit as new program"
    new_slot: (u8, u8),
    /// slots written by "save part edit as new program" (exported together with the performance)
    saved_slots: Vec<(u8, u8)>,
    confirm_fill: bool,
    confirm_set_send: bool,
    set_send_ack: bool,
    send_note: Option<String>,
    scan: Option<std::sync::Arc<midi::Scan>>,
    live_log: std::collections::VecDeque<String>,
}

fn recent_path() -> Option<PathBuf> {
    files::config_dir().map(|d| d.join("novaedit").join("recent_files.json"))
}

fn load_recent() -> Vec<PathBuf> {
    recent_path()
        .and_then(|p| std::fs::read(p).ok())
        .and_then(|d| serde_json::from_slice::<Vec<PathBuf>>(&d).ok())
        .unwrap_or_default()
}

fn save_recent(files: &[PathBuf]) {
    if let Some(p) = recent_path() {
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(s) = serde_json::to_string_pretty(files) {
            let _ = std::fs::write(p, s);
        }
    }
}

fn absolute(p: &std::path::Path) -> PathBuf {
    std::fs::canonicalize(p).unwrap_or_else(|_| std::env::current_dir().map(|d| d.join(p)).unwrap_or(p.to_path_buf()))
}

fn config_path() -> Option<PathBuf> {
    files::config_dir().map(|d| d.join("novaedit").join("assignments.json"))
}

/// Toggle compare (A/B). Entering stashes the edited bytes and shows `orig`; leaving restores them.
fn compare_toggle(prog: &mut Vec<u8>, orig: &[u8], stash: &mut Option<Vec<u8>>) {
    match stash.take() {
        Some(edited) => *prog = edited,
        None => {
            *stash = Some(std::mem::replace(prog, orig.to_vec()));
        }
    }
}

fn prog_name(b: &[u8]) -> String {
    b[..16].iter().map(|&c| if (0x20..0x7f).contains(&c) { c as char } else { ' ' }).collect()
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>, cli: Vec<PathBuf>) -> Self {
        cc.egui_ctx.set_visuals(egui::Visuals::dark());
        // iPad: everything outside the panel (list, windows) at 1.5x for touch and reading distance
        #[cfg(target_os = "ios")]
        cc.egui_ctx.set_zoom_factor(1.5);
        let assign = config_path()
            .and_then(|p| std::fs::read(p).ok())
            .and_then(|d| serde_json::from_slice(&d).ok())
            .unwrap_or_default();
        let mut midi = midi::Midi::new();
        midi.refresh(); // only enumerates ports; nothing is sent
        let mut app = App {
            params: params::load(),
            programs: Vec::new(),
            cur: None,
            prog: vec![0; N],
            orig: vec![0; N],
            label: String::new(),
            bank: 5,
            idx: 0,
            part: 9,
            filter: String::new(),
            tab: Tab::Panel,
            pstate: Default::default(),
            assign,
            info_id: None,
            readout_msg: None,
            status: ("Offline - editing files only".into(), None),
            name_buf: String::new(),
            midi,
            sel_in: None,
            sel_out: None,
            show_midi: false,
            lib_dirty: None,
            show_list: false,
            midi_manual_off: false,
            midi_check: std::time::Instant::now() - std::time::Duration::from_secs(10),
            show_dump: false,
            dump_bank: 7,
            bank_send: None,
            bank_send_ack: false,
            confirm_send: false,
            assign_win: None,
            learn: None,
            last_rx: None,
            scroll_to_sel: false,
            stash: None,
            mode: Mode::Program,
            perfs: Vec::new(),
            perf_cur: None,
            perf_body: vec![0; perf::PERF_LEN],
            perf_orig: vec![0; perf::PERF_LEN],
            perf_stash: None,
            perf_bank: 1,
            perf_idx: 0,
            mute_mode: false,
            solo: None,
            perf_bundle: None,
            perf_bundle_orig: None,
            bundle_stash: None,
            embedded_base: None,
            rx_parts: Default::default(),
            last_bundle: None,
            copy_mode: false,
            show_fx: false,
            show_arp: false,
            menus_open: Vec::new(),
            show_trig: false,
            send_enabled: true,
            send_chan: 0x0f,
            send_job: None,
            patterns: Vec::new(),
            patterns_orig: Vec::new(),
            pat_cur: None,
            pat_filter: String::new(),
            globals: None,
            globals_orig: None,
            globals_labels: extras_ui::load_labels(),
            globals_cmp: None,
            ports_restored: false,
            ports_cfg: midi::PortsConfig::default(),
            ports_saved: midi::PortsConfig::default(),
            mirror_on: false,
            mirror_chan: mirror::ChanMode::Any,
            mirror: Default::default(),
            mirror_log: Default::default(),
            rx_bank: [None; 16],
            live_on: false,
            live_wait: None,
            globals_live: false,
            live_snap: Vec::new(),
            live_key: (false, None, false, None),
            live_sel: (false, None),
            live_sent: 0,
            confirm_unlock: false,
            rx_collect: false,
            dump_all: None,
            rx_prog_n: 0,
            rx_perf_n: 0,
            unlock_ack: false,
            new_slot: (6, 127),
            saved_slots: Vec::new(),
            confirm_fill: false,
            confirm_set_send: false,
            set_send_ack: false,
            send_note: None,
            scan: None,
            live_log: Default::default(),
        };
        app.ports_cfg = midi::PortsConfig::load();
        app.ports_saved = app.ports_cfg.clone();
        app.mirror_on = app.ports_cfg.mirror_on;
        app.mirror_chan = app.ports_cfg.mirror_chan;
        if app.ports_cfg.input.is_some() {
            app.ports_restored = true;
            app.sel_in = midi::select_by_name(app.ports_cfg.input.as_deref(), &app.midi.inputs);
            app.sel_out = midi::select_by_name(app.ports_cfg.output.as_deref(), &app.midi.outputs);
            if app.sel_in.is_some() {
                match app.midi.connect(app.sel_in, app.sel_out, cc.egui_ctx.clone()) {
                    Ok(s) => {
                        app.status = (format!("Auto-connected. {s}"), Some(true));
                        if app.midi.has_output() {
                            let _ = app.send_dump_request(0x10);
                            app.live_wait = Some(std::time::Instant::now());
                        }
                    }
                    Err(e) => app.status = (format!("MIDI auto-connect failed: {e}"), Some(false)),
                }
            }
        }
        (app.programs, app.perfs) = sysex::parse_all(TOTAL_MID);
        let (pats, glob) = sysex::parse_extras_file(TOTAL_MID);
        app.set_extras(pats, glob);
        if !app.programs.is_empty() {
            app.select(0);
            app.status = (format!("Loaded {} factory programs", app.programs.len()), Some(true));
        }
        let files: Vec<PathBuf> = if let Some(lib) = files::library_path().filter(|p| p.exists()) {
            vec![lib]
        } else if cli.is_empty() {
            load_recent().into_iter().filter(|p| p.exists()).collect()
        } else {
            cli.iter().map(|p| absolute(p)).collect()
        };
        app.load_files(&files);
        if let (Some(b), Some(i)) = (app.ports_cfg.last_bank, app.ports_cfg.last_idx) {
            if let Some(k) = app.find_program(b, i) {
                app.select(k);
            }
        }
        app
    }

    /// Re-enumerate ports, keeping the selection by port name.
    /// Every 2 s: notice a lost interface, and (re)connect the remembered ports when they appear.
    fn midi_watch(&mut self, ctx: &egui::Context) {
        ctx.request_repaint_after(std::time::Duration::from_secs(2));
        let now = std::time::Instant::now();
        if now.duration_since(self.midi_check) < std::time::Duration::from_secs(2) {
            return;
        }
        self.midi_check = now;
        self.refresh_ports();
        let gone = self.midi.conn_in.as_ref().is_some_and(|n| !self.midi.inputs.contains(n))
            || self.midi.conn_out.as_ref().is_some_and(|n| !self.midi.outputs.contains(n));
        if gone {
            self.midi.disconnect();
            self.live_on = false;
            self.set_status("MIDI interface disconnected", Some(false));
        }
        if self.midi.conn_in.is_none() && !self.midi_manual_off && self.ports_cfg.input.is_some() {
            let si = midi::select_by_name(self.ports_cfg.input.as_deref(), &self.midi.inputs);
            let so = midi::select_by_name(self.ports_cfg.output.as_deref(), &self.midi.outputs);
            if si.is_some() {
                if let Ok(s) = self.midi.connect(si, so, ctx.clone()) {
                    self.sel_in = si;
                    self.sel_out = so;
                    self.set_status(format!("Auto-connected. {s}"), Some(true));
                    self.start_live();
                }
            }
        }
    }

    fn refresh_ports(&mut self) {
        let old_in = self.sel_in.and_then(|i| self.midi.inputs.get(i).cloned());
        let old_out = self.sel_out.and_then(|i| self.midi.outputs.get(i).cloned());
        self.midi.refresh();
        self.sel_in = midi::select_by_name(old_in.as_deref(), &self.midi.inputs);
        self.sel_out = midi::select_by_name(old_out.as_deref(), &self.midi.outputs);
    }

    /// Open the MIDI window without resetting an existing selection; the first time in a session
    /// pre-select the last successfully connected ports.
    fn open_midi_window(&mut self) {
        self.refresh_ports();
        if !self.ports_restored {
            self.ports_restored = true;
            let (i, o) = (self.ports_cfg.input.clone(), self.ports_cfg.output.clone());
            if self.sel_in.is_none() {
                self.sel_in = midi::select_by_name(i.as_deref(), &self.midi.inputs);
            }
            if self.sel_out.is_none() {
                self.sel_out = midi::select_by_name(o.as_deref(), &self.midi.outputs);
            }
        }
        self.show_midi = true;
    }

    fn set_status(&mut self, s: impl Into<String>, ok: Option<bool>) {
        self.status = (s.into(), ok);
    }

    /// The edited bytes even while compare shows the original.
    fn edited_bytes(&self) -> Vec<u8> {
        self.stash.clone().unwrap_or_else(|| self.prog.clone())
    }

    fn edited_perf(&self) -> Vec<u8> {
        self.perf_stash.clone().unwrap_or_else(|| self.perf_body.clone())
    }

    fn find_program(&self, bank: u8, idx: u8) -> Option<usize> {
        self.programs.iter().position(|p| p.bank == Some(bank) && p.idx == idx)
    }

    /// Parts shown as muted: the stored mask, or everything except the soloed part.
    fn display_muted(&self) -> u8 {
        match self.solo {
            Some(s) => 0x3f & !(1u8 << s),
            None => perf::mute_mask(&self.perf_body),
        }
    }

    fn select_perf(&mut self, i: usize) {
        self.solo = None;
        let p = &self.perfs[i];
        self.perf_body = p.body.clone();
        self.perf_orig = p.body.clone();
        self.perf_bank = p.bank;
        self.perf_idx = p.idx;
        self.perf_bundle = p.bundle.clone();
        self.perf_bundle_orig = p.bundle.clone();
        self.bundle_stash = None;
        self.perf_stash = None;
        self.perf_cur = Some(i);
        self.scroll_to_sel = true;
        // as on the Nova: a newly chosen performance shows the performance itself (Part buttons dark,
        // Bank/Prog step performances); Part 1's program is loaded so the panel has something to show
        self.select_part(0);
        self.pstate.part = None;
        let id = self.perf_id_label(self.perf_bank, self.perf_idx, self.perf_bundle.is_some());
        let name = prog_name(&self.perf_body).trim().to_string();
        self.set_status(format!("Performance {id} {name}. Press a Part button to edit that Part."), Some(true));
    }

    /// (id text, program name) of the program used by a part: embedded in a bundle, else looked up.
    fn part_prog_info(&self, part: usize) -> (String, String) {
        if let Some(p) = self.perf_bundle.as_ref().and_then(|b| b.parts[part].as_ref()) {
            return ("embedded".into(), prog_name(p).trim().to_string());
        }
        let b = perf::part_off(part);
        let (num, bank) = (self.perf_body[b + perf::rel::PROG_NUM], self.perf_body[b + perf::rel::PROG_BANK]);
        let name = self
            .find_program(bank, num)
            .map(|i| self.programs[i].name())
            .unwrap_or_else(|| "(not found)".into());
        (perf::prog_id(bank, num), name)
    }

    /// Select a part (as the Part button does): load its program and report it.
    /// Put `body` into program slot (bank, idx) of the app's list and remember it for export.
    /// iPadOS: write the library to its file a few seconds after the last change.
    fn library_tick(&mut self) {
        let Some(path) = files::library_path() else { return };
        let Some(t) = self.lib_dirty else { return };
        if t.elapsed() < std::time::Duration::from_secs(3) {
            return;
        }
        self.lib_dirty = None;
        let mut msgs = self.lib_program_msgs(None);
        msgs.extend(self.lib_perf_msgs());
        if !msgs.is_empty() {
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let _ = std::fs::write(&path, msgs.concat());
        }
    }

    fn store_program(&mut self, bank: u8, idx: u8, body: Vec<u8>) {
        self.lib_dirty = Some(std::time::Instant::now());
        match self.find_program(bank, idx) {
            Some(i) => self.programs[i].body = body,
            None => {
                let label = format!("Prog {}", perf::prog_id(bank, idx));
                self.programs.push(sysex::Program { body, bank: Some(bank), idx, label });
            }
        }
        if !self.saved_slots.contains(&(bank, idx)) {
            self.saved_slots.push((bank, idx));
        }
    }

    /// Key of the open stored performance (None for Nova-buffer performances).
    fn perf_slot_key(&self) -> Option<String> {
        (self.is_perf() && self.perf_bundle.is_none() && self.perf_cur.is_some()).then(|| format!("{}:{}", self.perf_bank, self.perf_idx))
    }

    /// Private slot of this performance's Part under the fixed rule (performance A000-A041 only).
    fn private_slot(&self, part: usize) -> Result<(u8, u8), String> {
        self.perf_slot_key().ok_or("this performance has no stored identity")?;
        let block = slots::block_for_perf(self.perf_bank, self.perf_idx)
            .ok_or("the fixed rule covers performances A000-A041 only; use the manual slot picker")?;
        Ok(slots::slot(block, part))
    }

    /// Copy each of the six Parts' programs into this performance's private slots and repoint the Parts.
    fn make_perf_independent(&mut self) {
        let sel = self.pstate.part;
        let mut done = 0;
        for part in 0..slots::PARTS {
            let b = perf::part_off(part);
            let (bank, num) = (self.perf_body[b + perf::rel::PROG_BANK], self.perf_body[b + perf::rel::PROG_NUM]);
            // the selected Part may carry unsaved edits
            let body = if sel == Some(part) && self.prog != self.orig {
                Some(self.prog.clone())
            } else {
                self.find_program(bank, num).map(|i| self.programs[i].body.clone())
            };
            let Some(body) = body else { continue };
            let (nb, ni) = match self.private_slot(part) {
                Ok(x) => x,
                Err(e) => {
                    self.set_status(format!("Cannot make independent: {e}"), Some(false));
                    return;
                }
            };
            self.store_program(nb, ni, body);
            self.perf_body[b + perf::rel::PROG_BANK] = nb;
            self.perf_body[b + perf::rel::PROG_NUM] = ni;
            done += 1;
        }
        if let Some(p) = sel {
            self.select_part(p);
        }
        self.set_status(
            format!("{done} Part programs copied to this performance's private slots in banks C/D. Export .syx saves them with the performance."),
            Some(true),
        );
    }

    /// Copy performance A000 into A001-A041 (in the app's list) and give each its private C/D block.
    fn fill_performances_from_first(&mut self) {
        let Some(src) = self.perfs.iter().position(|p| p.bank == 1 && p.idx == 0 && p.bundle.is_none()) else {
            self.set_status("No stored performance A000 to copy", Some(false));
            return;
        };
        let template = if self.perf_cur == Some(src) { self.perf_body.clone() } else { self.perfs[src].body.clone() };
        let base_name: String = template[..16].iter().map(|&b| if (0x20..0x7f).contains(&b) { b as char } else { ' ' }).collect::<String>().trim().chars().take(12).collect();
        let sel_part = self.pstate.part;
        // source programs of the six Parts (taken before anything is overwritten)
        let mut srcs: Vec<Option<Vec<u8>>> = Vec::new();
        for part in 0..slots::PARTS {
            let b = perf::part_off(part);
            let (bank, num) = (template[b + perf::rel::PROG_BANK], template[b + perf::rel::PROG_NUM]);
            let body = if sel_part == Some(part) && self.perf_cur == Some(src) && self.prog != self.orig {
                Some(self.prog.clone())
            } else {
                self.find_program(bank, num).map(|i| self.programs[i].body.clone())
            };
            srcs.push(body);
        }
        if srcs.iter().all(|s| s.is_none()) {
            self.set_status("Cannot copy: the Parts' programs are not in the program list", Some(false));
            return;
        }
        for k in 0..slots::BLOCKS {
            let mut body = template.clone();
            if k > 0 {
                let name = format!("{base_name} {:02}", k + 1);
                for (i, c) in name.chars().chain(std::iter::repeat(' ')).take(16).enumerate() {
                    body[i] = c as u8;
                }
            }
            for part in 0..perf::PARTS {
                perf::flag4_set(&mut body, part, false); // vocoder switch off in the copies
            }
            for part in 0..slots::PARTS {
                let Some(pb) = srcs[part].clone() else { continue };
                let (nb, ni) = slots::slot(k, part);
                self.store_program(nb, ni, pb);
                let b = perf::part_off(part);
                body[b + perf::rel::PROG_BANK] = nb;
                body[b + perf::rel::PROG_NUM] = ni;
            }
            match self.perfs.iter().position(|p| p.bank == 1 && p.idx == k as u8 && p.bundle.is_none()) {
                Some(i) => self.perfs[i].body = body,
                None => self.perfs.push(sysex::Performance { body, bank: 1, idx: k as u8, bundle: None }),
            }
        }
        if let Some(c) = self.perf_cur {
            if self.perfs.get(c).is_some_and(|p| p.bundle.is_none()) {
                self.select_perf(c);
            }
        }
        self.set_status(
            format!("Performances A000-A041 now copy A000, each with its own programs in C/D ({} program slots). Export the set to keep it.", slots::BLOCKS * slots::PARTS),
            Some(true),
        );
    }

    /// Messages for the performance set: C/D programs first, then performances A000-A041, each
    /// followed by its companion (the Nova rejects a performance without it).
    fn set_send_messages(&self) -> Vec<Vec<u8>> {
        let chan = self.send_chan;
        let mut msgs = Vec::new();
        for &(bank, idx) in &self.saved_slots {
            if let Some(i) = self.find_program(bank, idx) {
                msgs.push(sysex::build_type2(&self.programs[i].body, bank, idx, chan));
            }
        }
        for p in self.perfs.iter().filter(|p| p.bank == 1 && (p.idx as usize) < slots::BLOCKS && p.bundle.is_none()) {
            msgs.push(sysex::build_type7(&p.body, p.bank, p.idx, chan));
            msgs.push(sysex::build_perf_companion(p.bank, p.idx, chan));
        }
        msgs
    }

    /// One .syx with every C/D program saved so far, then performances A000-A041 with companions.
    fn export_filled_set(&mut self) {
        let Some(path) = files::FileDialog::new().set_file_name("nova_perf_set.syx").add_filter("SysEx", &["syx"]).save_file() else { return };
        let n = self.saved_slots.iter().filter(|&&(b, i)| self.find_program(b, i).is_some()).count();
        let data: Vec<u8> = self.set_send_messages().concat();
        match std::fs::write(&path, &data) {
            Ok(_) => self.set_status(format!("Saved {} ({} programs + performances with companions, {} bytes, SysEx channel {})", path.display(), n, data.len(), self.send_chan), Some(true)),
            Err(e) => self.set_status(format!("Save failed: {e}"), Some(false)),
        }
    }

    fn select_part(&mut self, part: usize) {
        self.pstate.part = Some(part);
        self.load_part_program(part);
        let (id, name) = self.part_prog_info(part);
        if name != "(not found)" {
            let ch = perf::midi_channel_label(self.perf_body[perf::part_off(part) + perf::rel::CHAN]);
            self.set_status(format!("Part {}: {id} {name}, {ch}", part + 1), Some(true));
        }
    }

    /// Load the program used by a part into the front-panel edit buffer.
    fn load_part_program(&mut self, part: usize) {
        if let Some(p) = self.perf_bundle.as_ref().and_then(|b| b.parts[part].clone()) {
            self.orig = self
                .perf_bundle_orig
                .as_ref()
                .and_then(|b| b.parts[part].clone())
                .unwrap_or_else(|| p.clone());
            self.prog = p.clone();
            self.embedded_base = Some(p);
            self.stash = None;
            self.cur = None;
            self.label = "embedded in performance".into();
            return;
        }
        self.embedded_base = None;
        let b = perf::part_off(part);
        let (num, bank) = (self.perf_body[b + perf::rel::PROG_NUM], self.perf_body[b + perf::rel::PROG_BANK]);
        match self.find_program(bank, num) {
            Some(i) => self.select(i),
            None => self.set_status(
                format!("Part {}: program {} not in the program list", part + 1, perf::prog_id(bank, num)),
                Some(false),
            ),
        }
    }

    /// Write front-panel edits of an embedded program back into the bundle.
    fn sync_embedded(&mut self) {
        if !self.is_perf() || self.stash.is_some() || self.perf_stash.is_some() {
            return;
        }
        let (Some(part), Some(base)) = (self.pstate.part, self.embedded_base.as_ref()) else { return };
        if &self.prog != base {
            if let Some(slot) = self.perf_bundle.as_mut().and_then(|b| b.parts.get_mut(part)) {
                *slot = Some(self.prog.clone());
                self.embedded_base = Some(self.prog.clone());
            }
        }
    }

    fn perf_id_label(&self, perf_bank: u8, idx: u8, bundle: bool) -> String {
        if bundle { "buf".into() } else { perf::perf_id(perf_bank, idx) }
    }

    /// Effects live in the performance part block when "Effects used" = Part.
    fn update_fx_part(&mut self) {
        self.pstate.fx_part = if self.is_perf() {
            self.pstate.part.filter(|p| !perf::effects_used_get(&self.perf_body, *p))
        } else {
            None
        };
    }

    /// Effects copy (x_copy + effect button): program effect bytes into the part's own effects area.
    fn do_copy(&mut self, fx: &str) {
        self.copy_mode = false;
        let (Some(g), Some(p)) = (effects::group(fx), self.pstate.part) else {
            self.set_status("select a Part first", Some(false));
            return;
        };
        if g.program_only {
            self.set_status("vocoder balance is stored in the program", Some(false));
        } else if self.perf_stash.is_some() {
            self.set_status("leave compare before copying", Some(false));
        } else if self.part_prog_info(p).1 == "(not found)" {
            self.set_status("the part's program was not found", Some(false));
        } else {
            let prog = self.prog.clone();
            effects::copy_effect(&mut self.perf_body, p, &prog, g);
            self.set_status(
                format!("Copied {} effect from the program into Part {}; Effects used set to Part", g.title, p + 1),
                Some(true),
            );
        }
    }

    fn set_mode(&mut self, m: Mode) {
        if m == Mode::Program {
            self.copy_mode = false;
        }
        if m == Mode::Performance && self.perfs.is_empty() {
            self.set_status("No performances loaded (open a file containing performance dumps)", Some(false));
            return;
        }
        self.mode = m;
        self.pstate.perf_mode = m == Mode::Performance;
        self.scroll_to_sel = true;
        if m == Mode::Performance {
            if self.perf_cur.is_none() {
                self.select_perf(0);
            } else if let Some(p) = self.pstate.part {
                self.select_part(p); // the program buffer may hold something else
            }
        }
    }

    /// The two 16-character display lines, laid out like the Nova: name on top, "Prog A000" /
    /// "Perf A000" right-aligned underneath. A selected Part shows its program, as on the hardware.
    fn lcd_lines(&self) -> (String, String) {
        let top = |b: &[u8]| prog_name(b).chars().take(16).collect::<String>();
        let right = |t: String| format!("{:>16}", t);
        match self.mode {
            Mode::Program => {
                let id = self
                    .cur
                    .map(|c| {
                        let p = &self.programs[c];
                        p.bank.map(|b| perf::prog_id(b, p.idx)).unwrap_or(format!("{:03}", p.idx))
                    })
                    .unwrap_or("----".into());
                (top(&self.prog), right(format!("Prog {id}")))
            }
            Mode::Performance => match self.pstate.part {
                Some(part) => {
                    let b = perf::part_off(part);
                    let id = perf::prog_id(self.perf_body[b + perf::rel::PROG_BANK], self.perf_body[b + perf::rel::PROG_NUM]);
                    let (_, nm) = self.part_prog_info(part);
                    let muted = part < 6 && self.display_muted() & (1 << part) != 0;
                    let l2 = if muted { format!("Muted  Prog {id}") } else { format!("Prog {id}") };
                    (nm.chars().take(16).collect(), right(l2))
                }
                None => (top(&self.perf_body), right(format!("Perf {}", self.perf_id_label(self.perf_bank, self.perf_idx, self.perf_bundle.is_some())))),
            },
        }
    }

    fn select(&mut self, i: usize) {
        self.stash = None;
        let p = &self.programs[i];
        self.prog = p.body.clone();
        self.orig = p.body.clone();
        self.bank = p.bank.unwrap_or(5);
        self.idx = p.idx;
        self.label = p.label.clone();
        self.cur = Some(i);
        self.scroll_to_sel = true;
    }

    /// Perf mode with a Part selected: Prog up/down steps the Part's program by one, Bank up/down
    /// (d = +-128) keeps the number and changes the bank (manual: "Editing Performances").
    /// The Nova follows through Bank Select + Program Change on the Part's MIDI channel.
    fn step_part_program(&mut self, part: usize, d: i32) {
        let b = perf::part_off(part);
        let (bank, num) = (self.perf_body[b + perf::rel::PROG_BANK], self.perf_body[b + perf::rel::PROG_NUM]);
        let max_bank: u8 = if self.programs.iter().any(|p| p.bank.is_some_and(|b| b >= 7)) { 8 } else { 6 };
        let (nb, nn) = if d.abs() >= 128 {
            ((bank as i32 + d.signum()).clamp(5, max_bank as i32) as u8, num)
        } else {
            let lin = ((bank.clamp(5, max_bank) as i32 - 5) * 128 + num as i32 + d).clamp(0, (max_bank as i32 - 5) * 128 + 127);
            ((5 + lin / 128) as u8, (lin % 128) as u8)
        };
        if (nb, nn) == (bank, num) {
            return;
        }
        self.perf_body[b + perf::rel::PROG_BANK] = nb;
        self.perf_body[b + perf::rel::PROG_NUM] = nn;
        let body = self.find_program(nb, nn).map(|i| self.programs[i].body.clone());
        if let (Some(bundle), Some(body)) = (self.perf_bundle.as_mut(), body.clone()) {
            if let Some(slot) = bundle.parts.get_mut(part) {
                *slot = Some(body);
            }
        }
        self.load_part_program(part);
        let name = body.as_deref().map(prog_name).map(|n| n.trim().to_string()).unwrap_or_else(|| "(not in the list)".into());
        let mut note = String::new();
        if self.live_on {
            let global = self.globals.as_ref().map(|g| g.body[0] & 0x0f).unwrap_or(self.send_chan);
            let ch = match self.perf_body[b + perf::rel::CHAN] {
                c if c < 16 => c,
                _ => global,
            };
            let ok = self.midi.send(&[0xb0 | ch, 32, nb]).is_ok() && self.midi.send(&[0xc0 | ch, nn]).is_ok();
            if ok {
                self.live_sent += 2;
                self.live_push_log(format!("ch {}: Part {} bank {nb}, program change {nn}", ch + 1, part + 1));
                note = format!(" (sent on MIDI channel {})", ch + 1);
            }
        }
        self.set_status(format!("Part {}: {} {name}{note}", part + 1, perf::prog_id(nb, nn)), Some(true));
    }

    fn step_prog(&mut self, d: i32) {
        if let Some(part) = self.pstate.part.filter(|p| self.is_perf() && *p < perf::PARTS) {
            self.step_part_program(part, d);
            return;
        }
        if self.is_perf() {
            let n = self.perfs.len() as i32;
            let c = self.perf_cur.map(|c| c as i32).unwrap_or(0);
            if n > 0 {
                self.select_perf((c + d).rem_euclid(n) as usize);
            }
            return;
        }
        let n = self.programs.len() as i32;
        if n == 0 {
            return;
        }
        let c = self.cur.map(|c| c as i32).unwrap_or(0);
        self.select((c + d).rem_euclid(n) as usize);
    }

    fn save_assign(&self) {
        if let Some(p) = config_path() {
            if let Some(dir) = p.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            if let Ok(s) = serde_json::to_string_pretty(&self.assign) {
                let _ = std::fs::write(p, s);
            }
        }
    }

    fn open_file(&mut self) {
        let Some(path) = files::FileDialog::new()
            .add_filter("MIDI / SysEx", &["mid", "midi", "syx"])
            .pick_file()
        else {
            return;
        };
        self.load_files(&[absolute(&path)]);
    }

    /// Load files in order; each replaces the lists of the kinds it contains. Remembers the files that loaded.
    fn load_files(&mut self, files: &[PathBuf]) {
        if files.is_empty() {
            return;
        }
        let (mut msgs, mut ok, mut good) = (Vec::new(), true, Vec::new());
        for f in files {
            match self.load_file(f) {
                Ok(s) => {
                    msgs.push(s);
                    good.push(f.clone());
                }
                Err(e) => {
                    msgs.push(e);
                    ok = false;
                }
            }
        }
        if !good.is_empty() {
            save_recent(&good);
            self.lib_dirty = Some(std::time::Instant::now());
        }
        self.set_status(msgs.join("; "), Some(ok));
    }

    fn load_file(&mut self, path: &std::path::Path) -> Result<String, String> {
        let name = path.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        let d = std::fs::read(path).map_err(|e| format!("Cannot read {name}: {e}"))?;
        let (progs, perfs) = sysex::parse_all(&d);
        let (pats, glob) = sysex::parse_extras_file(&d);
        if progs.is_empty() && perfs.is_empty() && pats.is_empty() && glob.is_none() {
            return Err(format!("No Nova program, performance, arp pattern or globals dumps found in {name}"));
        }
        let mut msg = format!("Loaded {} programs, {} performances from {name}", progs.len(), perfs.len());
        if !pats.is_empty() || glob.is_some() {
            msg.push_str(&format!(" ({} arp patterns{})", pats.len(), if glob.is_some() { ", globals" } else { "" }));
        }
        self.set_extras(pats, glob);
        if !progs.is_empty() {
            self.programs = progs;
            self.select(0);
        }
        if !perfs.is_empty() {
            self.perfs = perfs;
            self.perf_cur = None;
            let bundle = self.perfs.iter().position(|p| p.bundle.is_some());
            if self.is_perf() || self.programs.is_empty() || bundle.is_some() {
                self.mode = Mode::Performance;
                self.pstate.perf_mode = true;
                self.pstate.part = Some(0);
                self.select_perf(bundle.unwrap_or(0));
            }
        }
        Ok(msg)
    }

    /// Reload the embedded factory data and forget the remembered files.
    fn use_factory(&mut self) {
        let (progs, perfs) = sysex::parse_all(TOTAL_MID);
        self.lib_dirty = Some(std::time::Instant::now());
        self.programs = progs;
        self.perfs = perfs;
        self.perf_cur = None;
        let (pats, glob) = sysex::parse_extras_file(TOTAL_MID);
        self.globals = None;
        self.globals_orig = None;
        self.patterns.clear();
        self.patterns_orig.clear();
        self.set_extras(pats, glob);
        self.mode = Mode::Program;
        self.pstate.perf_mode = false;
        if !self.programs.is_empty() {
            self.select(0);
        }
        save_recent(&[]);
        self.set_status(
            format!("Loaded {} factory programs, {} performances", self.programs.len(), self.perfs.len()),
            Some(true),
        );
    }

    fn export_syx(&mut self) {
        let perf_mode = self.is_perf();
        let body = if perf_mode { self.edited_perf() } else { self.edited_bytes() };
        let nm: String = prog_name(&body)
            .trim()
            .chars()
            .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_' || *c == ' ')
            .collect();
        let nm = if nm.is_empty() { (if perf_mode { "performance" } else { "program" }).to_string() } else { nm };
        let Some(path) = files::FileDialog::new()
            .set_file_name(format!("{nm}.syx"))
            .add_filter("SysEx", &["syx"])
            .save_file()
        else {
            return;
        };
        let data = if perf_mode {
            match self.bundle_stash.as_ref().or(self.perf_bundle.as_ref()) {
                Some(b) => sysex::bundle_to_syx(&body, b),
                None => {
                    let mut d = sysex::build_type7(&body, self.perf_bank, self.perf_idx, 0);
                    // programs saved from Part edits go in the same file, after the performance
                    for &(bank, idx) in &self.saved_slots {
                        if let Some(i) = self.find_program(bank, idx) {
                            d.extend(sysex::build_type2(&self.programs[i].body, bank, idx, 0));
                        }
                    }
                    d
                }
            }
        } else {
            sysex::build_type2(&body, self.bank, self.idx, 0)
        };
        match std::fs::write(&path, &data) {
            Ok(_) => self.set_status(format!("Saved {} ({} bytes)", path.display(), data.len()), Some(true)),
            Err(e) => self.set_status(format!("Save failed: {e}"), Some(false)),
        }
    }

    /// Collect mode: store bank dumps from the Nova in the library without touching the current
    /// selection and without marking them as "saved slots" (so they are never re-sent by Send set).
    fn absorb_bank_dump(&mut self, msg: &[u8]) -> bool {
        if !self.rx_collect || msg.len() < 9 || msg[1..6] != [0x00, 0x20, 0x29, 0x01, 0x21] {
            return false;
        }
        match msg[7] {
            0x1c => true,
            0x02 => {
                let Some(p) = sysex::parse_live(msg) else { return false };
                let Some(bank) = p.bank else { return false };
                let p_idx = p.idx;
                match self.find_program(bank, p.idx) {
                    Some(i) => self.programs[i].body = p.body,
                    None => self.programs.push(p),
                }
                self.rx_prog_n += 1;
                self.lib_dirty = Some(std::time::Instant::now());
                if bank >= 7 && !self.saved_slots.contains(&(bank, p_idx)) {
                    self.saved_slots.push((bank, p_idx));
                }
                self.dump_progress(true);
                self.set_status(format!("Collected {} programs, {} performances", self.rx_prog_n, self.rx_perf_n), Some(true));
                true
            }
            0x07 => {
                let Some(p) = sysex::parse_live_perf(msg) else { return false };
                match self.perfs.iter().position(|q| q.bundle.is_none() && q.bank == p.bank && q.idx == p.idx) {
                    Some(i) => self.perfs[i].body = p.body,
                    None => self.perfs.push(p),
                }
                self.rx_perf_n += 1;
                self.lib_dirty = Some(std::time::Instant::now());
                self.dump_progress(false);
                self.set_status(format!("Collected {} programs, {} performances", self.rx_prog_n, self.rx_perf_n), Some(true));
                true
            }
            _ => false,
        }
    }

    fn dump_progress(&mut self, prog: bool) {
        if let Some(d) = self.dump_all.as_mut() {
            d.last_rx = std::time::Instant::now();
            d.got += 1;
            if prog { d.progs += 1 } else { d.perfs += 1 }
        }
    }

    fn send_dump_request(&mut self, kind: u8) -> Result<(), String> {
        // 0x7f = any device: accepted whatever the Nova's global channel is
        self.midi.send(&sysex::dump_request(kind, 0x7f))
    }

    fn start_dump_all(&mut self) {
        let now = std::time::Instant::now();
        match self.send_dump_request(4) {
            Ok(_) => {
                self.dump_all = Some(DumpAll { stage: 0, started: now, last_rx: now, got: 0, progs: 0, perfs: 0, prev_collect: self.rx_collect });
                self.rx_collect = true;
                self.rx_prog_n = 0;
                self.rx_perf_n = 0;
                self.set_status("Dump everything: asked for all programs...", None);
            }
            Err(e) => self.set_status(format!("Send failed: {e}"), Some(false)),
        }
    }

    fn finish_dump_all(&mut self, msg: String, ok: bool) {
        if let Some(d) = self.dump_all.take() {
            self.rx_collect = d.prev_collect;
        }
        self.set_status(msg, Some(ok));
    }

    /// Advances a "Dump everything" run: a stage ends after a quiet period.
    fn dump_all_tick(&mut self, ctx: &egui::Context) {
        let Some(d) = self.dump_all.as_ref() else { return };
        ctx.request_repaint_after(std::time::Duration::from_millis(500));
        let now = std::time::Instant::now();
        let (stage, got, started, quiet) = (d.stage, d.got, d.started, now.duration_since(d.last_rx));
        if got == 0 && now.duration_since(started).as_secs() >= DUMP_NO_ANSWER_SECS {
            self.finish_dump_all("Dump everything: the Nova did not answer. Check that the MIDI output and input are both connected.".into(), false);
        } else if got > 0 && quiet.as_secs() >= DUMP_QUIET_SECS {
            if stage == 0 {
                match self.send_dump_request(9) {
                    Ok(_) => {
                        if let Some(d) = self.dump_all.as_mut() {
                            d.stage = 1;
                            d.got = 0;
                            d.started = now;
                            d.last_rx = now;
                        }
                        self.set_status("Dump everything: programs done, asked for all performances...", None);
                    }
                    Err(e) => self.finish_dump_all(format!("Send failed: {e}"), false),
                }
            } else {
                let (p, f) = self.dump_all.as_ref().map(|d| (d.progs, d.perfs)).unwrap_or((0, 0));
                self.finish_dump_all(format!("Dump everything finished: {p} programs, {f} performances received (existing slots overwritten)."), true);
            }
        }
    }

    fn handle_rx(&mut self, msg: Vec<u8>) {
        if msg.len() >= 6 && msg[1] == 0x7e && msg[3] == 0x06 && msg[4] == 0x02 {
            let id: Vec<String> = msg[5..msg.len() - 1].iter().map(|b| format!("{b:02x}")).collect();
            self.midi.id_replies.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            self.set_status(format!("Identity reply received ({}): the MIDI output AND input both work", id.join(" ")), Some(true));
            return;
        }
        if self.absorb_bank_dump(&msg) {
            return;
        }
        if self.receive_extras(&msg) {
            self.set_status("Received an arp pattern / globals dump from the Nova", Some(true));
            return;
        }
        let now = std::time::Instant::now();
        let recent = |t: std::time::Instant, s: u64| now.duration_since(t) < std::time::Duration::from_secs(s);
        if let Some((chan, body)) = sysex::parse_live_type5(&msg) {
            let mut b = sysex::Bundle { chan, ..Default::default() };
            for (i, slot) in self.rx_parts.iter_mut().enumerate() {
                if let Some((t, d)) = slot.take() {
                    if recent(t, 20) {
                        b.parts[i] = Some(d);
                    }
                }
            }
            let n = b.parts.iter().flatten().count();
            self.perfs.push(sysex::Performance { body, bank: 1, idx: 0, bundle: Some(b) });
            self.mode = Mode::Performance;
            self.pstate.perf_mode = true;
            let i = self.perfs.len() - 1;
            self.last_bundle = Some((i, now));
            self.pstate.part = Some(0);
            self.select_perf(i);
            self.set_status(format!("Received a performance from the Nova with {n} embedded programs (Nova buffer)"), Some(true));
            return;
        }
        if msg.len() > 8 && msg[1..6] == [0x00, 0x20, 0x29, 0x01, 0x21] && msg[7] == 0x1b {
            if let Some((i, t)) = self.last_bundle.filter(|(_, t)| recent(*t, 5)) {
                let _ = t;
                if let Some(b) = self.perfs[i].bundle.as_mut() {
                    b.extra.push(msg.clone());
                }
                if self.perf_cur == Some(i) {
                    for slot in [self.perf_bundle.as_mut(), self.perf_bundle_orig.as_mut()].into_iter().flatten() {
                        slot.extra.push(msg.clone());
                    }
                }
                return;
            }
        }
        if let Some(p) = sysex::parse_live(&msg) {
            if p.bank.is_none() && p.idx < 8 {
                self.rx_parts[p.idx as usize] = Some((now, p.body.clone()));
            }
        }
        if let Some(p) = sysex::parse_live_perf(&msg) {
            self.perfs.push(p);
            self.mode = Mode::Performance;
            self.pstate.perf_mode = true;
            let i = self.perfs.len() - 1;
            self.select_perf(i);
            self.set_status("Received a performance dump from the Nova (added to the performance list)", Some(true));
            return;
        }
        match sysex::parse_live(&msg) {
            Some(p) => {
                self.stash = None;
                self.learn_check(&p.body);
                self.prog = p.body.clone();
                self.orig = p.body.clone();
                self.bank = p.bank.unwrap_or(self.bank);
                self.idx = p.idx;
                self.label = "received from Nova".into();
                self.embedded_base = Some(p.body.clone());
                self.cur = None;
                if self.learn.is_none() {
                    self.set_status("Received a program dump from the Nova", Some(true));
                }
            }
            // the Nova's own extra messages (e.g. type 0x15 sent with its global settings) need no notice
            None if msg.len() > 7 && msg[1..6] == [0x00, 0x20, 0x29, 0x01, 0x21] => {}
            None => self.set_status(format!("Received SysEx ({} bytes), not a program dump", msg.len()), None),
        }
    }

    fn learn_check(&mut self, body: &[u8]) {
        let base = self.last_rx.replace(body.to_vec());
        let Some(id) = self.learn.clone() else { return };
        let Some(base) = base else {
            self.set_status(
                format!("Baseline stored. Now change \"{id}\" on the Nova and transmit the program again"),
                Some(true),
            );
            return;
        };
        match sysex::learn_diff(&base, body) {
            None => self.set_status("No byte changed - move the control further and transmit again", Some(false)),
            Some((big, all)) => {
                self.assign.insert(id.clone(), panel::Assign { off: big, min: 0, max: 127 });
                self.save_assign();
                let also: Vec<String> =
                    all.iter().filter(|&&x| x != big).map(|x| format!("0x{x:02x}")).collect();
                let mut s = format!("Learned \"{id}\" = byte 0x{big:02x}");
                if !also.is_empty() {
                    s.push_str(&format!(" (also changed: {})", also.join(", ")));
                }
                self.set_status(s, Some(true));
                self.learn = None;
                self.info_id = Some(id);
                self.readout_msg = None;
            }
        }
    }

    fn matches_filter(&self, p: &sysex::Program) -> bool {
        if self.filter.is_empty() {
            return true;
        }
        let f = self.filter.to_lowercase();
        p.name().to_lowercase().contains(&f)
            || CATS.get(p.body[0x11] as usize).map(|c| c.to_lowercase().contains(&f)).unwrap_or(false)
    }

    // ---------- UI pieces ----------

    fn top_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            ui.heading("Nova Editor");
            ui.selectable_value(&mut self.tab, Tab::Panel, "Panel");
            ui.selectable_value(&mut self.tab, Tab::Bytes, "All bytes");
            ui.selectable_value(&mut self.tab, Tab::Arp, "Arp patterns");
            ui.selectable_value(&mut self.tab, Tab::Globals, "Globals");
            ui.separator();
            if ui.button("Open .mid / .syx...").clicked() {
                self.open_file();
            }
            if ui.button("Use factory data").on_hover_text("Reload the embedded total.mid and forget remembered files").clicked() {
                self.use_factory();
            }
            if ui.button("Export .syx...").clicked() {
                self.export_syx();
            }
            let edited = if self.is_perf() {
                self.perf_stash.is_some() || self.perf_body != self.perf_orig || self.perf_bundle != self.perf_bundle_orig
            } else {
                self.stash.is_some() || self.prog != self.orig
            };
            if edited {
                ui.label(RichText::new("edited").background_color(Color32::from_rgb(0x22, 0x2c, 0x45)));
            }
            ui.separator();
            if ui.button("Connect MIDI...").clicked() {
                self.open_midi_window();
            }
            self.midi_summary(ui);
            let out = self.midi.has_output();
            let can_send = out
                && self.send_enabled
                && self.send_job.is_none()
                && (!self.is_perf() || self.perf_bundle.is_some());
            let why = if !out {
                "connect an output first"
            } else if !self.send_enabled {
                "tick 'Enable sending to the Nova' in the MIDI window first"
            } else if self.send_job.is_some() {
                "a send is in progress"
            } else {
                "only performances received/loaded as a Nova buffer (with embedded programs) can be sent"
            };
            if ui
                .add_enabled(can_send, egui::Button::new(RichText::new("Send to Nova...").color(Color32::from_rgb(0xf3, 0xa2, 0x4b))))
                .on_hover_text("Sends to the Nova's edit buffer after a confirmation")
                .on_disabled_hover_text(why)
                .clicked()
            {
                self.confirm_send = true;
            }
        });
    }

    fn is_perf(&self) -> bool {
        self.mode == Mode::Performance
    }

    fn program_bar(&mut self, ui: &mut egui::Ui) {
        let ro = if self.is_perf() { self.perf_stash.is_some() } else { self.stash.is_some() };
        ui.add_enabled_ui(!ro, |ui| {
            if self.is_perf() {
                self.perf_bar_inner(ui)
            } else {
                self.program_bar_inner(ui)
            }
        });
    }

    /// Name edit box shared by both modes; edits `body[..16]`.
    fn name_edit(ui: &mut egui::Ui, buf: &mut String, body: &mut [u8]) {
        ui.label("Name");
        let focused = ui.memory(|m| m.has_focus(egui::Id::new("name_edit")));
        if !focused {
            *buf = prog_name(body).trim_end().to_string();
        }
        let r = ui.add(
            egui::TextEdit::singleline(buf)
                .id(egui::Id::new("name_edit"))
                .char_limit(16)
                .font(egui::TextStyle::Monospace)
                .desired_width(170.0),
        );
        if r.changed() {
            let mut t: Vec<u8> = buf.chars().map(|c| if c.is_ascii() && !c.is_control() { c as u8 } else { b' ' }).collect();
            t.resize(16, b' ');
            body[..16].copy_from_slice(&t[..16]);
        }
    }

    fn perf_bar_inner(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            Self::name_edit(ui, &mut self.name_buf, &mut self.perf_body);
            ui.label("Perf bank byte");
            ui.add(egui::DragValue::new(&mut self.perf_bank).range(0..=7))
                .on_hover_text("Performance bank byte (1 = A)");
            ui.label("Number");
            ui.add(egui::DragValue::new(&mut self.perf_idx).range(0..=127));
            ui.weak(self.perf_id_label(self.perf_bank, self.perf_idx, self.perf_bundle.is_some()));
            if ui.button("Revert").clicked() {
                self.perf_body = self.perf_orig.clone();
                self.perf_bundle = self.perf_bundle_orig.clone();
                if let Some(p) = self.pstate.part {
                    self.load_part_program(p);
                }
            }
            ui.weak("Performance");
        });
    }

    fn program_bar_inner(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            Self::name_edit(ui, &mut self.name_buf, &mut self.prog);
            ui.label("Category");
            let c = self.prog[0x11] as usize;
            let txt = CATS.get(c).map(|n| format!("{c} - {n}")).unwrap_or(format!("{c} - ?"));
            egui::ComboBox::from_id_salt("cat").selected_text(txt).show_ui(ui, |ui| {
                for (i, n) in CATS.iter().enumerate() {
                    if ui.selectable_label(i == c, format!("{i} - {n}")).clicked() {
                        self.prog[0x11] = i as u8;
                    }
                }
            });
            ui.label("Bank");
            ui.add(egui::DragValue::new(&mut self.bank).range(0..=7));
            ui.label("Number");
            ui.add(egui::DragValue::new(&mut self.idx).range(0..=127));
            ui.label("Part");
            ui.add(egui::DragValue::new(&mut self.part).range(0..=9)).on_hover_text("Target part for Send (9 = current)");
            if ui.button("Revert").clicked() {
                self.prog = self.orig.clone();
            }
            if !self.label.is_empty() {
                ui.weak(&self.label);
            }
        });
    }

    fn bytes_view(&mut self, ui: &mut egui::Ui) {
        let ro = if self.is_perf() { self.perf_stash.is_some() } else { self.stash.is_some() };
        ui.add_enabled_ui(!ro, |ui| self.bytes_inner(ui));
    }

    /// Section list: (title, first, last) for the active mode.
    fn byte_sections(&self) -> Vec<(String, usize, usize)> {
        if self.is_perf() {
            let mut v = vec![("Name".to_string(), 0, 15), ("Header (raw, unknown)".to_string(), 16, 19)];
            for p in 0..perf::PARTS {
                let a = perf::part_off(p);
                let extra = if p >= 6 { " (extra, not a front-panel part)" } else { "" };
                v.push((format!("Part {}{extra}", p + 1), a, a + perf::PART_LEN - 1));
            }
            v
        } else {
            params::SECTIONS.iter().map(|(t, a, b)| (t.to_string(), *a, *b)).collect()
        }
    }

    /// Row label, hover text and observed-range text for byte `o` of the active buffer.
    fn byte_info(&self, o: usize) -> (String, String, String) {
        if self.is_perf() {
            if o < 16 {
                return (format!("Name char {o}"), String::new(), String::new());
            }
            if o < perf::PART_BASE {
                return (format!("Header byte {}", o - 16), String::new(), String::new());
            }
            let r = (o - perf::PART_BASE) % perf::PART_LEN;
            if let Some(n) = perf::rel_name(r) {
                return (n.to_string(), String::new(), String::new());
            }
            let pp = &self.params[0xfe + r - perf::rel::FX];
            let nm = params::display_name(pp);
            let nm = if nm.is_empty() { format!("Effects byte 0x{:02x}", 0xfe + r - perf::rel::FX) } else { nm };
            return (nm, pp.sources.join("\n"), String::new());
        }
        let p = &self.params[o];
        let obs = match (p.observed_min, p.observed_max) {
            (Some(lo), Some(hi)) => format!("seen {lo}-{hi}"),
            _ => String::new(),
        };
        (params::display_name(p), p.sources.join("\n"), obs)
    }

    fn bytes_inner(&mut self, ui: &mut egui::Ui) {
        let sections = self.byte_sections();
        let perf_mode = self.is_perf();
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            for (title, a, b) in sections {
                egui::CollapsingHeader::new(format!("{title}   0x{a:02x}-0x{b:02x}"))
                    .id_salt((perf_mode, a))
                    .default_open(!perf_mode || a < 20 + 44 * 2)
                    .show(ui, |ui| {
                        egui::Grid::new((perf_mode, "sec", a)).striped(true).num_columns(5).show(ui, |ui| {
                            for o in a..=b {
                                let (cur, orig) = if perf_mode { (self.perf_body[o], self.perf_orig[o]) } else { (self.prog[o], self.orig[o]) };
                                let (nm, src, obs) = self.byte_info(o);
                                let mut off = RichText::new(format!("0x{o:02x}")).monospace();
                                if cur != orig {
                                    off = off.color(Color32::from_rgb(0x6b, 0x9b, 0xff));
                                }
                                ui.label(off);
                                let lbl = if nm.is_empty() {
                                    ui.label(RichText::new(format!("Byte 0x{o:02x}")).weak())
                                } else {
                                    ui.label(nm)
                                };
                                if !src.is_empty() {
                                    lbl.on_hover_text(src);
                                }
                                let mut v = cur;
                                let r1 = ui.add(egui::Slider::new(&mut v, 0..=127).show_value(false));
                                let r2 = ui.add(egui::DragValue::new(&mut v).range(0..=127));
                                if r1.changed() || r2.changed() {
                                    if perf_mode { self.perf_body[o] = v } else { self.prog[o] = v }
                                }
                                ui.weak(obs);
                                ui.end_row();
                            }
                        });
                    });
            }
        });
    }

    /// "Part parameters" strip shown under the panel in Performance mode.
    /// One row per part; returns the part whose row was clicked.
    fn parts_overview(&self, ui: &mut egui::Ui) -> Option<usize> {
        use perf::rel;
        let mut clicked = None;
        let muted_mask = self.display_muted();
        let grid = |ui: &mut egui::Ui, range: std::ops::Range<usize>, clicked: &mut Option<usize>, id: &str| {
            egui::Grid::new(id).striped(true).num_columns(10).show(ui, |ui| {
                for h in ["Part", "Program", "MIDI ch", "Range", "Semi", "Detune", "Poly", "Level", "Muted", "Effects"] {
                    ui.strong(h);
                }
                ui.end_row();
                for p in range {
                    let b = perf::part_off(p);
                    let bd = &self.perf_body;
                    let (pid, pname) = self.part_prog_info(p);
                    let muted = p < 6 && muted_mask & (1 << p) != 0;
                    let sel = self.pstate.part == Some(p);
                    let txt = |s: String| {
                        let t = RichText::new(s);
                        if muted { t.color(Color32::from_gray(110)) } else { t }
                    };
                    if ui.selectable_label(sel, txt(format!("{}", p + 1))).clicked() {
                        *clicked = Some(p);
                    }
                    if ui.selectable_label(sel, txt(format!("{pid} {pname}"))).clicked() {
                        *clicked = Some(p);
                    }
                    ui.label(txt(perf::midi_channel_label(bd[b + rel::CHAN])));
                    ui.label(txt(format!("{}-{}", perf::note_name(bd[b + rel::LOW]), perf::note_name(bd[b + rel::HIGH]))));
                    ui.label(txt(format!("{:+}", bd[b + rel::SEMI] as i32 - 64)));
                    ui.label(txt(format!("{:+}", bd[b + rel::DETUNE] as i32 - 64)));
                    ui.label(txt(perf::POLY_NAMES[perf::poly_get(bd, p) as usize].to_string()));
                    ui.label(txt(bd[b + rel::LEVEL].to_string()));
                    ui.label(txt(if muted { "Muted".into() } else { String::new() }));
                    ui.label(txt(if perf::effects_used_get(bd, p) { "Program" } else { "Part" }.to_string()));
                    ui.end_row();
                }
            });
        };
        egui::CollapsingHeader::new("Parts overview (click a row to select the part)").id_salt("parts_overview").default_open(false).show(ui, |ui| {
            grid(ui, 0..6, &mut clicked, "ov16");
            egui::CollapsingHeader::new("Extra parts 7-8").default_open(false).show(ui, |ui| {
                grid(ui, 6..8, &mut clicked, "ov78");
            });
        });
        clicked
    }

    fn part_strip(&mut self, ui: &mut egui::Ui) {
        use perf::rel;
        if let Some(p) = self.parts_overview(ui) {
            self.select_part(p);
        }
        let ro = self.perf_stash.is_some();
        ui.add_enabled_ui(!ro, |ui| {
            let Some(p) = self.pstate.part else {
                ui.label("Part parameters: select a part with the Part Edit buttons 1-6.");
                return;
            };
            let b = perf::part_off(p);
            let mut reload = false;
            ui.horizontal_wrapped(|ui| {
                ui.strong(format!("Part {} parameters", p + 1));
                let embedded = self.perf_bundle.as_ref().is_some_and(|x| x.parts[p].is_some());
                if embedded {
                    ui.label(format!("Program: embedded - {} (edited with the front panel)", self.part_prog_info(p).1));
                } else {
                ui.label("Program");
                let mut bank = self.perf_body[b + rel::PROG_BANK];
                egui::ComboBox::from_id_salt("pbank")
                    .selected_text(perf::prog_bank_letter(bank).to_string())
                    .show_ui(ui, |ui| {
                        for v in [5u8, 6, 7, 8] {
                            ui.selectable_value(&mut bank, v, perf::prog_bank_letter(v).to_string());
                        }
                    });
                let mut num = self.perf_body[b + rel::PROG_NUM];
                ui.add(egui::DragValue::new(&mut num).range(0..=127));
                if bank != self.perf_body[b + rel::PROG_BANK] || num != self.perf_body[b + rel::PROG_NUM] {
                    reload = true;
                }
                self.perf_body[b + rel::PROG_BANK] = bank;
                self.perf_body[b + rel::PROG_NUM] = num;
                let pname = self
                    .find_program(bank, num)
                    .map(|i| self.programs[i].name())
                    .unwrap_or_else(|| "(not in program list)".into());
                ui.label(format!("{} {}", perf::prog_id(bank, num), pname));
                }
                ui.label("Level");
                let mut lv = self.perf_body[b + rel::LEVEL];
                ui.add(egui::Slider::new(&mut lv, 0..=127));
                self.perf_body[b + rel::LEVEL] = lv;
            });
            let embedded_now = self.perf_bundle.as_ref().is_some_and(|x| x.parts[p].is_some());
            if self.perf_slot_key().is_some() && !embedded_now {
                ui.horizontal_wrapped(|ui| {
                    let known = slots::block_for_perf(self.perf_bank, self.perf_idx);
                    let target = known.map(|k| slots::slot(k, p.min(slots::PARTS - 1))).map(|(bk, ix)| perf::prog_id(bk, ix));
                    if ui
                        .button("Make performance independent")
                        .on_hover_text("Copies the programs of Parts 1-6 into this performance's own private slots in banks C/D and repoints the Parts, so later edits never touch programs other performances use.")
                        .clicked()
                    {
                        self.make_perf_independent();
                    }
                    if self.perf_bank == 1 && self.perf_idx == 0 && ui.button("Fill A001-A041 from this performance...").clicked() {
                        self.confirm_fill = true;
                    }
                    if !self.saved_slots.is_empty() && ui.button("Export performance set (.syx)...").clicked() {
                        self.export_filled_set();
                    }
                    let can = !self.saved_slots.is_empty() && self.midi.has_output() && self.send_enabled && self.send_job.is_none();
                    if ui
                        .add_enabled(can, egui::Button::new("Send performance set to Nova..."))
                        .on_hover_text("Needs an output and 'Enable sending to the Nova'. Asks for confirmation first.")
                        .clicked()
                    {
                        self.confirm_set_send = true;
                        self.set_send_ack = false;
                    }
                    match (&target, known) {
                        (Some(t), Some(k)) => ui.weak(format!("private block {} of {}: this Part's slot is {t}", k + 1, slots::BLOCKS)),
                        _ => ui.weak("outside A000-A041: no private block (use the manual slot picker)"),
                    };
                });
            }
            if !embedded_now && self.prog != self.orig {
                let cur_slot = (self.perf_body[b + rel::PROG_BANK], self.perf_body[b + rel::PROG_NUM]);
                ui.horizontal_wrapped(|ui| {
                    ui.colored_label(Color32::from_rgb(0xf3, 0xa2, 0x4b), "This Part's program is edited but not saved in the performance.");
                    if self.perf_slot_key().is_some() && p < slots::PARTS {
                        if let Ok((bk, ix)) = self.private_slot(p) {
                            let same = (bk, ix) == cur_slot;
                            let label = if same { format!("Save edit to {}", perf::prog_id(bk, ix)) } else { format!("Save edit to private slot {}", perf::prog_id(bk, ix)) };
                            if ui.button(label).on_hover_text("Stores the edit in this Part's private slot (banks C/D) and points the Part at it. Original programs are never overwritten.").clicked() {
                                if let Ok((bank, idx)) = self.private_slot(p) {
                                    let body = self.prog.clone();
                                    self.store_program(bank, idx, body);
                                    self.perf_body[b + rel::PROG_BANK] = bank;
                                    self.perf_body[b + rel::PROG_NUM] = idx;
                                    if let Some(i) = self.find_program(bank, idx) {
                                        self.select(i);
                                    }
                                    self.set_status(format!("Part {} edit saved to {}. Export .syx saves it with the performance.", p + 1, perf::prog_id(bank, idx)), Some(true));
                                }
                            }
                        }
                    }
                    ui.label("or choose a slot:");
                    egui::ComboBox::from_id_salt("newslot_bank")
                        .selected_text(perf::prog_bank_letter(self.new_slot.0).to_string())
                        .show_ui(ui, |ui| {
                            for v in [5u8, 6, 7, 8] {
                                ui.selectable_value(&mut self.new_slot.0, v, perf::prog_bank_letter(v).to_string());
                            }
                        });
                    ui.add(egui::DragValue::new(&mut self.new_slot.1).range(0..=127));
                    let existing = self.find_program(self.new_slot.0, self.new_slot.1).map(|i| self.programs[i].name());
                    ui.label(match &existing {
                        Some(n) => format!("{} (replaces \"{}\" in the app's list)", perf::prog_id(self.new_slot.0, self.new_slot.1), n.trim()),
                        None => format!("{} (empty in the app's list)", perf::prog_id(self.new_slot.0, self.new_slot.1)),
                    });
                    let same = self.new_slot == cur_slot;
                    if ui
                        .add_enabled(!same, egui::Button::new("Save & point this Part at it"))
                        .on_hover_text("The original program is left untouched. The edit is stored as its own program in the chosen slot and this Part is repointed to it.")
                        .clicked()
                    {
                        let (bank, idx) = self.new_slot;
                        let body = self.prog.clone();
                        self.store_program(bank, idx, body);
                        self.perf_body[b + rel::PROG_BANK] = bank;
                        self.perf_body[b + rel::PROG_NUM] = idx;
                        if let Some(i) = self.find_program(bank, idx) {
                            self.select(i);
                        }
                        self.set_status(
                            format!("Part {} edit saved as {} (original program untouched). Export .syx saves the performance together with it.", p + 1, perf::prog_id(bank, idx)),
                            Some(true),
                        );
                    }
                    if same {
                        ui.weak("pick a different slot than the Part's current program");
                    }
                });
            }
            ui.horizontal_wrapped(|ui| {
                match self.pstate.ppage {
                    "tune" => {
                        let mut semi = self.perf_body[b + rel::SEMI] as i32 - 64;
                        ui.label("Semitone");
                        if ui.add(egui::DragValue::new(&mut semi).range(-24..=24)).changed() {
                            self.perf_body[b + rel::SEMI] = (semi + 64) as u8;
                        }
                        let mut det = self.perf_body[b + rel::DETUNE] as i32 - 64;
                        ui.label("Detune (cents)");
                        if ui.add(egui::DragValue::new(&mut det).range(-64..=63)).changed() {
                            self.perf_body[b + rel::DETUNE] = (det + 64) as u8;
                        }
                    }
                    "vel" => {
                        ui.label("Velocity curve");
                        let raw = self.perf_body[b + rel::VEL];
                        let cur = raw as i32 - 80;
                        let txt = perf::VEL_CURVES.get(cur.max(0) as usize).filter(|_| cur >= 0).map(|s| s.to_string()).unwrap_or(format!("raw {raw}"));
                        egui::ComboBox::from_id_salt("vel").selected_text(txt).show_ui(ui, |ui| {
                            for (i, n) in perf::VEL_CURVES.iter().enumerate() {
                                if ui.selectable_label(cur == i as i32, *n).clicked() {
                                    self.perf_body[b + rel::VEL] = 80 + i as u8;
                                }
                            }
                        });
                    }
                    "range" => {
                        for (lbl, r) in [("Lowest note", rel::LOW), ("Highest note", rel::HIGH)] {
                            ui.label(lbl);
                            let mut n = self.perf_body[b + r];
                            let w = egui::DragValue::new(&mut n)
                                .range(0..=127)
                                .custom_formatter(|v, _| perf::note_name(v as u8))
                                .custom_parser(|s| perf::parse_note(s).map(|n| n as f64));
                            if ui.add(w).changed() {
                                self.perf_body[b + r] = n;
                            }
                        }
                        ui.weak("lowest above highest leaves a hole");
                    }
                    "midi" => {
                        let ch = self.perf_body[b + rel::CHAN];
                        ui.label("MIDI channel");
                        egui::ComboBox::from_id_salt("midich").selected_text(perf::midi_channel_label(ch)).show_ui(ui, |ui| {
                            for v in 0..=17u8 {
                                if ui.selectable_label(ch == v, perf::midi_channel_label(v)).clicked() {
                                    self.perf_body[b + rel::CHAN] = v;
                                }
                            }
                        });
                        ui.weak(format!("raw byte {} (16 = Omni is assumed)", self.perf_body[b + rel::CHAN]));
                        for (lbl, r) in [("raw +3", rel::RAW3), ("raw +4", rel::RAW4)] {
                            ui.label(lbl);
                            let mut v = self.perf_body[b + r];
                            if ui.add(egui::DragValue::new(&mut v).range(0..=127)).changed() {
                                self.perf_body[b + r] = v;
                            }
                        }
                    }
                    page => {
                        let raw_i = b + rel::PACKED;
                        if page == "poly" {
                            ui.label("Polyphony");
                            let cur = perf::poly_get(&self.perf_body, p);
                            egui::ComboBox::from_id_salt("poly")
                                .selected_text(perf::POLY_NAMES[cur as usize])
                                .show_ui(ui, |ui| {
                                    for (i, n) in perf::POLY_NAMES.iter().enumerate() {
                                        if ui.selectable_label(cur as usize == i, *n).clicked() {
                                            perf::poly_set(&mut self.perf_body, p, i as u8);
                                        }
                                    }
                                });
                            if p < 6 {
                                let mut muted = perf::is_part_muted(&self.perf_body, p);
                                if ui.checkbox(&mut muted, "Muted").changed() {
                                    perf::set_part_muted(&mut self.perf_body, p, muted);
                                }
                            }
                        } else {
                            ui.label("Effects used");
                            let prog_fx = perf::effects_used_get(&self.perf_body, p);
                            egui::ComboBox::from_id_salt("fxused")
                                .selected_text(if prog_fx { "Program" } else { "Part" })
                                .show_ui(ui, |ui| {
                                    if ui.selectable_label(prog_fx, "Program").clicked() {
                                        perf::effects_used_set(&mut self.perf_body, p, true);
                                    }
                                    if ui.selectable_label(!prog_fx, "Part").clicked() {
                                        perf::effects_used_set(&mut self.perf_body, p, false);
                                    }
                                });
                            let mut f4 = perf::flag4_get(&self.perf_body, p);
                            if ui.checkbox(&mut f4, "flag bit 4 (unknown)").changed() {
                                perf::flag4_set(&mut self.perf_body, p, f4);
                            }
                        }
                        let mut v = self.perf_body[raw_i];
                        ui.label("raw byte +10");
                        if ui.add(egui::DragValue::new(&mut v).range(0..=127)).changed() {
                            self.perf_body[raw_i] = v;
                        }
                        ui.weak("bits 0-1 polyphony, bit 6 effects used, bit 4 unknown");
                    }
                }
            });
            if reload {
                self.load_part_program(p);
            }
        });
    }

    fn panel_view(&mut self, ui: &mut egui::Ui) {
        self.update_fx_part();
        let lcd = self.lcd_lines();
        let perf = self.is_perf();
        let flags = panel::Flags {
            mute_mode: self.mute_mode,
            copy_mode: self.copy_mode,
            trig_open: self.show_trig,
            solo: self.solo.is_some(),
            muted: self.display_muted(),
            prog: !perf,
            perf,
            midi: self.show_midi,
            list: self.show_list,
            midi_ok: self.midi.conn_in.is_some() && self.midi.has_output(),
            dump: self.show_dump,
        };
        // while comparing, controls act on scratch copies so the original stays read-only
        let mut scratch = self.prog.clone();
        let mut perf_scratch = self.perf_body.clone();
        let perf_ro = self.perf_stash.is_some();
        let ro = self.stash.is_some() || (perf && perf_ro);
        let out = panel::draw(
            ui,
            &mut self.pstate,
            if ro { &mut scratch } else { &mut self.prog },
            &self.assign,
            &self.params,
            if perf_ro { &mut perf_scratch } else { &mut self.perf_body },
            lcd,
            flags,
        );
        if let Some(fx) = out.fx_clicked {
            if self.copy_mode {
                self.do_copy(fx);
            }
        }
        if let Some(p) = out.mute_toggle {
            if self.perf_stash.is_none() {
                let m = perf::is_part_muted(&self.perf_body, p);
                perf::set_part_muted(&mut self.perf_body, p, !m);
            }
        }
        if let Some(p) = out.part_selected {
            self.select_part(p);
        }
        if let Some(id) = out.info {
            self.info_id = Some(id);
            self.readout_msg = None;
        }
        if let Some(m) = out.select_msg {
            self.readout_msg = Some(m);
        }
        match out.action {
            Some("midi") => self.open_midi_window(),
            Some("dump") => self.show_dump = true,
            Some("list") => {
                self.show_list = !self.show_list;
                self.scroll_to_sel = true;
            }
            Some("perf") => {
                if self.is_perf() {
                    // as on the Nova: back to the performance itself; Bank/Prog then select performances
                    self.pstate.part = None;
                    self.set_status("Performance view: Bank/Prog select performances. Press a Part button to change that Part's program.", None);
                } else {
                    self.set_mode(Mode::Performance);
                }
            }
            Some("x_menu") => self.show_fx = !self.show_fx,
            Some("e_trig") => self.show_trig = !self.show_trig,
            Some(a @ ("o_menu" | "f_menu" | "l_menu" | "e_menu")) => {
                let key = match a {
                    "o_menu" => "osc",
                    "f_menu" => "filter",
                    "l_menu" => "lfo",
                    _ => "env",
                };
                if key == "osc" && self.pstate.osc >= 3 {
                    self.set_status("no menu for this source", Some(false));
                } else if let Some(i) = self.menus_open.iter().position(|k| *k == key) {
                    self.menus_open.remove(i);
                } else {
                    self.menus_open.push(key);
                }
            }
            Some("a_menu") => self.show_arp = !self.show_arp,
            Some("x_copy") if self.is_perf() => {
                if self.copy_mode {
                    self.copy_mode = false;
                } else if self.pstate.part.is_none() {
                    self.set_status("select a Part first", Some(false));
                } else {
                    self.copy_mode = true;
                    self.set_status("Copy mode: click an effect button (delay, reverb, chorus, dist, pan)", Some(true));
                }
            }
            Some("mute") if self.is_perf() => self.mute_mode = !self.mute_mode,
            Some("solo") if self.is_perf() => {
                if self.solo.is_some() {
                    self.solo = None;
                } else if let Some(p) = self.pstate.part.filter(|p| *p < 6) {
                    self.solo = Some(p);
                } else {
                    self.set_status("select a Part first", Some(false));
                }
            }
            Some("prog") => self.set_mode(Mode::Program),
            Some("write") => self.export_syx(),
            Some("pageU") => self.tab = Tab::Panel,
            Some("pageD") => self.tab = Tab::Bytes,
            _ => {}
        }
        if let Some(d) = out.step_prog {
            self.step_prog(d);
        }
        if let Some(id) = out.right_click {
            let a = self.assign.get(&id);
            self.assign_win = Some(AssignWin {
                off: a.map(|a| format!("0x{:02x}", a.off)).unwrap_or_default(),
                min: a.map(|a| a.min.to_string()).unwrap_or("0".into()),
                max: a.map(|a| a.max.to_string()).unwrap_or("127".into()),
                id,
            });
        }
    }

    /// Messages and a description of exactly what Send would transmit.
    fn send_plan(&self) -> Result<(Vec<Vec<u8>>, String), String> {
        let chan = self.send_chan;
        if self.is_perf() {
            let b = self.bundle_stash.as_ref().or(self.perf_bundle.as_ref()).ok_or("this performance has no embedded programs")?;
            let msgs = sysex::bundle_send_messages(&self.edited_perf(), b, chan)?;
            let parts: Vec<String> = b.parts.iter().take(6).enumerate().filter(|(_, p)| p.is_some()).map(|(i, _)| (i + 1).to_string()).collect();
            let desc = format!(
                "{} messages on SysEx channel {chan}: type-0 part dumps (296 bytes each) for parts {}, then the type-5 performance dump (381 bytes) \"{}\". Parts 7-8 and the 0x1b message are not sent.",
                msgs.len(),
                parts.join(", "),
                prog_name(&self.edited_perf()).trim()
            );
            Ok((msgs, desc))
        } else {
            let part = self.part;
            let msg = sysex::program_send_message(&self.edited_bytes(), part, chan)?;
            let desc = format!(
                "1 message on SysEx channel {chan}: type-0 part dump (296 bytes) of \"{}\" to part {part}.",
                prog_name(&self.edited_bytes()).trim()
            );
            Ok((vec![msg], desc))
        }
    }

    /// Receive-only mirror of the hardware's controller changes into the current program.
    fn mirror_chan_combo(&mut self, ui: &mut egui::Ui, id: &str) {
        egui::ComboBox::from_id_salt(id).selected_text(self.mirror_chan.label()).show_ui(ui, |ui| {
            let mut opts = vec![mirror::ChanMode::Any, mirror::ChanMode::Global];
            opts.extend((0..16).map(mirror::ChanMode::Ch));
            opts.push(mirror::ChanMode::Follow);
            for o in opts {
                ui.selectable_value(&mut self.mirror_chan, o, o.label());
            }
        });
    }

    /// Connection summary with live counters plus the Mirror controls (top bar).
    fn midi_summary(&mut self, ui: &mut egui::Ui) {
        let (chan, sx, ago) = self.midi.stats.snapshot();
        match &self.midi.conn_in {
            Some(n) => {
                ui.label(format!("In: {n} | msgs: {chan} channel, {sx} sysex | last: {}", midi::ago_text(ago)));
            }
            None => {
                ui.weak("MIDI in: not connected");
            }
        }
        ui.checkbox(&mut self.mirror_on, "Mirror").on_hover_text("Apply the hardware's knob changes (receive-only)");
        self.mirror_chan_combo(ui, "mirror_chan_top");
        let out = self.midi.has_output();
        let r = ui.add_enabled(out, egui::Checkbox::new(&mut self.live_on, "Live"))
            .on_hover_text("Live edit: send edits in Program mode to the Nova as CC/NRPN (edit buffer only). Off at every start.");
        if r.changed() && self.live_on {
            self.live_resync();
            self.live_push_selection();
        }
        if self.mirror_on && ago.is_some_and(|a| a < 1000) {
            ui.colored_label(Color32::from_rgb(0x4c, 0xc3, 0x8a), "\u{25cf}");
        }
    }

    fn live_selection(&self) -> (bool, Option<usize>) {
        if self.is_perf() { (true, self.perf_cur) } else { (false, self.cur) }
    }

    /// Connected: start Live edit at once and select the app's program / performance on the Nova.
    /// First asks the Nova for its global settings (for its MIDI channel); live_tick starts Live edit
    /// when they arrive, or after 2.5 s with what the app has.
    fn start_live(&mut self) {
        if self.midi.has_output() {
            let _ = self.send_dump_request(0x10);
            self.live_wait = Some(std::time::Instant::now());
        }
    }

    fn live_resync(&mut self) {
        self.live_snap = self.prog.clone();
        self.live_key = (self.is_perf(), self.cur, self.stash.is_some(), self.pstate.part);
        self.live_sel = self.live_selection();
    }

    fn live_push_log(&mut self, line: String) {
        if self.live_log.len() >= 40 {
            self.live_log.pop_front();
        }
        self.live_log.push_back(line);
    }

    /// Select the app's current program / performance on the Nova (bank select + program change).
    fn live_push_selection(&mut self) {
        let sel = self.live_selection();
        let chan = self.globals.as_ref().map(|g| g.body[0] & 0x0f).unwrap_or(self.send_chan);
        let target = if sel.0 {
            (self.perf_bundle.is_none() && sel.1.is_some()).then_some((self.perf_bank, self.perf_idx))
        } else {
            sel.1.map(|_| (self.bank, self.idx))
        };
        let Some((bank, idx)) = target else { return };
        let msgs = [[0xb0 | chan, 32, bank], [0xc0 | chan, idx, 0]];
        for (i, m) in msgs.iter().enumerate() {
            let r = if i == 0 { self.midi.send(&m[..3]) } else { self.midi.send(&m[..2]) };
            if let Err(e) = r {
                self.live_on = false;
                self.set_status(format!("Live edit stopped: {e}"), Some(false));
                return;
            }
            self.live_sent += 1;
        }
        self.live_push_log(format!("ch {}: bank {bank}, program change {idx} (select on the Nova)", chan + 1));
        self.set_status(format!("Live: sent bank {bank} + program change {idx} on MIDI channel {} (the Nova must be in {} mode)", chan + 1, if sel.0 { "Perf" } else { "Prog" }), None);
    }

    /// Live edit: send changed program bytes to the Nova.
    fn live_tick(&mut self) {
        if let Some(t) = self.live_wait {
            if !self.globals_live && t.elapsed() < std::time::Duration::from_millis(2500) {
                return;
            }
            self.live_wait = None;
            self.live_on = true;
            self.live_resync();
            self.live_push_selection();
            let ch = self.globals.as_ref().map(|g| g.body[0] & 0x0f).unwrap_or(self.send_chan) + 1;
            if self.globals_live {
                self.set_status(format!("Live edit on: the Nova's global MIDI channel is {ch}"), Some(true));
            } else {
                self.set_status(format!("Live edit on, but the Nova did not send its settings; using MIDI channel {ch}"), Some(false));
            }
        }
        if !self.live_on {
            // always on while connected: resume after a bank send or a send error
            if self.midi.has_output() && self.send_job.is_none() && self.live_wait.is_none() {
                self.live_on = true;
                self.live_resync();
            }
            return;
        }
        let sel = self.live_selection();
        if sel != self.live_sel {
            // a different program / performance was picked in the app: select it on the Nova too
            self.live_resync();
            self.live_push_selection();
            return;
        }
        let key = (self.is_perf(), self.cur, self.stash.is_some(), self.pstate.part);
        if key != self.live_key || self.live_snap.len() != self.prog.len() {
            self.live_key = key;
            self.live_snap = self.prog.clone();
            return;
        }
        if self.stash.is_some() || self.prog == self.live_snap {
            self.live_snap = self.prog.clone();
            return;
        }
        let global = self.globals.as_ref().map(|g| g.body[0] & 0x0f).unwrap_or(self.send_chan);
        // Performance mode: the panel edits the selected Part's program, so send on that Part's channel
        let chan = match (self.is_perf(), self.pstate.part) {
            (true, Some(p)) if p < perf::PARTS => match self.perf_body.get(perf::part_off(p) + perf::rel::CHAN) {
                Some(&c) if c < 16 => c,
                _ => global,
            },
            _ => global,
        };
        let mut msgs: Vec<[u8; 3]> = Vec::new();
        for off in 16..self.prog.len() {
            if self.prog[off] != self.live_snap[off] {
                match live::messages_for(off, self.live_snap[off], self.prog[off], &self.params, chan) {
                    Some(m) => msgs.extend(m),
                    None => {
                        let line = format!("byte 0x{off:02x} = {} has no CC/NRPN route (not sent)", self.prog[off]);
                        if self.live_log.len() >= 40 {
                            self.live_log.pop_front();
                        }
                        self.live_log.push_back(line);
                    }
                }
            }
        }
        self.live_snap = self.prog.clone();
        for m in msgs {
            if let Err(e) = self.midi.send(&m) {
                self.live_on = false;
                self.set_status(format!("Live edit stopped: {e}"), Some(false));
                return;
            }
            self.live_sent += 1;
            if self.live_log.len() >= 40 {
                self.live_log.pop_front();
            }
            self.live_log.push_back(format!("ch {}: {:02x} {:02x} {:02x}", (m[0] & 15) + 1, m[0], m[1], m[2]));
        }
    }

    fn mirror_log_push(&mut self, line: String) {
        if self.mirror_log.len() >= 40 {
            self.mirror_log.pop_front();
        }
        self.mirror_log.push_back(line);
    }

    /// Bank Select (CC32) and Program Change from the hardware: load the matching stored patch.
    /// Returns true when the message was consumed.
    fn mirror_patch_select(&mut self, m: mirror::ChannelMsg, global: u8) -> bool {
        let ch = m.chan();
        match m.status & 0xf0 {
            0xb0 if m.d1 == 32 => {
                self.rx_bank[ch as usize] = Some(m.d2);
                self.mirror_log_push(format!("ch {}: Bank select = {}", ch + 1, m.d2));
                true
            }
            0xb0 if m.d1 == 0 => true,
            0xc0 => {
                let prog_mode_any = !self.is_perf() && self.mirror_chan == mirror::ChanMode::Any;
                if ch != global && !prog_mode_any {
                    // a Part's program changed on the hardware: repoint the matching Part(s)
                    if !self.is_perf() || self.perf_bundle.is_some() {
                        self.mirror_log_push(format!("ch {}: Program change {} on a part channel (not applied)", ch + 1, m.d1));
                        return true;
                    }
                    let mut hit = Vec::new();
                    for p in 0..slots::PARTS {
                        let b = perf::part_off(p);
                        if self.perf_body[b + perf::rel::CHAN] == ch {
                            if let Some(bk) = self.rx_bank[ch as usize].filter(|v| (5..=8).contains(v)) {
                                self.perf_body[b + perf::rel::PROG_BANK] = bk;
                            }
                            self.perf_body[b + perf::rel::PROG_NUM] = m.d1;
                            hit.push(p);
                        }
                    }
                    if hit.is_empty() {
                        self.mirror_log_push(format!("ch {}: Program change {} - no Part uses this channel", ch + 1, m.d1));
                    } else {
                        if let Some(sel) = self.pstate.part.filter(|p| hit.contains(p)) {
                            self.load_part_program(sel);
                        }
                        let names: Vec<String> = hit.iter().map(|p| (p + 1).to_string()).collect();
                        let b = perf::part_off(hit[0]);
                        let id = perf::prog_id(self.perf_body[b + perf::rel::PROG_BANK], self.perf_body[b + perf::rel::PROG_NUM]);
                        self.mirror_log_push(format!("ch {}: Part {} now uses {id}", ch + 1, names.join(", ")));
                    }
                    return true;
                }
                let bank = self.rx_bank[ch as usize].unwrap_or(if self.is_perf() { 1 } else { self.bank });
                let idx = m.d1;
                match bank {
                    5..=8 => match self.find_program(bank, idx) {
                        Some(i) => {
                            if self.stash.is_some() {
                                self.mirror_log_push(format!("ch {}: Program {} skipped (compare is active)", ch + 1, perf::prog_id(bank, idx)));
                            } else {
                                self.mode = Mode::Program;
                                self.select(i);
                                self.mirror_log_push(format!("ch {}: Program {} loaded", ch + 1, perf::prog_id(bank, idx)));
                            }
                        }
                        None => self.mirror_log_push(format!("ch {}: Program {} not in the program list", ch + 1, perf::prog_id(bank, idx))),
                    },
                    1 | 2 => match self.perfs.iter().position(|p| p.bank == bank && p.idx == idx && p.bundle.is_none()) {
                        Some(i) => {
                            self.mode = Mode::Performance;
                            self.select_perf(i);
                            self.mirror_log_push(format!("ch {}: Performance {} loaded", ch + 1, perf::perf_id(bank, idx)));
                        }
                        None => self.mirror_log_push(format!("ch {}: Performance {} not in the list", ch + 1, perf::perf_id(bank, idx))),
                    },
                    b => self.mirror_log_push(format!("ch {}: Program change {} in bank {b} (not a stored program/performance bank)", ch + 1, idx)),
                }
                true
            }
            _ => false,
        }
    }

    fn mirror_msg(&mut self, m: mirror::ChannelMsg) {
        if !self.mirror_on {
            return;
        }
        let global = self.globals.as_ref().map(|g| g.body[0] & 0x0f).unwrap_or(self.send_chan);
        if self.mirror_patch_select(m, global) {
            return;
        }
        let part_chan = self.pstate.part.map(|p| self.perf_body[perf::part_off(p) + perf::rel::CHAN]);
        let Some(accept) = mirror::accept_for(self.mirror_chan, global, part_chan, self.is_perf()) else {
            if matches!(m.status & 0xf0, 0xb0 | 0xc0) {
                if self.mirror_log.len() >= 40 {
                    self.mirror_log.pop_front();
                }
                self.mirror_log.push_back(format!("ch {} ignored (no part selected)", m.chan() + 1));
            }
            return;
        };
        let writable = self.stash.is_none();
        if let Some(line) = self.mirror.process(m, accept, &mut self.prog, &self.params, writable) {
            if self.mirror_log.len() >= 40 {
                self.mirror_log.pop_front();
            }
            let sep = if line.starts_with("ignored") { " " } else { ": " };
            self.mirror_log.push_back(format!("ch {}{sep}{line}", m.chan() + 1));
        }
    }

    fn effects_window(&mut self, ctx: &egui::Context) {
        if !self.show_fx {
            return;
        }
        let Some(g) = effects::group(self.pstate.fx) else { return };
        let part = self.pstate.fx_part.filter(|_| !g.program_only);
        let src = match part {
            Some(p) => format!("Part {} effects", p + 1),
            None => "Program effects".to_string(),
        };
        let ro = part.is_some() && self.perf_stash.is_some();
        let mut open = true;
        egui::Window::new(format!("Effects - {} ({src})", g.title))
            .id(egui::Id::new("fx_window"))
            .open(&mut open)
            .default_pos(egui::pos2(ctx.content_rect().right() - 380.0, 140.0))
            .show(ctx, |ui| {
                ui.add_enabled_ui(!ro, |ui| {
                    for &(name, off) in g.params {
                        let idx = part.map(|p| effects::part_index(p, off)).unwrap_or(off);
                        let buf = if part.is_some() { &mut self.perf_body } else { &mut self.prog };
                        let mut v = buf[idx];
                        ui.horizontal(|ui| {
                            ui.label(format!("{name} (0x{off:02x})"));
                            ui.add(egui::Slider::new(&mut v, 0..=127).show_value(false));
                            ui.add(egui::DragValue::new(&mut v).range(0..=127));
                        });
                        buf[idx] = v;
                    }
                });
                ui.weak("Other bytes of this effect are not decoded yet.");
            });
        self.show_fx = open;
    }

    fn arp_window(&mut self, ctx: &egui::Context) {
        if !self.show_arp {
            return;
        }
        let mut open = true;
        egui::Window::new("Arpeggiator")
            .id(egui::Id::new("arp_window"))
            .open(&mut open)
            .default_pos(egui::pos2(ctx.content_rect().right() - 380.0, 140.0))
            .show(ctx, |ui| {
                ui.add_enabled_ui(self.stash.is_none(), |ui| {
                    let mut on = arp::on_lit(self.prog[arp::ON]);
                    if ui.checkbox(&mut on, "On").changed() {
                        self.prog[arp::ON] = if on { 72 } else { 73 };
                    }
                    let mut latch = arp::latch_lit(self.prog[arp::LATCH]);
                    if ui.checkbox(&mut latch, "Latch").changed() {
                        self.prog[arp::LATCH] = latch as u8;
                    }
                    let mut mute = arp::mute_lit(self.prog[arp::MUTE]);
                    if ui.checkbox(&mut mute, "Mute").changed() {
                        self.prog[arp::MUTE] = arp::mute_toggle(self.prog[arp::MUTE]);
                    }
                    for (label, off, opts) in arp::ENUMS {
                        let cur = self.prog[off];
                        ui.horizontal(|ui| {
                            ui.label(format!("{label} (0x{off:02x})"));
                            let txt = opts.iter().find(|(v, _)| *v == cur).map(|(_, n)| n.to_string()).unwrap_or(format!("raw {cur}"));
                            egui::ComboBox::from_id_salt(("arp", off)).selected_text(txt).show_ui(ui, |ui| {
                                for (v, n) in opts {
                                    if ui.selectable_label(cur == *v, *n).clicked() {
                                        self.prog[off] = *v;
                                    }
                                }
                            });
                        });
                    }
                    ui.horizontal(|ui| {
                        ui.label(format!("Arp setting (122..127) (0x{:02x})", arp::SETTING));
                        let mut v = self.prog[arp::SETTING];
                        ui.add(egui::DragValue::new(&mut v).range(0..=127));
                        self.prog[arp::SETTING] = v;
                    });
                });
                ui.weak("Arp speed is the global clock rate and is not in the program dump.");
            });
        self.show_arp = open;
    }

    /// Generated parameter menu (oscillator / filter / LFO / envelope) editing the current program.
    fn menu_window(&mut self, ctx: &egui::Context, key: &'static str) -> bool {
        let n = match key {
            "osc" => self.pstate.osc.min(2) + 1,
            "lfo" => self.pstate.lfo + 1,
            "env" => self.pstate.env + 1,
            _ => 1,
        };
        let title = match key {
            "osc" => format!("Oscillator {n} menu"),
            "filter" => "Filter menu".to_string(),
            "lfo" => format!("LFO {n} menu"),
            _ => format!("Envelope {n} menu"),
        };
        let rows = menus::rows_for(key, n, &self.params);
        let mut open = true;
        egui::Window::new(title)
            .id(egui::Id::new(("menu_window", key)))
            .open(&mut open)
            .default_pos(egui::pos2(ctx.content_rect().right() - 460.0, 120.0))
            .default_size([430.0, 420.0])
            .show(ctx, |ui| {
                ui.add_enabled_ui(self.stash.is_none(), |ui| {
                    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                        for r in rows {
                            let cur = self.prog[r.off];
                            ui.horizontal(|ui| {
                                let l = ui.label(format!("{} (0x{:02x})", r.label, r.off));
                                if !r.hover.is_empty() {
                                    l.on_hover_text(&r.hover);
                                }
                                match &r.kind {
                                    menus::RowKind::Combo(opts) => {
                                        let txt = opts.iter().find(|(v, _)| *v == cur).map(|(_, n)| n.clone()).unwrap_or(format!("raw {cur}"));
                                        egui::ComboBox::from_id_salt(("mrow", key, r.off)).selected_text(txt).show_ui(ui, |ui| {
                                            for (v, name) in opts {
                                                if ui.selectable_label(cur == *v, name).clicked() {
                                                    self.prog[r.off] = *v;
                                                }
                                            }
                                        });
                                    }
                                    menus::RowKind::Slider => {
                                        let mut v = cur;
                                        let a = ui.add(egui::Slider::new(&mut v, r.min as u8..=r.max as u8).show_value(false));
                                        let b = ui.add(egui::DragValue::new(&mut v).range(r.min as u8..=r.max as u8));
                                        if !r.hover.is_empty() {
                                            b.clone().on_hover_text(&r.hover);
                                        }
                                        if a.changed() || b.changed() {
                                            self.prog[r.off] = v;
                                        }
                                    }
                                }
                            });
                        }
                    });
                });
            });
        open
    }

    fn trig_window(&mut self, ctx: &egui::Context) {
        if !self.show_trig {
            return;
        }
        let mut open = true;
        egui::Window::new("Envelope triggering")
            .id(egui::Id::new("trig_window"))
            .open(&mut open)
            .default_pos(egui::pos2(ctx.content_rect().right() - 330.0, 140.0))
            .show(ctx, |ui| {
                ui.add_enabled_ui(self.stash.is_none(), |ui| {
                    let raw = self.prog[menus::TRIG_BYTE];
                    match menus::trig_get(raw) {
                        Some(mut single) => {
                            let mut changed = false;
                            for (i, name) in ["Env 1 (amp)", "Env 2", "Env 3"].iter().enumerate() {
                                ui.horizontal(|ui| {
                                    ui.label(*name);
                                    changed |= ui.selectable_value(&mut single[i], true, "Single").clicked();
                                    changed |= ui.selectable_value(&mut single[i], false, "Multiple").clicked();
                                });
                            }
                            if changed {
                                self.prog[menus::TRIG_BYTE] = menus::trig_set(single);
                            }
                        }
                        None => {
                            ui.label("Unexpected value - set it with the raw field.");
                        }
                    }
                    ui.horizontal(|ui| {
                        ui.label(format!("raw byte 0x{:02x}", menus::TRIG_BYTE));
                        let mut v = self.prog[menus::TRIG_BYTE];
                        if ui.add(egui::DragValue::new(&mut v).range(0..=127)).changed() {
                            self.prog[menus::TRIG_BYTE] = v;
                        }
                    });
                    ui.weak("34 + 4*env1 + 2*env2 + env3 (1 = single trigger)");
                });
            });
        self.show_trig = open;
    }

    fn windows(&mut self, ctx: &egui::Context) {
        self.update_fx_part();
        let keys: Vec<&'static str> = self.menus_open.clone();
        let mut still = Vec::new();
        for k in keys {
            if self.menu_window(ctx, k) {
                still.push(k);
            }
        }
        self.menus_open = still;
        self.trig_window(ctx);
        self.effects_window(ctx);
        self.arp_window(ctx);
        // MIDI connect
        if self.show_midi {
            let mut open = true;
            let mut close = false;
            egui::Window::new("MIDI").open(&mut open).collapsible(false).show(ctx, |ui| {
                ui.label(format!(
                    "Connected: in {} / out {}",
                    self.midi.conn_in.as_deref().unwrap_or("none"),
                    self.midi.conn_out.as_deref().unwrap_or("none")
                ));
                if self.midi.inputs.is_empty() && self.midi.outputs.is_empty() {
                    ui.colored_label(Color32::from_rgb(0xf3, 0xa2, 0x4b), "No MIDI ports found - check the interface is connected, then press Refresh");
                }
                let name = |v: &Vec<String>, s: Option<usize>| {
                    s.and_then(|i| v.get(i).cloned()).unwrap_or_else(|| "(none)".into())
                };
                let (old_in, old_out) = (self.sel_in, self.sel_out);
                ui.horizontal(|ui| {
                    ui.label("Input ");
                    egui::ComboBox::from_id_salt("min").selected_text(name(&self.midi.inputs, self.sel_in)).show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.sel_in, None, "(none)");
                        for (i, n) in self.midi.inputs.iter().enumerate() {
                            ui.selectable_value(&mut self.sel_in, Some(i), n);
                        }
                    });
                });
                ui.horizontal(|ui| {
                    ui.label("Output");
                    egui::ComboBox::from_id_salt("mout").selected_text(name(&self.midi.outputs, self.sel_out)).show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.sel_out, None, "(none)");
                        for (i, n) in self.midi.outputs.iter().enumerate() {
                            ui.selectable_value(&mut self.sel_out, Some(i), n);
                        }
                    });
                });
                // picking a port connects at once and is remembered for the next start
                if (self.sel_in, self.sel_out) != (old_in, old_out) {
                    match self.midi.connect(self.sel_in, self.sel_out, ctx.clone()) {
                        Ok(s) => {
                            self.ports_cfg.input = self.midi.conn_in.clone();
                            self.ports_cfg.output = self.midi.conn_out.clone();
                            self.ports_cfg.save();
                            self.set_status(s, Some(true));
                            self.start_live();
                        }
                        Err(e) => self.set_status(format!("MIDI error: {e}"), Some(false)),
                    }
                }
                ui.weak("Ports are found and reconnected automatically; Live edit starts as soon as the Nova is connected.");
                {
                    let (c, x, ago) = self.midi.stats.snapshot();
                    ui.label(format!("Received: {c} channel msgs, {x} sysex, last {}", midi::ago_text(ago)));
                }
                {
                    let running = self.scan.as_ref().is_some_and(|s| !s.finished.load(std::sync::atomic::Ordering::SeqCst));
                    let ready = self.midi.conn_in.is_some() && !running;
                    if ui.add_enabled(ready, egui::Button::new("Find the Nova's output port"))
                        .on_hover_text("Sends one standard Identity Request to each output port in turn and shows which one the Nova answers. Needs the input connected. Nothing else is sent.")
                        .clicked()
                    {
                        self.scan = Some(self.midi.scan_outputs(ui.ctx().clone()));
                    }
                    if let Some(s) = &self.scan {
                        for l in s.lines.lock().unwrap().iter() {
                            ui.monospace(l);
                        }
                        if running {
                            ui.weak("Scanning...");
                        } else if self.midi.conn_in.is_none() {
                            ui.weak("Connect the input first.");
                        }
                    }
                }
                if ui
                    .add_enabled(self.midi.has_output() && self.send_enabled, egui::Button::new("Unlock banks C/D..."))
                    .on_hover_text("Sends the firmware's hidden unlock command. Asks for confirmation first.")
                    .clicked()
                {
                    self.confirm_unlock = true;
                    self.unlock_ack = false;
                }
                if ui.add_enabled(self.midi.has_output(), egui::Button::new("Test connection (identity request)"))
                    .on_hover_text("Sends the standard MIDI Identity Request (F0 7E 7F 06 01 F7). The Nova answers it; no sound or patch data is changed.")
                    .clicked()
                {
                    match self.midi.send(&[0xf0, 0x7e, 0x7f, 0x06, 0x01, 0xf7]) {
                        Ok(_) => self.set_status("Identity request sent - watch for a reply (needs the input connected too)", None),
                        Err(e) => self.set_status(format!("Send failed: {e}"), Some(false)),
                    }
                }
                {
                    ui.weak(format!("Live edit is {} ({} message(s) sent). In Perf mode, edits go to the selected Part on its MIDI channel. Only the Nova's edit buffer changes; nothing is stored.", if self.live_on { "on" } else { "waiting for the Nova" }, self.live_sent));
                    egui::CollapsingHeader::new("Live edit log (last 40)").default_open(false).show(ui, |ui| {
                        for l in &self.live_log {
                            ui.monospace(l);
                        }
                    });
                }
                ui.checkbox(&mut self.mirror_on, "Mirror hardware (receive-only)")
                    .on_hover_text("Applies incoming CC/NRPN from the Nova to the current program. Sends nothing.");
                ui.horizontal(|ui| {
                    ui.label("Mirror channel");
                    self.mirror_chan_combo(ui, "mirror_chan");
                });
                egui::CollapsingHeader::new("Mirror log (last 40)").default_open(false).show(ui, |ui| {
                    egui::ScrollArea::vertical().max_height(160.0).stick_to_bottom(true).show(ui, |ui| {
                        for l in &self.mirror_log {
                            ui.monospace(l);
                        }
                    });
                });
                ui.horizontal(|ui| {
                    ui.label("SysEx channel");
                    ui.add(egui::DragValue::new(&mut self.send_chan).range(0..=15));
                    ui.weak("(15 = the hardware's global channel 16 in the capture)");
                });
                ui.separator();
                ui.label(format!("{} input port(s), {} output port(s) found.", self.midi.inputs.len(), self.midi.outputs.len()));
                ui.weak("Dumps, sending and file save/load are in the Dump menu (panel button).");
            });
            self.show_midi = open && !close;
        }
        self.dump_window(ctx);

        // send performance set confirmation
        if self.confirm_set_send {
            let msgs = self.set_send_messages();
            let np = self.saved_slots.len();
            let nf = msgs.len().saturating_sub(np) / 2;
            let secs = msgs.len() as u64 * midi::SEND_GAP_MS / 1000;
            let mut go = false;
            let mut cancel = false;
            egui::Window::new("Send performance set to the Nova").collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
                ui.set_max_width(480.0);
                ui.label(format!("{np} programs (banks C/D) and {nf} performances (A000-A041) = {} SysEx messages, about {secs} seconds or more (slow byte pacing), on SysEx channel {}.", msgs.len(), self.send_chan));
                ui.colored_label(Color32::from_rgb(0xf3, 0xa2, 0x4b), "These are bank dumps: they overwrite the Nova's stored performances A000-A041 and the listed C/D program slots. They are stored, not just edit-buffer changes. Your current Nova contents there are lost unless backed up.");
                ui.label("Needs Memory protect = Off and Sysex reception = Normal on the Nova (as in your Global pages). You can cancel while it sends; the slots already written stay written.");
                ui.checkbox(&mut self.set_send_ack, "I have a current full backup and want to overwrite those slots");
                ui.horizontal(|ui| {
                    go = ui.add_enabled(self.set_send_ack && !msgs.is_empty(), egui::Button::new("Send set")).clicked();
                    cancel = ui.button("Cancel").clicked();
                });
            });
            if go {
                self.live_on = false;
                match self.midi.start_send(msgs, ctx.clone()) {
                    Ok(job) => {
                        self.set_status(format!("Sending {} message(s)...", job.total), None);
                        self.send_note = Some("Sent the performance set to the Nova (stored in A000-A041 and C/D). Check the Nova.".into());
                        self.send_job = Some(job);
                    }
                    Err(e) => self.set_status(format!("Send failed: {e}"), Some(false)),
                }
            }
            if go || cancel {
                self.confirm_set_send = false;
                self.set_send_ack = false;
            }
        }

        // fill performances confirmation
        if self.confirm_fill {
            let mut go = false;
            let mut cancel = false;
            egui::Window::new("Fill performances from A000").collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
                ui.set_max_width(460.0);
                ui.label("This copies performance A000 into A001-A041 in the app's list, replacing what the app holds there. Each copy gets a number added to its name (for example \"Name 02\") and its own six programs in banks C/D: A000 uses C000-C005, A001 uses C006-C011, and so on.");
                ui.label("Only the app's list changes. Nothing is sent to the Nova; your backup files are not modified. Use Export performance set to write it out.");
                ui.horizontal(|ui| {
                    go = ui.button("Copy").clicked();
                    cancel = ui.button("Cancel").clicked();
                });
            });
            if go {
                self.fill_performances_from_first();
            }
            if go || cancel {
                self.confirm_fill = false;
            }
        }

        // unlock banks C/D confirmation
        if self.confirm_unlock {
            let chan = self.globals.as_ref().map(|g| g.body[0] & 0x0f).unwrap_or(self.send_chan);
            let mut go = false;
            let mut cancel = false;
            egui::Window::new("Unlock banks C/D").collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
                ui.set_max_width(460.0);
                ui.label("This sends a hidden firmware command: Controller 99 (NRPN MSB) = 127, then SysEx type 0x1D. If accepted, the Nova's persistent settings change so that program banks C and D (and performance bank B) become available, and the firmware copies A to C, B to D and performance A to B.");
                ui.colored_label(Color32::from_rgb(0xf3, 0xa2, 0x4b), "Found by reading the firmware; never tested on hardware. It is not known whether your Nova has memory for banks C/D, and no command to undo it is known. A full backup should exist first.");
                ui.label(format!("Sent on MIDI channel {} (the global channel).", chan + 1));
                ui.checkbox(&mut self.unlock_ack, "I have a current full backup and want to do this");
                ui.horizontal(|ui| {
                    go = ui.add_enabled(self.unlock_ack, egui::Button::new("Send unlock")).clicked();
                    cancel = ui.button("Cancel").clicked();
                });
            });
            if go {
                self.live_on = false; // nothing may be sent between the two messages
                let (cc, sx) = sysex::unlock_banks_messages(chan);
                let r = self.midi.send(&cc).and_then(|_| self.midi.send(&sx));
                match r {
                    Ok(_) => self.set_status("Unlock sent (CC99=127 then SysEx 1D). Press Bank on the Nova and look for banks C/D; tell me what it shows.", Some(true)),
                    Err(e) => self.set_status(format!("Unlock send failed: {e}"), Some(false)),
                }
            }
            if go || cancel {
                self.confirm_unlock = false;
                self.unlock_ack = false;
            }
        }

        // send confirmation
        if self.confirm_send {
            let plan = self.send_plan();
            let mut send = false;
            let mut cancel = false;
            egui::Window::new("Send to Nova").collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
                if !self.is_perf() {
                    ui.horizontal(|ui| {
                        ui.label("Target part (0-5 = parts 1-6, 8, 9 = current):");
                        ui.add(egui::DragValue::new(&mut self.part).range(0..=9));
                    });
                }
                match &plan {
                    Ok((_, desc)) => {
                        ui.label(desc);
                    }
                    Err(e) => {
                        ui.colored_label(Color32::from_rgb(0xf3, 0xa2, 0x4b), format!("Cannot send: {e}"));
                    }
                }
                ui.label("This only changes the Nova's EDIT BUFFER. Press Write on the Nova to store it.");
                ui.horizontal(|ui| {
                    send = ui.add_enabled(plan.is_ok(), egui::Button::new("Send")).clicked();
                    cancel = ui.button("Cancel").clicked();
                });
            });
            if send {
                if let Ok((msgs, _)) = plan {
                    match self.midi.start_send(msgs, ctx.clone()) {
                        Ok(job) => {
                            self.set_status(format!("Sending {} message(s)...", job.total), None);
                            self.send_note = None;
                            self.send_job = Some(job);
                        }
                        Err(e) => self.set_status(format!("Send failed: {e}"), Some(false)),
                    }
                }
            }
            if send || cancel {
                self.confirm_send = false;
            }
        }

        // send progress
        if let Some(job) = &self.send_job {
            use std::sync::atomic::Ordering;
            let (done, total) = (job.done.load(Ordering::SeqCst), job.total);
            if job.finished.load(Ordering::SeqCst) {
                let err = job.error.lock().unwrap().clone();
                let cancelled = job.cancel.load(Ordering::SeqCst);
                self.send_job = None;
                match err {
                    Some(e) => self.set_status(format!("Send failed after {done} of {total} message(s): {e}"), Some(false)),
                    None if cancelled => self.set_status(format!("Send cancelled after {done} of {total} message(s)"), None),
                    None => match self.send_note.take() {
                        Some(n) => self.set_status(n, Some(true)),
                        None => self.set_status(format!("Sent {total} message(s) to the Nova's edit buffer (press Write on the Nova to store)"), Some(true)),
                    },
                }
            } else {
                let cancel = job.cancel.clone();
                egui::Window::new("Sending to Nova").collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
                    ui.label(format!("Sent {done} of {total} message(s)"));
                    ui.add(egui::ProgressBar::new(done as f32 / total.max(1) as f32));
                    if ui.button("Cancel").clicked() {
                        cancel.store(true, Ordering::SeqCst);
                    }
                });
                ctx.request_repaint_after(std::time::Duration::from_millis(100));
            }
        }

        // assign window
        if let Some(mut w) = self.assign_win.take() {
            let mut keep = true;
            let mut apply = false;
            let mut remove = false;
            let mut learn = false;
            let mut open = true;
            egui::Window::new(format!("Assign control: {}", w.id)).open(&mut open).collapsible(false).show(ctx, |ui| {
                ui.label("Program byte offset (e.g. 0x3a or 58), with optional min / max:");
                ui.horizontal(|ui| {
                    ui.label("Offset");
                    ui.add(egui::TextEdit::singleline(&mut w.off).desired_width(70.0));
                    ui.label("min");
                    ui.add(egui::TextEdit::singleline(&mut w.min).desired_width(40.0));
                    ui.label("max");
                    ui.add(egui::TextEdit::singleline(&mut w.max).desired_width(40.0));
                });
                ui.horizontal(|ui| {
                    apply = ui.button("Apply").clicked();
                    remove = ui.button("Remove").clicked();
                    learn = ui.button("Learn").on_hover_text("Turn that control on the Nova, then have it transmit the program").clicked();
                    if ui.button("Cancel").clicked() {
                        keep = false;
                    }
                });
            });
            if !open {
                keep = false;
            }
            if apply {
                let t = w.off.trim();
                let off = if let Some(h) = t.strip_prefix("0x").or(t.strip_prefix("0X")) {
                    usize::from_str_radix(h, 16).ok()
                } else {
                    t.parse::<usize>().ok()
                };
                match (off, w.min.trim().parse::<i32>(), w.max.trim().parse::<i32>()) {
                    (Some(off), Ok(min), Ok(max)) if off < N && min <= max => {
                        self.assign.insert(w.id.clone(), panel::Assign { off, min, max });
                        self.save_assign();
                        self.info_id = Some(w.id.clone());
                        self.readout_msg = None;
                        keep = false;
                    }
                    _ => self.set_status("Offset must be 0..285 and min <= max", Some(false)),
                }
            } else if remove {
                self.assign.remove(&w.id);
                self.save_assign();
                keep = false;
            } else if learn {
                let msg = if self.last_rx.is_some() {
                    format!("Learning \"{}\": now change that control on the Nova and transmit the program", w.id)
                } else {
                    format!("Learning \"{}\": first transmit the program once from the Nova (baseline), then change the control and transmit again", w.id)
                };
                self.learn = Some(w.id.clone());
                self.set_status(msg, Some(true));
                keep = false;
            }
            if keep {
                self.assign_win = Some(w);
            }
        }
    }
}

/// "Dump everything" run: all programs, then all performances, each ended by a quiet period.
struct DumpAll {
    stage: u8,
    started: std::time::Instant,
    last_rx: std::time::Instant,
    got: usize,
    progs: usize,
    perfs: usize,
    prev_collect: bool,
}

const DUMP_QUIET_SECS: u64 = 3;
const DUMP_NO_ANSWER_SECS: u64 = 12;

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        for m in self.midi.poll() {
            self.handle_rx(m);
        }
        for m in self.midi.poll_channel() {
            self.mirror_msg(m);
        }
        if self.live_on {
            // changes that came from the hardware or a patch load must not be echoed back
            self.live_resync();
        }
        let ctx = ui.ctx().clone();
        if self.midi.conn_in.is_some() {
            // keep polling even when the window is idle or in the background
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }

        egui::Panel::bottom("status").show(ui, |ui| {
            let (t, ok) = &self.status;
            let c = match ok {
                Some(true) => Color32::from_rgb(0x4c, 0xc3, 0x8a),
                Some(false) => Color32::from_rgb(0xf3, 0xa2, 0x4b),
                None => ui.visuals().weak_text_color(),
            };
            let mut s = t.clone();
            if let Some(l) = &self.learn {
                s.push_str(&format!("   [learning: {l}]"));
            }
            ui.horizontal(|ui| {
                // iPad: keep clear of the rounded screen corner
                #[cfg(target_os = "ios")]
                ui.add_space(22.0);
                ui.label(RichText::new(s).color(c));
                let recent = self.midi.stats.snapshot().2.is_some_and(|a| a < 3000);
                if self.mirror_on && self.midi.conn_in.is_none() {
                    ui.colored_label(Color32::from_rgb(0xf3, 0xa2, 0x4b), "   Mirror on - not connected");
                } else if !self.mirror_on && self.midi.conn_in.is_some() && recent {
                    ui.colored_label(Color32::from_rgb(0xf3, 0xa2, 0x4b), "   messages arriving - tick Mirror");
                }
            });
            // iPad: keep the line above the home indicator
            #[cfg(target_os = "ios")]
            ui.add_space(14.0);
        });

        let mut clicked: Option<usize> = None;
        let perf_mode = self.is_perf();
        if self.show_list {
        egui::Panel::left("list").default_size(240.0).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.strong(if perf_mode { "Performances" } else { "Programs" });
                ui.label(if perf_mode { self.perfs.len() } else { self.programs.len() }.to_string());
                ui.add(egui::TextEdit::singleline(&mut self.filter).id(egui::Id::new("filter_edit")).hint_text("filter...").desired_width(f32::INFINITY));
            });
            ui.separator();
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                let mut rows: Vec<(usize, String, String, bool)> = Vec::new();
                if perf_mode {
                    let f = self.filter.to_lowercase();
                    for (i, p) in self.perfs.iter().enumerate() {
                        let nm = p.name();
                        if f.is_empty() || nm.to_lowercase().contains(&f) {
                            rows.push((i, if p.bundle.is_some() { "buffer".to_string() } else { perf::perf_id(p.bank, p.idx) }, nm, self.perf_cur == Some(i)));
                        }
                    }
                } else {
                    for (i, p) in self.programs.iter().enumerate() {
                        if self.matches_filter(p) {
                            let tag = p.bank.map(|b| perf::prog_id(b, p.idx)).unwrap_or(p.idx.to_string());
                            rows.push((i, tag, p.name(), self.cur == Some(i)));
                        }
                    }
                }
                for (i, tag, nm, sel) in rows {
                    let r = ui.selectable_label(sel, format!("{tag:>6}  {}", if nm.is_empty() { "(blank)" } else { &nm }));
                    if sel && self.scroll_to_sel {
                        r.scroll_to_me(None);
                    }
                    if r.clicked() {
                        clicked = Some(i);
                    }
                }
                self.scroll_to_sel = false;
            });
        });
        }
        if let Some(i) = clicked {
            if perf_mode { self.select_perf(i) } else { self.select(i) }
        }

        egui::CentralPanel::default().show(ui, |ui| self.panel_view(ui));

        self.sync_embedded();
        self.ports_cfg.mirror_on = self.mirror_on;
        self.ports_cfg.mirror_chan = self.mirror_chan;
        if !self.is_perf() && self.cur.is_some() {
            self.ports_cfg.last_bank = Some(self.bank);
            self.ports_cfg.last_idx = Some(self.idx);
        }
        if self.ports_cfg != self.ports_saved {
            self.ports_cfg.save();
            self.ports_saved = self.ports_cfg.clone();
        }
        self.midi_watch(&ctx);
        self.windows(&ctx);
        self.live_tick();
        self.dump_all_tick(&ctx);
        self.library_tick();
    }
}

/// Desktop entry point (called by src/main.rs).
pub fn run(cli: Vec<PathBuf>) -> eframe::Result {
    let opts = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1500.0, 960.0]).with_title("Nova Editor"),
        ..Default::default()
    };
    eframe::run_native("Nova Editor", opts, Box::new(move |cc| Ok(Box::new(App::new(cc, cli)))))
}

/// iPadOS: append `log` records (warn and up) to Documents/novaedit_log.txt.
#[cfg(target_os = "ios")]
struct FileLog(std::path::PathBuf);
#[cfg(target_os = "ios")]
impl log::Log for FileLog {
    fn enabled(&self, m: &log::Metadata) -> bool {
        m.level() <= log::Level::Info
    }
    fn log(&self, r: &log::Record) {
        if self.enabled(r.metadata()) {
            use std::io::Write;
            if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&self.0) {
                let _ = writeln!(f, "{} {}: {}", r.level(), r.target(), r.args());
            }
        }
    }
    fn flush(&self) {}
}

/// iPadOS entry point: the Xcode wrapper's main() calls this; it never returns.
#[cfg(target_os = "ios")]
#[unsafe(no_mangle)]
pub extern "C" fn start_nova_editor() {
    // errors and panics go to Documents/novaedit_error.txt (no console on the iPad)
    if let Some(d) = files::documents_dir() {
        let _ = std::fs::create_dir_all(&d);
        let _ = std::fs::write(d.join("novaedit_started.txt"), "started");
        eprintln!("novaedit: started, documents at {}", d.display());
        if log::set_boxed_logger(Box::new(FileLog(d.join("novaedit_log.txt")))).is_ok() {
            log::set_max_level(log::LevelFilter::Info);
        }
    }
    let log = files::documents_dir().map(|d| d.join("novaedit_error.txt"));
    let plog = log.clone();
    std::panic::set_hook(Box::new(move |info| {
        if let Some(p) = &plog {
            let _ = std::fs::write(p, format!("panic: {info}\n{}", std::backtrace::Backtrace::force_capture()));
        }
    }));
    // ask the GPU only for the limits it reports (the Simulator's Metal is below wgpu's defaults)
    let mut setup = eframe::egui_wgpu::WgpuSetupCreateNew::without_display_handle();
    setup.device_descriptor = std::sync::Arc::new(|adapter| eframe::wgpu::DeviceDescriptor {
        label: Some("egui wgpu device"),
        required_limits: adapter.limits(),
        ..Default::default()
    });
    let mut opts = eframe::NativeOptions::default();
    opts.wgpu_options.wgpu_setup = eframe::egui_wgpu::WgpuSetup::CreateNew(setup);
    if let Err(e) = eframe::run_native("Nova Editor", opts, Box::new(move |cc| Ok(Box::new(App::new(cc, Vec::new()))))) {
        if let Some(p) = &log {
            let _ = std::fs::write(p, format!("eframe error: {e:?}"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn compare_keeps_edits() {
        let orig = vec![1u8; N];
        let mut prog = orig.clone();
        prog[0x40] = 99;
        let mut stash = None;
        compare_toggle(&mut prog, &orig, &mut stash);
        assert_eq!(prog, orig);
        assert!(stash.is_some());
        compare_toggle(&mut prog, &orig, &mut stash);
        assert_eq!(prog[0x40], 99);
        assert!(stash.is_none());
    }
}
