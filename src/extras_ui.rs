//! "Arp patterns" and "Globals" tabs.
use crate::{App, arp, perf, sysex};
use eframe::egui::{self, Color32, RichText};
use std::collections::BTreeMap;

const CHANGED: Color32 = Color32::from_rgb(0x6b, 0x9b, 0xff);

fn save_syx(name: &str, data: &[u8]) -> Result<String, String> {
    let Some(path) = crate::files::FileDialog::new().set_file_name(name).add_filter("SysEx", &["syx"]).save_file() else {
        return Err("Export cancelled".into());
    };
    std::fs::write(&path, data).map_err(|e| format!("Save failed: {e}"))?;
    Ok(format!("Saved {} ({} bytes)", path.display(), data.len()))
}

fn labels_path() -> Option<std::path::PathBuf> {
    crate::files::config_dir().map(|d| d.join("novaedit").join("globals_labels.json"))
}

pub fn load_labels() -> BTreeMap<usize, String> {
    labels_path()
        .and_then(|p| std::fs::read(p).ok())
        .and_then(|d| serde_json::from_slice(&d).ok())
        .unwrap_or_default()
}

fn save_labels(m: &BTreeMap<usize, String>) {
    if let Some(p) = labels_path() {
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(s) = serde_json::to_string_pretty(m) {
            let _ = std::fs::write(p, s);
        }
    }
}

/// Indexes at which two bodies differ.
pub fn diff_bodies(a: &[u8], b: &[u8]) -> Vec<usize> {
    (0..a.len().min(b.len())).filter(|&i| a[i] != b[i]).collect()
}

/// Label editor for byte `i`; returns true when changed.
fn label_edit(ui: &mut egui::Ui, labels: &mut BTreeMap<usize, String>, i: usize, width: f32) -> bool {
    let mut t = labels.get(&i).cloned().unwrap_or_default();
    if ui.add(egui::TextEdit::singleline(&mut t).desired_width(width).hint_text("label")).changed() {
        if t.is_empty() { labels.remove(&i); } else { labels.insert(i, t); }
        true
    } else {
        false
    }
}

impl App {
    pub fn set_extras(&mut self, pats: Vec<sysex::ArpPattern>, glob: Option<sysex::Globals>) {
        if !pats.is_empty() {
            self.patterns_orig = pats.clone();
            self.patterns = pats;
            self.pat_cur = Some(0);
        }
        if let Some(g) = glob {
            self.globals_orig = Some(g.clone());
            self.globals = Some(g);
        }
    }

    /// A pattern/globals dump received over MIDI replaces the entry with the same index.
    pub fn receive_extras(&mut self, msg: &[u8]) -> bool {
        if msg.first() != Some(&0xf0) {
            return false;
        }
        let (pats, glob) = sysex::parse_extras(&[msg[1..].to_vec()]);
        let got = !pats.is_empty() || glob.is_some();
        for p in pats {
            match self.patterns.iter().position(|q| q.idx == p.idx) {
                Some(i) => {
                    self.patterns[i] = p.clone();
                    self.patterns_orig[i] = p;
                }
                None => {
                    self.patterns.push(p.clone());
                    self.patterns_orig.push(p);
                }
            }
        }
        if let Some(g) = glob {
            self.globals_orig = Some(g.clone());
            self.globals = Some(g);
            self.globals_live = true;
        }
        got
    }

    pub fn arp_tab(&mut self, ui: &mut egui::Ui) {
        if self.patterns.is_empty() {
            ui.label("No arpeggiator patterns loaded. Open nova_backup_all.syx.");
            return;
        }
        let mut clicked = None;
        egui::Panel::left("pat_list").default_size(220.0).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.strong("Patterns");
                ui.label(self.patterns.len().to_string());
                ui.add(egui::TextEdit::singleline(&mut self.pat_filter).hint_text("filter...").desired_width(f32::INFINITY));
            });
            let f = self.pat_filter.to_lowercase();
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                for (i, p) in self.patterns.iter().enumerate() {
                    let kind = if arp::is_poly(p.idx) { "Poly" } else { "Mono" };
                    let text = format!("{:>3}  {kind}  {} steps", p.idx, arp::steps(&p.body));
                    if !f.is_empty() && !text.to_lowercase().contains(&f) {
                        continue;
                    }
                    let edited = *p != self.patterns_orig[i];
                    let label = if edited { RichText::new(format!("{text} *")).color(CHANGED) } else { RichText::new(text) };
                    if ui.selectable_label(self.pat_cur == Some(i), label).clicked() {
                        clicked = Some(i);
                    }
                }
            });
        });
        if clicked.is_some() {
            self.pat_cur = clicked;
        }
        let Some(ci) = self.pat_cur.filter(|i| *i < self.patterns.len()) else {
            ui.label("Select a pattern.");
            return;
        };
        let (idx, chan, poly) = (self.patterns[ci].idx, self.patterns[ci].chan, arp::is_poly(self.patterns[ci].idx));
        ui.horizontal_wrapped(|ui| {
            ui.strong(format!("Pattern {idx} ({})", if poly { "polyphonic" } else { "mono" }));
            let body = &mut self.patterns[ci].body;
            let mut n = body[1];
            ui.label("Steps");
            if ui.add(egui::DragValue::new(&mut n).range(1..=64)).changed() {
                body[1] = n;
            }
            let mut f = body[0];
            ui.label("flag (raw byte 0)");
            if ui.add(egui::DragValue::new(&mut f).range(0..=127)).changed() {
                body[0] = f;
            }
            if ui.button("Revert").clicked() {
                self.patterns[ci] = self.patterns_orig[ci].clone();
            }
            if ui.button("Export .syx...").clicked() {
                let msg = sysex::build_type0c(idx, &self.patterns[ci].body, chan);
                let r = save_syx(&format!("arp_pattern_{idx}.syx"), &msg);
                self.set_status(r.clone().unwrap_or_else(|e| e), Some(r.is_ok()));
            }
            if ui.button("Export all patterns...").clicked() {
                let data: Vec<u8> = self.patterns.iter().flat_map(|p| sysex::build_type0c(p.idx, &p.body, p.chan)).collect();
                let r = save_syx("arp_patterns.syx", &data);
                self.set_status(r.clone().unwrap_or_else(|e| e), Some(r.is_ok()));
            }
        });
        ui.weak("Velocity = (B&7)<<4 | C, flag = B bit 3. Gate (probable): Rest = no flag, vel 0; Norm = vel > 0; Tie = flag, vel 0; Glide = flag, vel > 0.");
        let steps = arp::steps(&self.patterns[ci].body);
        let orig = self.patterns_orig[ci].body.clone();
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            let body = &mut self.patterns[ci].body;
            let step_grid = |ui: &mut egui::Ui, body: &mut Vec<u8>, range: std::ops::Range<usize>, id: &str| {
                egui::Grid::new(id).striped(true).num_columns(5).show(ui, |ui| {
                    for h in ["Step", "Note", "Velocity", "Gate", "raw note / B / C"] {
                        ui.strong(h);
                    }
                    ui.end_row();
                    for s in range {
                        let o = 2 + 3 * s;
                        let changed = body[o..o + 3] != orig[o..o + 3];
                        let num = RichText::new(format!("{}", s + 1));
                        ui.label(if changed { num.color(CHANGED) } else { num });
                        // note
                        let mut note = body[o];
                        if poly {
                            let mut off = note as i32 - 64;
                            if ui.add(egui::DragValue::new(&mut off).range(-36..=36).suffix(" st")).changed() {
                                note = (off + 64) as u8;
                            }
                        } else {
                            let mut v = note;
                            if ui.add(egui::DragValue::new(&mut v).range(0..=127)).changed() {
                                note = v;
                            }
                        }
                        body[o] = note;
                        // velocity
                        let (b, c) = (body[o + 1], body[o + 2]);
                        let mut v = arp::velocity(b, c);
                        if ui.add(egui::DragValue::new(&mut v).range(0..=127)).changed() {
                            let (nb, nc) = arp::set_velocity(b, v);
                            body[o + 1] = nb;
                            body[o + 2] = nc;
                        }
                        // gate
                        let (b, c) = (body[o + 1], body[o + 2]);
                        let g = arp::gate(b, c);
                        let txt = match g {
                            arp::Gate::Norm => "Norm",
                            arp::Gate::Tie => "Tie?",
                            arp::Gate::Rest => "Rest",
                            arp::Gate::Glide => "Glide?",
                        };
                        egui::ComboBox::from_id_salt((id, "gate", s))
                            .selected_text(txt)
                            .show_ui(ui, |ui| {
                                for (g2, n) in [(arp::Gate::Norm, "Norm"), (arp::Gate::Tie, "Tie? (probable)"), (arp::Gate::Rest, "Rest"), (arp::Gate::Glide, "Glide? (probable)")] {
                                    if ui.selectable_label(g == g2, n).clicked() {
                                        let (nb, nc) = arp::set_gate(b, c, g2);
                                        body[o + 1] = nb;
                                        body[o + 2] = nc;
                                    }
                                }
                            })
                            .response
                            .on_hover_text("probable (decoded from firmware; no glide in factory data)");
                        ui.weak(format!("{} / {} / {}", body[o], body[o + 1], body[o + 2]));
                        ui.end_row();
                    }
                });
            };
            step_grid(ui, body, 0..steps, "steps");
            if steps < arp::MAX_STEPS {
                egui::CollapsingHeader::new(format!("Unused steps {}-{}", steps + 1, arp::MAX_STEPS)).default_open(false).show(ui, |ui| {
                    ui.add_enabled_ui(false, |ui| step_grid(ui, body, steps..arp::MAX_STEPS, "unused"));
                });
            }
            egui::CollapsingHeader::new("Raw bytes (B, C)").default_open(false).show(ui, |ui| {
                egui::Grid::new("rawbc").striped(true).show(ui, |ui| {
                    for s in 0..steps {
                        let o = 2 + 3 * s;
                        ui.label(format!("{}", s + 1));
                        for k in 1..3 {
                            let mut v = body[o + k];
                            if ui.add(egui::DragValue::new(&mut v).range(0..=127)).changed() {
                                body[o + k] = v;
                            }
                        }
                        ui.end_row();
                    }
                });
            });
        });
    }

    pub fn globals_tab(&mut self, ui: &mut egui::Ui) {
        let Some(orig) = self.globals_orig.clone() else {
            ui.label("No global parameters loaded. Open nova_backup_all.syx.");
            return;
        };
        let g = self.globals.as_mut().expect("globals");
        let chan = g.chan;
        let mut labels = self.globals_labels.clone();
        let mut labels_dirty = false;
        let sfx = |l: &BTreeMap<usize, String>, i: usize| l.get(&i).map(|t| format!("  - {t}")).unwrap_or_default();
        let mut compare_file = false;
        let mut compare_factory = false;
        ui.horizontal(|ui| {
            if ui.button("Revert").clicked() {
                *g = orig.clone();
            }
            if ui.button("Export .syx...").clicked() {
                let msg = sysex::build_type0f(&g.body, chan);
                let r = save_syx("nova_globals.syx", &msg);
                let (text, ok) = (r.clone().unwrap_or_else(|e| e), r.is_ok());
                self.status = (text, Some(ok));
            }
            compare_file = ui.button("Compare with file...").clicked();
            compare_factory = ui.button("Compare with factory globals").clicked();
            ui.weak("Global parameters (type 0x0f); not sent to the Nova.");
        });
        if compare_file {
            if let Some(path) = crate::files::FileDialog::new().add_filter("SysEx / MIDI", &["syx", "mid", "midi"]).pick_file() {
                let name = path.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
                match std::fs::read(&path).map(|d| sysex::parse_extras_file(&d).1) {
                    Ok(Some(o)) => self.globals_cmp = Some((name, o.body)),
                    Ok(None) => self.status = (format!("No globals dump in {name}"), Some(false)),
                    Err(e) => self.status = (format!("Cannot read {name}: {e}"), Some(false)),
                }
            }
        }
        if compare_factory {
            match sysex::parse_extras_file(crate::TOTAL_MID).1 {
                Some(o) => self.globals_cmp = Some(("factory (total.mid)".into(), o.body)),
                None => self.status = ("The embedded total.mid contains no globals message".into(), Some(false)),
            }
        }
        let g = self.globals.as_mut().expect("globals");
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            let b = &mut g.body;
            if let Some((name, other)) = self.globals_cmp.clone() {
                let mut close = false;
                ui.horizontal(|ui| {
                    ui.strong(format!("Differences: loaded globals vs {name}"));
                    close = ui.button("Close").clicked();
                });
                let diffs = diff_bodies(&orig.body, &other);
                if diffs.is_empty() {
                    ui.label("No differences.");
                }
                egui::Grid::new("globals_diff").striped(true).show(ui, |ui| {
                    for h in ["Index", "Hex", "Loaded", "Other", "Label"] {
                        ui.strong(h);
                    }
                    ui.end_row();
                    for i in diffs {
                        ui.label(format!("[{i}]"));
                        ui.monospace(format!("0x{i:02x}"));
                        ui.label(orig.body[i].to_string());
                        ui.label(other[i].to_string());
                        labels_dirty |= label_edit(ui, &mut labels, i, 200.0);
                        ui.end_row();
                    }
                });
                if close {
                    self.globals_cmp = None;
                }
                ui.separator();
            }
            egui::Grid::new("globals_named").striped(true).show(ui, |ui| {
                ui.label(format!("Global MIDI channel [0]{}", sfx(&labels, 0)));
                let cur = b[0];
                egui::ComboBox::from_id_salt("gchan").selected_text(format!("Ch {}", cur as u32 + 1)).show_ui(ui, |ui| {
                    for v in 0..16u8 {
                        if ui.selectable_label(cur == v, format!("Ch {}", v + 1)).clicked() {
                            b[0] = v;
                        }
                    }
                });
                ui.end_row();
                ui.label(format!("Master tune [2] (value-64){}", sfx(&labels, 2)));
                let mut t = b[2] as i32 - 64;
                if ui.add(egui::DragValue::new(&mut t).range(-64..=63)).changed() {
                    b[2] = (t + 64) as u8;
                }
                ui.end_row();
                for (lbl, i) in [
                    ("Breath/Pedal CC02 remap option [3] (raw)", 3usize),
                    ("Arp realtime transpose channel [4] (raw; 127 = default)", 4),
                    ("Input trim 1 [1] (probable, raw)", 1),
                    ("Input trim 2 [6] (probable, raw)", 6),
                ] {
                    ui.label(format!("{lbl}{}", sfx(&labels, i)));
                    let mut v = b[i];
                    if ui.add(egui::DragValue::new(&mut v).range(0..=127)).changed() {
                        b[i] = v;
                    }
                    ui.end_row();
                }
                ui.label(format!("Arp realtime transpose reference note [5]{}", sfx(&labels, 5)));
                let mut n = b[5];
                let w = egui::DragValue::new(&mut n)
                    .range(0..=127)
                    .custom_formatter(|v, _| perf::note_name(v as u8))
                    .custom_parser(|s| perf::parse_note(s).map(|n| n as f64));
                if ui.add(w).changed() {
                    b[5] = n;
                }
                ui.end_row();
                ui.label(format!("Master volume [7]{}", sfx(&labels, 7)));
                let mut v = b[7];
                if ui.add(egui::Slider::new(&mut v, 0..=127)).changed() {
                    b[7] = v;
                }
                ui.end_row();
                ui.label(format!("Local control flags [10] (raw, bit 2 relevant){}", sfx(&labels, 10)));
                let mut v = b[10];
                ui.horizontal(|ui| {
                    if ui.add(egui::DragValue::new(&mut v).range(0..=127)).changed() {
                        b[10] = v;
                    }
                    let mut bit2 = b[10] & 4 != 0;
                    if ui.checkbox(&mut bit2, "bit 2").changed() {
                        b[10] ^= 4;
                    }
                });
                ui.end_row();
            });
            ui.separator();
            egui::CollapsingHeader::new("All 272 bytes").default_open(false).show(ui, |ui| {
                egui::Grid::new("globals_all").striped(true).num_columns(4).show(ui, |ui| {
                    for i in 0..sysex::GLOBALS_LEN {
                        let t = RichText::new(format!("[{i}]  0x{i:02x}")).monospace();
                        ui.label(if b[i] != orig.body[i] { t.color(CHANGED) } else { t });
                        let mut v = b[i];
                        let r1 = ui.add(egui::Slider::new(&mut v, 0..=127).show_value(false));
                        let r2 = ui.add(egui::DragValue::new(&mut v).range(0..=127));
                        if r1.changed() || r2.changed() {
                            b[i] = v;
                        }
                        labels_dirty |= label_edit(ui, &mut labels, i, 180.0);
                        ui.end_row();
                    }
                });
            });
        });
        if labels_dirty {
            save_labels(&labels);
            self.globals_labels = labels;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diff_reports_exact_indexes() {
        let a = vec![0u8; 272];
        let mut b = a.clone();
        b[0] = 1;
        b[1] = 5;
        b[6] = 127;
        assert_eq!(diff_bodies(&a, &b), vec![0, 1, 6]);
        assert!(diff_bodies(&a, &a).is_empty());
    }
}
