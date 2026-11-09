//! Performance (type 7) layout helpers: part blocks, note names, velocity curves, ids.

pub const PERF_LEN: usize = 372;
pub const PART_BASE: usize = 20;
pub const PART_LEN: usize = 44;
pub const PARTS: usize = 8;

/// Offsets inside a 44-byte part block.
pub mod rel {
    pub const PROG_NUM: usize = 0;
    pub const PROG_BANK: usize = 1;
    pub const CHAN: usize = 2;
    pub const RAW3: usize = 3;
    pub const RAW4: usize = 4;
    pub const SEMI: usize = 5;
    pub const DETUNE: usize = 6;
    pub const VEL: usize = 7;
    pub const LOW: usize = 8;
    pub const HIGH: usize = 9;
    pub const PACKED: usize = 10;
    pub const LEVEL: usize = 11;
    pub const FX: usize = 12;
}

/// Header byte 3 (body offset 19) is the Part mute bitmask (bit i = Part i+1 muted).
pub const MUTE_BYTE: usize = 19;

pub fn mute_mask(body: &[u8]) -> u8 {
    body[MUTE_BYTE] & 0x3f
}
pub fn is_part_muted(body: &[u8], p: usize) -> bool {
    p < 6 && body[MUTE_BYTE] & (1 << p) != 0
}
/// Changes only bit `p`.
pub fn set_part_muted(body: &mut [u8], p: usize, muted: bool) {
    if p < 6 {
        if muted { body[MUTE_BYTE] |= 1 << p } else { body[MUTE_BYTE] &= !(1 << p) }
    }
}

fn packed(body: &[u8], p: usize) -> usize {
    part_off(p) + rel::PACKED
}
/// Polyphony (bits 0-1): 0 Off, 1 Mono, 2 Poly, 3 Prog.
pub const POLY_NAMES: [&str; 4] = ["Off", "Mono", "Poly", "Prog"];
pub fn poly_get(body: &[u8], p: usize) -> u8 {
    body[packed(body, p)] & 3
}
pub fn poly_set(body: &mut [u8], p: usize, v: u8) {
    let i = packed(body, p);
    body[i] = (body[i] & !3) | (v & 3);
}
/// Effects used (bit 6): true = Program's own effects, false = Part (performance's block).
pub fn effects_used_get(body: &[u8], p: usize) -> bool {
    body[packed(body, p)] & 0x40 != 0
}
pub fn effects_used_set(body: &mut [u8], p: usize, program: bool) {
    let i = packed(body, p);
    if program { body[i] |= 0x40 } else { body[i] &= !0x40 }
}
/// Bit 4 of the Part's packed byte: purpose unknown. It is NOT the vocoder (disproved on hardware 2026-11-09:
/// the "[V]" indicator means vocoder balance != 000, stored in the performance's 0x1c companion message).
pub fn flag4_get(body: &[u8], p: usize) -> bool {
    body[packed(body, p)] & 0x10 != 0
}
pub fn flag4_set(body: &mut [u8], p: usize, v: bool) {
    let i = packed(body, p);
    if v { body[i] |= 0x10 } else { body[i] &= !0x10 }
}

/// MIDI channel byte: 0..15 = channel 1..16, 16 = Omni (assumed), 17 = Global.
pub fn midi_channel_label(raw: u8) -> String {
    match raw {
        0..=15 => format!("Ch {}", raw + 1),
        16 => "Omni".into(),
        17 => "Global".into(),
        _ => format!("raw {raw}"),
    }
}

pub fn part_off(p: usize) -> usize {
    PART_BASE + PART_LEN * p
}

pub const NOTES: [&str; 12] = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];

/// MIDI note number to name; 24 = C0, 60 = C3, 120 = C8.
pub fn note_name(n: u8) -> String {
    format!("{}{}", NOTES[(n % 12) as usize], (n / 12) as i32 - 2)
}

/// Parse "C3", "F#1", "a-1" or a plain number.
pub fn parse_note(s: &str) -> Option<u8> {
    let s = s.trim();
    if let Ok(n) = s.parse::<u8>() {
        return (n < 128).then_some(n);
    }
    let up = s.to_uppercase();
    let letter = up.chars().next()?;
    let mut rest = &up[1..];
    let mut name = letter.to_string();
    if let Some(r) = rest.strip_prefix('#') {
        name.push('#');
        rest = r;
    }
    let idx = NOTES.iter().position(|n| *n == name)? as i32;
    let oct: i32 = rest.parse().ok()?;
    let n = (oct + 2) * 12 + idx;
    (0..128).contains(&n).then_some(n as u8)
}

pub const VEL_CURVES: [&str; 16] = [
    "Normal", "Inverse", "Cross Fade - High", "Cross Fade - Low", "To value 60", "From value 61",
    "To value 70", "From value 71", "To value 80", "From value 81", "To value 90", "From value 91",
    "To value 100", "From value 101", "To value 110", "From value 111",
];

/// Program bank byte 5/6 -> 'A'/'B'.
pub fn prog_bank_letter(b: u8) -> char {
    (b'A' + b.saturating_sub(5).min(25)) as char
}
/// Performance bank byte 1 -> 'A'.
pub fn perf_bank_letter(b: u8) -> char {
    (b'A' + b.saturating_sub(1).min(25)) as char
}
pub fn prog_id(bank: u8, idx: u8) -> String {
    format!("{}{:03}", prog_bank_letter(bank), idx)
}
pub fn perf_id(bank: u8, idx: u8) -> String {
    format!("{}{:03}", perf_bank_letter(bank), idx)
}

/// Short description of a byte inside a part block.
pub fn rel_name(r: usize) -> Option<&'static str> {
    Some(match r {
        rel::PROG_NUM => "Program number",
        rel::PROG_BANK => "Program bank (5=A, 6=B, 7=C, 8=D)",
        rel::CHAN => "MIDI channel (tentative: 0 omni?, 1-16, 17 global)",
        rel::RAW3 => "raw (packed, unknown)",
        rel::RAW4 => "raw (packed, unknown)",
        rel::SEMI => "Semitone (value-64)",
        rel::DETUNE => "Detune cents (value-64)",
        rel::VEL => "Velocity curve (value-80)",
        rel::LOW => "Lowest note",
        rel::HIGH => "Highest note",
        rel::PACKED => "Packed flags (polyphony/mute?)",
        rel::LEVEL => "Part level",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sysex;

    #[test]
    fn notes_and_curves() {
        assert_eq!(note_name(24), "C0");
        assert_eq!(note_name(120), "C8");
        assert_eq!(note_name(60), "C3");
        assert_eq!(parse_note("C3"), Some(60));
        assert_eq!(parse_note("f#1"), Some(42));
        assert_eq!(VEL_CURVES[0], "Normal");
        assert_eq!(VEL_CURVES[2], "Cross Fade - High");
        assert_eq!(VEL_CURVES[15], "From value 111");
        assert_eq!(prog_id(5, 30), "A030");
        assert_eq!(prog_id(6, 7), "B007");
        assert_eq!(perf_id(1, 0), "A000");
    }

    #[test]
    fn channel_labels() {
        assert_eq!(midi_channel_label(0), "Ch 1");
        assert_eq!(midi_channel_label(5), "Ch 6");
        assert_eq!(midi_channel_label(15), "Ch 16");
        assert_eq!(midi_channel_label(16), "Omni");
        assert_eq!(midi_channel_label(17), "Global");
    }

    #[test]
    fn mute_and_packed_bits() {
        let mut b = vec![0u8; PERF_LEN];
        b[MUTE_BYTE] = 0x40;
        set_part_muted(&mut b, 2, true);
        assert_eq!(b[MUTE_BYTE], 0x44);
        assert!(is_part_muted(&b, 2) && !is_part_muted(&b, 1));
        set_part_muted(&mut b, 2, false);
        assert_eq!(b[MUTE_BYTE], 0x40);
        assert_eq!(mute_mask(&b), 0);
        let i = part_off(1) + rel::PACKED;
        b[i] = 0x52;
        poly_set(&mut b, 1, 3);
        assert_eq!(b[i], 0x53);
        assert_eq!(poly_get(&b, 1), 3);
        effects_used_set(&mut b, 1, false);
        assert_eq!(b[i], 0x13);
        assert!(!effects_used_get(&b, 1));
        flag4_set(&mut b, 1, false);
        assert_eq!(b[i], 0x03);
        assert!(!flag4_get(&b, 1));
    }

    #[test]
    fn factory_mutes() {
        let d = std::fs::read("assets/total.mid").unwrap();
        let (_, perfs) = sysex::parse_all(&d);
        let muted: Vec<_> = perfs.iter().filter(|p| mute_mask(&p.body) != 0).collect();
        assert!(!muted.is_empty());
        for p in muted {
            let m = mute_mask(&p.body);
            for i in 0..6 {
                assert_eq!(is_part_muted(&p.body, i), m & (1 << i) != 0);
            }
        }
        let msg = sysex::build_type7(&perfs[0].body, 1, 0, 0);
        assert_eq!(msg.len(), 383);
    }

    #[test]
    fn performance_sanity() {
        let d = std::fs::read("assets/total.mid").unwrap();
        let (progs, perfs) = sysex::parse_all(&d);
        assert_eq!(perfs.len(), 128);
        assert!(perfs.iter().all(|p| p.body.len() == PERF_LEN && p.bank == 1));
        let p = &perfs[0];
        assert_eq!(&p.body[..16], b"Blaster Master! ");
        let b = part_off(0);
        assert_eq!(&p.body[b..b + 12], &[30, 5, 17, 98, 77, 76, 64, 80, 24, 48, 82, 127]);
        let prog = progs
            .iter()
            .find(|q| q.bank == Some(p.body[b + rel::PROG_BANK]) && q.idx == p.body[b + rel::PROG_NUM])
            .unwrap();
        assert_eq!(prog.name(), "DigiTrowserCough");
        assert_eq!(p.body[b + rel::SEMI] as i32 - 64, 12);
        assert_eq!(note_name(p.body[b + rel::LOW]), "C0");
        assert_eq!(note_name(p.body[b + rel::HIGH]), "C2");
    }
}
