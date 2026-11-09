//! Parameter table (one row per program byte) loaded from the embedded JSON.
use serde::Deserialize;

pub const N: usize = 286;

#[derive(Debug, Clone, Deserialize)]
pub struct Param {
    pub offset: usize,
    #[serde(default)]
    #[allow(dead_code)]
    pub hex: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub sources: Vec<String>,
    pub observed_min: Option<i64>,
    pub observed_max: Option<i64>,
    #[allow(dead_code)]
    pub observed_common: Option<i64>,
}

pub fn load() -> Vec<Param> {
    let mut v: Vec<Param> =
        serde_json::from_str(include_str!("../assets/nova_params.json")).unwrap_or_default();
    v.sort_by_key(|p| p.offset);
    v
}

/// Find the program byte by exact (case-insensitive) name; falls back to an exact
/// (case-insensitive) match of any source text.
pub fn find_name(params: &[Param], name: &str) -> Option<usize> {
    params
        .iter()
        .find(|p| p.name.eq_ignore_ascii_case(name))
        .or_else(|| params.iter().find(|p| p.sources.iter().any(|s| s.eq_ignore_ascii_case(name))))
        .map(|p| p.offset)
}

/// Display name for a byte ("" when unknown).
pub fn display_name(p: &Param) -> String {
    if !p.name.is_empty() && !p.name.starts_with("Program name") {
        p.name.clone()
    } else {
        String::new()
    }
}

pub const CATS: [&str; 20] = [
    "Bass", "Hard bass", "Soft lead", "Hard lead", "Portalead", "Arpeggio", "Motion", "Bell", "Pad",
    "Strings", "Keyboard", "SFX", "Drums", "Pianos", "Organ", "User 1", "User 2", "User 3", "User 4",
    "User 5",
];

pub const SECTIONS: [(&str, usize, usize); 6] = [
    ("Header", 0x10, 0x15),
    ("Oscillator 1", 0x16, 0x37),
    ("Oscillator 2", 0x38, 0x59),
    ("Oscillator 3", 0x5a, 0x7b),
    ("Filters, envelopes, LFOs, arpeggiator (names not yet decoded)", 0x7c, 0xfd),
    ("Effects", 0xfe, 0x11d),
];

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn has_286_rows() {
        let p = load();
        assert_eq!(p.len(), 286);
        assert!(p.iter().enumerate().all(|(i, r)| r.offset == i));
    }
    #[test]
    fn name_lookup() {
        let p = load();
        assert_eq!(find_name(&p, "Env 1 Attack"), Some(0xc2));
        assert_eq!(find_name(&p, "env 3 delay"), Some(0xbb));
        assert_eq!(find_name(&p, "Filter Cutoff Freq"), Some(0xd3));
        assert!(find_name(&p, "Filter Mod knob Wh frequency").is_some());
        assert_eq!(find_name(&p, "Osc 2 Fine Tune"), Some(0x3b));
        assert!(find_name(&p, "Osc 1 Pitch Env 2").is_some());
        assert_eq!(find_name(&p, "Pan"), Some(0x105));
        assert_eq!(find_name(&p, "No such thing"), None);
    }
}
