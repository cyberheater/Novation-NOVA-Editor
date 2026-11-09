//! Row generation for the panel menu windows (oscillator / filter / LFO / envelope) from the params table,
//! plus the envelope-trigger byte encoding.
use crate::params::Param;

pub enum RowKind {
    /// combo of (value, label)
    Combo(Vec<(u8, String)>),
    Slider,
}

pub struct MenuRow {
    pub off: usize,
    pub label: String,
    pub kind: RowKind,
    pub min: i32,
    pub max: i32,
    pub hover: String,
}

/// Parse "Name (0=a, 1=b)" into the base name and, when every item is `digits=label`, the options.
/// Returns (base, options-if-all-pairs, text inside the parentheses).
pub fn parse_options(name: &str) -> (String, Option<Vec<(u8, String)>>, Option<String>) {
    let Some(i) = name.find('(') else { return (name.trim().to_string(), None, None) };
    let base = name[..i].trim().to_string();
    let inner = name[i + 1..].trim_end().trim_end_matches(')').to_string();
    let mut pairs = Vec::new();
    for piece in inner.split(',') {
        let piece = piece.trim();
        let Some((k, v)) = piece.split_once('=') else { return (base, None, Some(inner)) };
        match (k.trim().parse::<u8>(), v.trim()) {
            (Ok(k), v) if !v.is_empty() => pairs.push((k, v.to_string())),
            _ => return (base, None, Some(inner)),
        }
    }
    if pairs.len() >= 2 { (base, Some(pairs), Some(inner)) } else { (base, None, Some(inner)) }
}

fn starts_ci(name: &str, prefix: &str) -> bool {
    name.len() >= prefix.len() && name[..prefix.len()].eq_ignore_ascii_case(prefix)
}

/// Rows for a menu: `kind` is "osc", "filter", "lfo" or "env"; `n` is the 1-based selection.
pub fn rows_for(kind: &str, n: usize, params: &[Param]) -> Vec<MenuRow> {
    let prefix = match kind {
        "osc" => format!("Osc {n} "),
        "filter" => "Filter ".to_string(),
        "lfo" => format!("LFO {n} "),
        _ => format!("Env {n} "),
    };
    let mut rows = Vec::new();
    let base = 0x16 + 0x22 * (n.max(1) - 1);
    if kind == "osc" {
        let o = 10 * (n as u8 - 1);
        let opts = vec![
            (o, "Square".to_string()),
            (o + 1, "Saw".to_string()),
            (o + 2, "Special 2".to_string()),
            (o + 3, "Special 3".to_string()),
            (o + 4, "Special 4".to_string()),
        ];
        rows.push(MenuRow {
            off: base,
            label: "Waveform".into(),
            kind: RowKind::Combo(opts),
            min: 0,
            max: 127,
            hover: "0 Square, 1 Saw, 2-4 Special (mapping guessed); raw value = 10*(osc-1)+v".into(),
        });
        let n = n as i32 - 1;
        rows.push(MenuRow { off: base + 1, label: "Octave".into(), kind: RowKind::Slider, min: 30 + 5 * n, max: 34 + 5 * n, hover: String::new() });
        rows.push(MenuRow { off: base + 2, label: "Semitone (coarse)".into(), kind: RowKind::Slider, min: 45 + 25 * n, max: 69 + 25 * n, hover: String::new() });
    }
    let mut ps: Vec<&Param> = params
        .iter()
        .filter(|p| starts_ci(&p.name, &prefix) && !(kind == "osc" && (base..base + 3).contains(&p.offset)))
        .collect();
    ps.sort_by_key(|p| p.offset);
    for p in ps {
        let (label, opts, inner) = parse_options(&p.name);
        let mut hover = inner.map(|t| format!("({t})")).unwrap_or_default();
        if let (Some(lo), Some(hi)) = (p.observed_min, p.observed_max) {
            if !hover.is_empty() {
                hover.push('\n');
            }
            hover.push_str(&format!("seen {lo}-{hi}"));
        }
        rows.push(MenuRow {
            off: p.offset,
            label,
            kind: opts.map(RowKind::Combo).unwrap_or(RowKind::Slider),
            min: 0,
            max: 127,
            hover,
        });
    }
    rows
}

// ---- envelope triggering (byte 0xeb) ----
pub const TRIG_BYTE: usize = 0xeb;

/// 34 + 4*env1 + 2*env2 + env3, each 1 = single trigger. None when out of range.
pub fn trig_get(b: u8) -> Option<[bool; 3]> {
    (34..=41).contains(&b).then(|| {
        let v = b - 34;
        [v & 4 != 0, v & 2 != 0, v & 1 != 0]
    })
}
pub fn trig_set(single: [bool; 3]) -> u8 {
    34 + 4 * single[0] as u8 + 2 * single[1] as u8 + single[2] as u8
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::params;

    #[test]
    fn option_parsing() {
        let (b, o, _) = parse_options("Filter slope (0=12dB, 1=18dB, 2=24dB)");
        assert_eq!(b, "Filter slope");
        assert_eq!(o.unwrap(), vec![(0, "12dB".into()), (1, "18dB".into()), (2, "24dB".into())]);
        let (_, o, inner) = parse_options("Arp quantize (81=Off, 82..85=Mode 1..4)");
        assert!(o.is_none());
        assert_eq!(inner.unwrap(), "81=Off, 82..85=Mode 1..4");
        let (b, o, i) = parse_options("Env 1 Attack");
        assert_eq!((b.as_str(), o.is_none(), i.is_none()), ("Env 1 Attack", true, true));
        let (_, o, _) = parse_options("LFO 1 waveform (16=square, 17=saw, 18=triangle, 19=s/h)");
        assert_eq!(o.unwrap().len(), 4);
    }

    #[test]
    fn rows() {
        let p = params::load();
        let env1 = rows_for("env", 1, &p);
        assert!(env1.iter().any(|r| r.label == "Env 1 Attack" && r.off == 0xc2));
        assert!(env1.iter().any(|r| r.label == "Env 1 Wh"));
        assert!(env1.windows(2).all(|w| w[0].off <= w[1].off));
        let osc2 = rows_for("osc", 2, &p);
        assert_eq!(osc2[0].off, 0x38);
        assert_eq!((osc2[1].min, osc2[1].max), (35, 39));
        assert_eq!((osc2[2].min, osc2[2].max), (70, 94));
        assert!(osc2.iter().any(|r| r.label.contains("Mod knob")));
        assert!(osc2.iter().all(|r| !r.label.starts_with("Osc 1 ")));
        let filt = rows_for("filter", 1, &p);
        assert!(filt.iter().any(|r| r.off == 0xd0 && matches!(r.kind, RowKind::Combo(_))));
        assert!(filt.iter().any(|r| r.off == 0xcf));
        let lfo = rows_for("lfo", 2, &p);
        assert!(lfo.iter().any(|r| r.off == 0xab && matches!(r.kind, RowKind::Combo(_))));
    }

    #[test]
    fn trigger_encoding() {
        assert_eq!(trig_get(41), Some([true, true, true]));
        assert_eq!(trig_get(34), Some([false, false, false]));
        assert_eq!(trig_get(39), Some([true, false, true]));
        assert_eq!(trig_get(33), None);
        assert_eq!(trig_get(42), None);
        for b in 34..=41 {
            assert_eq!(trig_set(trig_get(b).unwrap()), b);
        }
    }
}
