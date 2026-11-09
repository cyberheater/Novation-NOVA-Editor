//! "Dump" window: retrieve from / send to the Nova, and save / load local files.
use crate::{App, midi, sysex};
use eframe::egui::{self, Color32};

/// A confirmed-before-sending batch of bank dumps.
pub struct BankSend {
    pub label: String,
    pub msgs: Vec<Vec<u8>>,
}

const WARN: Color32 = Color32::from_rgb(0xf3, 0xa2, 0x4b);
const BANKS: [(u8, &str); 4] = [(5, "A"), (6, "B"), (7, "C"), (8, "D")];

const BOX: egui::Vec2 = egui::vec2(240.0, 64.0);
const GAP: [f32; 2] = [14.0, 14.0];

/// Section title.
fn heading(ui: &mut egui::Ui, t: &str) {
    ui.label(egui::RichText::new(t).size(24.0).strong());
    ui.add_space(6.0);
}

/// Large, prominent box button (half width).
fn big(ui: &mut egui::Ui, enabled: bool, label: &str) -> egui::Response {
    boxed(ui, enabled, label, BOX)
}

fn boxed(ui: &mut egui::Ui, enabled: bool, label: &str, size: egui::Vec2) -> egui::Response {
    let b = egui::Button::new(egui::RichText::new(label).size(20.0).strong())
        .corner_radius(8.0)
        .stroke(egui::Stroke::new(1.5, egui::Color32::from_rgb(0x8a, 0x9c, 0xd8)));
    ui.add_enabled_ui(enabled, |ui| ui.add_sized(size, b)).inner
}

/// Full-width box button under a 2x2 grid.
fn wide(ui: &mut egui::Ui, enabled: bool, label: &str) -> egui::Response {
    ui.add_space(4.0);
    let w = BOX.x * 2.0 + GAP[0];
    boxed(ui, enabled, label, egui::vec2(w, BOX.y))
}

fn bank_letter(b: u8) -> &'static str {
    BANKS.iter().find(|(x, _)| *x == b).map(|(_, l)| *l).unwrap_or("?")
}

impl App {
    /// Type-2 messages for library programs (one bank, or all banks), ordered by bank and index.
    fn lib_program_msgs(&self, bank: Option<u8>) -> Vec<Vec<u8>> {
        let chan = self.send_chan;
        let mut v: Vec<(u8, u8, Vec<u8>)> = self
            .programs
            .iter()
            .filter_map(|p| {
                let b = p.bank?;
                (bank.is_none_or(|x| x == b) && p.body.len() >= crate::N).then(|| (b, p.idx, sysex::build_type2(&p.body, b, p.idx, chan)))
            })
            .collect();
        v.sort_by_key(|t| (t.0, t.1));
        v.into_iter().map(|t| t.2).collect()
    }

    /// Type-7 messages for stored library performances, each followed by its companion.
    fn lib_perf_msgs(&self) -> Vec<Vec<u8>> {
        let chan = self.send_chan;
        let mut v: Vec<&sysex::Performance> = self.perfs.iter().filter(|p| p.bundle.is_none() && p.body.len() >= crate::perf::PERF_LEN).collect();
        v.sort_by_key(|p| (p.bank, p.idx));
        let mut out = Vec::new();
        for p in v {
            out.push(sysex::build_type7(&p.body, p.bank, p.idx, chan));
            out.push(sysex::build_perf_companion(p.bank, p.idx, chan));
        }
        out
    }

    fn save_bytes(&mut self, name: &str, msgs: Vec<Vec<u8>>) {
        if msgs.is_empty() {
            self.set_status("Nothing to save: the library has none of those", Some(false));
            return;
        }
        let Some(path) = crate::files::FileDialog::new().set_file_name(name).add_filter("SysEx", &["syx"]).save_file() else { return };
        let data: Vec<u8> = msgs.concat();
        match std::fs::write(&path, &data) {
            Ok(_) => self.set_status(format!("Saved {} ({} messages, {} bytes)", path.display(), msgs.len(), data.len()), Some(true)),
            Err(e) => self.set_status(format!("Save failed: {e}"), Some(false)),
        }
    }

    /// Load .syx/.mid files and merge their programs and performances into the library slot by slot.
    fn merge_files(&mut self) {
        let Some(paths) = crate::files::FileDialog::new().add_filter("MIDI / SysEx", &["mid", "midi", "syx"]).pick_files() else { return };
        let (mut np, mut nf, mut skipped) = (0, 0, 0);
        for path in paths {
            let Ok(d) = std::fs::read(&path) else { continue };
            let (progs, perfs) = sysex::parse_all(&d);
            for p in progs {
                let Some(b) = p.bank else {
                    skipped += 1;
                    continue;
                };
                let idx = p.idx;
                match self.find_program(b, idx) {
                    Some(i) => self.programs[i].body = p.body,
                    None => self.programs.push(p),
                }
                if b >= 7 && !self.saved_slots.contains(&(b, idx)) {
                    self.saved_slots.push((b, idx));
                }
                np += 1;
            }
            for p in perfs {
                if p.bundle.is_none() {
                    match self.perfs.iter().position(|q| q.bundle.is_none() && q.bank == p.bank && q.idx == p.idx) {
                        Some(i) => self.perfs[i].body = p.body,
                        None => self.perfs.push(p),
                    }
                } else {
                    self.perfs.push(p);
                }
                nf += 1;
            }
        }
        let extra = if skipped > 0 { format!(" ({skipped} part dumps without a bank skipped)") } else { String::new() };
        self.set_status(format!("Merged {np} programs and {nf} performances into the library{extra}"), Some(np + nf > 0));
    }

    pub fn dump_window(&mut self, ctx: &egui::Context) {
        if !self.show_dump {
            return;
        }
        let mut open = true;
        egui::Window::new("Dump").open(&mut open).collapsible(false).default_width(500.0).default_height(620.0).show(ctx, |ui| {
            let out = self.midi.has_output();
            let bl = bank_letter(self.dump_bank);
            if let Some(j) = self.send_job.as_ref() {
                ui.horizontal(|ui| {
                    ui.label(format!("Sending to the Nova: {} of {} messages", j.done.load(std::sync::atomic::Ordering::SeqCst), j.total));
                    if ui.button("Cancel send").clicked() {
                        j.cancel.store(true, std::sync::atomic::Ordering::SeqCst);
                    }
                });
                ui.separator();
            }

            egui::ScrollArea::vertical().auto_shrink([false, true]).show(ui, |ui| {
            // ---- retrieve ----
            heading(ui, "Retrieve from the Nova");
            egui::Grid::new("dump_rx").num_columns(2).spacing(GAP).show(ui, |ui| {
                for (i, (label, kind, collect, tip)) in [
                    ("Current prog", 3u8, false, "The current Part's program."),
                    ("Current perf", 8, false, "The current performance with its Part programs."),
                    ("All programs", 4, true, "Every program bank the Nova has (A-D once C/D are unlocked)."),
                    ("All perfs", 9, true, "Every stored performance."),
                ]
                .into_iter()
                .enumerate()
                {
                    if big(ui, out, label).on_hover_text(tip).clicked() {
                        if collect {
                            self.rx_collect = true;
                        }
                        match self.send_dump_request(kind) {
                            Ok(_) => self.set_status(format!("Asked the Nova for: {label}. Watch for the dump."), None),
                            Err(e) => self.set_status(format!("Send failed: {e}"), Some(false)),
                        }
                    }
                    if i % 2 == 1 {
                        ui.end_row();
                    }
                }
            });
            if let Some(d) = self.dump_all.as_ref() {
                let txt = format!("Retrieving {}: {} programs, {} perfs... (tap to cancel)", if d.stage == 0 { "programs" } else { "perfs" }, d.progs, d.perfs);
                if wide(ui, true, &txt).clicked() {
                    self.finish_dump_all("Dump everything cancelled".into(), false);
                }
            } else if wide(ui, out, "Everything").on_hover_text("All programs, then all performances (about 1-2 minutes).").clicked() {
                self.start_dump_all();
            }
            ui.checkbox(&mut self.rx_collect, "Also collect dumps sent from the Nova's own Sysex transmission menu");
            if self.rx_collect || self.rx_prog_n + self.rx_perf_n > 0 {
                ui.label(format!("Collected {} programs, {} performances", self.rx_prog_n, self.rx_perf_n));
            }

            // ---- send ----
            ui.add_space(14.0);
            heading(ui, "Send to the Nova");
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Program bank").size(17.0));
                egui::ComboBox::from_id_salt("dump_bank").selected_text(egui::RichText::new(bl).size(17.0)).show_ui(ui, |ui| {
                    for (b, l) in BANKS {
                        ui.selectable_value(&mut self.dump_bank, b, l);
                    }
                });
            });
            let can = out && self.send_enabled && self.send_job.is_none();
            let can_cur = can && (!self.is_perf() || self.perf_bundle.is_some());
            let n = self.lib_program_msgs(Some(self.dump_bank)).len();
            let nf = self.lib_perf_msgs().len() / 2;
            let na = self.lib_program_msgs(None).len();
            egui::Grid::new("dump_tx").num_columns(2).spacing(GAP).show(ui, |ui| {
                let cur = if self.is_perf() { "Current perf" } else { "Current prog" };
                if big(ui, can_cur, cur).on_hover_text("To the Nova's edit buffer (not stored), after a confirmation.").clicked() {
                    self.confirm_send = true;
                }
                if big(ui, can && n > 0, &format!("Bank {bl} ({n})")).clicked() {
                    self.bank_send = Some(BankSend { label: format!("program bank {bl}"), msgs: self.lib_program_msgs(Some(self.dump_bank)) });
                    self.bank_send_ack = false;
                }
                ui.end_row();
                if big(ui, can && na > 0, &format!("All programs ({na})")).clicked() {
                    self.bank_send = Some(BankSend { label: "all programs".into(), msgs: self.lib_program_msgs(None) });
                    self.bank_send_ack = false;
                }
                if big(ui, can && nf > 0, &format!("All perfs ({nf})")).clicked() {
                    self.bank_send = Some(BankSend { label: "all performances".into(), msgs: self.lib_perf_msgs() });
                    self.bank_send_ack = false;
                }
                ui.end_row();
            });
            if wide(ui, can && na + nf > 0, "Everything").clicked() {
                let mut msgs = self.lib_program_msgs(None);
                msgs.extend(self.lib_perf_msgs());
                self.bank_send = Some(BankSend { label: "everything (all programs and performances)".into(), msgs });
                self.bank_send_ack = false;
            }

            // ---- performance set ----
            ui.add_space(14.0);
            heading(ui, "Performance set (A000-A041)");
            let perf = self.is_perf();
            let have = !self.saved_slots.is_empty();
            egui::Grid::new("dump_set").num_columns(2).spacing(GAP).show(ui, |ui| {
                if big(ui, perf, "Make independent").on_hover_text("Copies Parts 1-6 into this performance's private C/D slots (performance mode).").clicked() {
                    self.make_perf_independent();
                }
                let can_fill = perf && self.perf_bank == 1 && self.perf_idx == 0;
                if big(ui, can_fill, "Fill A001-A041").on_hover_text("Open performance A000 first.").clicked() {
                    self.confirm_fill = true;
                }
                ui.end_row();
                if big(ui, have, "Export set").clicked() {
                    self.export_filled_set();
                }
                if big(ui, have && can, "Send set").clicked() {
                    self.confirm_set_send = true;
                    self.set_send_ack = false;
                }
                ui.end_row();
            });

            // ---- files ----
            ui.add_space(14.0);
            heading(ui, "Save to a file");
            egui::Grid::new("dump_save").num_columns(2).spacing(GAP).show(ui, |ui| {
                if big(ui, true, "Current item").clicked() {
                    self.export_syx();
                }
                if big(ui, true, &format!("Bank {bl}")).clicked() {
                    let m = self.lib_program_msgs(Some(self.dump_bank));
                    self.save_bytes(&format!("nova_programs_bank_{bl}.syx"), m);
                }
                ui.end_row();
                if big(ui, true, "All programs").clicked() {
                    let m = self.lib_program_msgs(None);
                    self.save_bytes("nova_all_programs.syx", m);
                }
                if big(ui, true, "All perfs").clicked() {
                    let m = self.lib_perf_msgs();
                    self.save_bytes("nova_all_performances.syx", m);
                }
                ui.end_row();
            });
            if wide(ui, true, "Everything").clicked() {
                let mut m = self.lib_program_msgs(None);
                m.extend(self.lib_perf_msgs());
                self.save_bytes("nova_everything.syx", m);
            }
            ui.add_space(14.0);
            heading(ui, "Load");
            egui::Grid::new("dump_load").num_columns(2).spacing(GAP).show(ui, |ui| {
                if big(ui, true, "Merge file(s)").on_hover_text("File slots replace the library's same bank and number; everything else stays.").clicked() {
                    self.merge_files();
                }
                if big(ui, true, "Open backup").on_hover_text("Replaces the program and performance lists with the file's.").clicked() {
                    self.open_file();
                }
                ui.end_row();
            });
            if wide(ui, true, "Factory data").on_hover_text("Reload the factory programs and performances.").clicked() {
                self.use_factory();
            }
            });
        });

        // confirmation for bank sends
        if let Some(bs) = self.bank_send.take() {
            let bytes: usize = bs.msgs.iter().map(|m| m.len()).sum();
            let secs = bytes as u64 / 800 + bs.msgs.len() as u64 * midi::SEND_GAP_MS / 1000;
            let (mut go, mut cancel) = (false, false);
            egui::Window::new("Send to the Nova").collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
                ui.set_max_width(480.0);
                ui.label(format!("Send {}: {} messages, roughly {} min {} s, on SysEx channel {}.", bs.label, bs.msgs.len(), secs / 60, secs % 60, self.send_chan));
                ui.colored_label(WARN, "These are bank dumps: they overwrite the matching stored programs and performances on the Nova. Contents there are lost unless backed up.");
                ui.label("Needs Memory protect = Off and Sysex reception = Normal on the Nova. You can cancel while it sends; slots already written stay written.");
                ui.checkbox(&mut self.bank_send_ack, "I have a current backup and want to overwrite those slots");
                ui.horizontal(|ui| {
                    go = ui.add_enabled(self.bank_send_ack, egui::Button::new("Send")).clicked();
                    cancel = ui.button("Cancel").clicked();
                });
            });
            if go {
                self.live_on = false;
                let label = bs.label.clone();
                match self.midi.start_send(bs.msgs, ctx.clone()) {
                    Ok(job) => {
                        self.set_status(format!("Sending {} message(s)...", job.total), None);
                        self.send_note = Some(format!("Sent {label} to the Nova. Check the Nova."));
                        self.send_job = Some(job);
                    }
                    Err(e) => self.set_status(format!("Send failed: {e}"), Some(false)),
                }
                self.bank_send_ack = false;
            } else if !cancel {
                self.bank_send = Some(bs);
            } else {
                self.bank_send_ack = false;
            }
        }
        self.show_dump = open;
    }
}
