//! Arpeggiator program bytes and button mappings.

pub const ON: usize = 0xfa; // 72 = On, 73 = Off
pub const LATCH: usize = 0xf7; // 0 / 1
pub const MUTE: usize = 0xf9; // bit 0 = mute on
pub const SETTING: usize = 0xf4; // raw, factory values 122..127

pub fn on_lit(b: u8) -> bool {
    b == 72
}
pub fn on_toggle(b: u8) -> u8 {
    if b == 72 { 73 } else { 72 }
}
pub fn latch_lit(b: u8) -> bool {
    b == 1
}
pub fn latch_toggle(b: u8) -> u8 {
    if b == 1 { 0 } else { 1 }
}
pub fn mute_lit(b: u8) -> bool {
    b & 1 != 0
}
/// Toggles only bit 0.
pub fn mute_toggle(b: u8) -> u8 {
    b ^ 1
}

/// (label, byte offset, [(value, name)]) for the Arpeggiator window combos.
pub type EnumRow = (&'static str, usize, &'static [(u8, &'static str)]);
pub const ENUMS: [EnumRow; 6] = [
    ("Keysync", 0xf3, &[(74, "On"), (75, "Off")]),
    ("Pattern bank", 0xf0, &[(76, "Mono"), (77, "Poly"), (78, "User")]),
    ("Latch type", 0xf8, &[(79, "Constant"), (80, "Pattern")]),
    ("Quantize", 0xf5, &[(81, "Off"), (82, "Mode 1"), (83, "Mode 2"), (84, "Mode 3"), (85, "Mode 4")]),
    ("Velocity", 0xf2, &[(86, "Played"), (87, "Full"), (88, "Half"), (89, "Programmed")]),
    ("Realtime transpose", 0xfb, &[(92, "On"), (93, "Off")]),
];

// ---- arp pattern step decoding ----
pub const MAX_STEPS: usize = 64;

pub fn is_poly(idx: u8) -> bool {
    idx >= 64
}
pub fn steps(body: &[u8]) -> usize {
    (body[1] as usize).clamp(1, MAX_STEPS)
}
/// (note, B, C) of step `s`.
pub fn step(body: &[u8], s: usize) -> (u8, u8, u8) {
    let o = 2 + 3 * s;
    (body[o], body[o + 1], body[o + 2])
}
pub fn velocity(b: u8, c: u8) -> u8 {
    ((b & 7) << 4) | (c & 15)
}
/// Keeps B bit 3 (tie flag).
pub fn set_velocity(b: u8, v: u8) -> (u8, u8) {
    ((b & 8) | ((v >> 4) & 7), v & 15)
}

/// Step gate state (probable, decoded from firmware): internal velocity byte = flag<<7 | vel7 with
/// flag = (B >> 3) & 1 and vel7 = ((B & 7) << 4) | C.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Gate {
    Norm,
    Tie,
    Rest,
    Glide,
}
pub fn flag(b: u8) -> bool {
    b & 8 != 0
}
pub fn gate(b: u8, c: u8) -> Gate {
    match (flag(b), velocity(b, c) > 0) {
        (false, false) => Gate::Rest,
        (false, true) => Gate::Norm,
        (true, false) => Gate::Tie,
        (true, true) => Gate::Glide,
    }
}
/// New (B, C) after choosing a gate type. Rest/Tie clear the velocity; Norm/Glide keep it (64 if it was 0).
pub fn set_gate(b: u8, c: u8, g: Gate) -> (u8, u8) {
    let keep = match velocity(b, c) {
        0 => 64,
        v => v,
    };
    let (vb, vc) = ((keep >> 4) & 7, keep & 15);
    match g {
        Gate::Rest => (0, 0),
        Gate::Tie => (8, 0),
        Gate::Norm => (vb, vc),
        Gate::Glide => (8 | vb, vc),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sysex;

    #[test]
    fn button_mappings() {
        assert!(on_lit(72) && !on_lit(73));
        assert_eq!((on_toggle(72), on_toggle(73)), (73, 72));
        assert_eq!((latch_toggle(0), latch_toggle(1)), (1, 0));
        assert!(latch_lit(1) && !latch_lit(0));
        assert_eq!(mute_toggle(4), 5);
        assert_eq!(mute_toggle(5), 4);
        assert!(mute_lit(5) && !mute_lit(4));
    }

    #[test]
    fn pattern_step_decoding() {
        assert_eq!(velocity(7, 15), 127);
        assert_eq!(velocity(0, 1), 1);
        assert_eq!(set_velocity(8, 127), (15, 15));
        assert_eq!(set_velocity(0, 64), (4, 0));
        assert_eq!(gate(0, 0), Gate::Rest);
        assert_eq!(gate(8, 0), Gate::Tie);
        assert_eq!(gate(7, 15), Gate::Norm);
        assert_eq!(gate(15, 15), Gate::Glide);
        assert_eq!(gate(8, 1), Gate::Glide);
        assert_eq!(set_gate(7, 15, Gate::Rest), (0, 0));
        assert_eq!(set_gate(7, 15, Gate::Tie), (8, 0));
        assert_eq!(set_gate(0, 0, Gate::Norm), (4, 0)); // default velocity 64
        assert_eq!(set_gate(0, 0, Gate::Glide), (12, 0));
        assert_eq!(set_gate(15, 15, Gate::Norm), (7, 15)); // keeps velocity, clears flag
        assert_eq!(set_gate(7, 15, Gate::Glide), (15, 15));
        // round trip through every gate keeps decoding consistent
        for g in [Gate::Norm, Gate::Tie, Gate::Rest, Gate::Glide] {
            let (b, c) = set_gate(5, 3, g);
            assert_eq!(gate(b, c), g);
        }
        assert!(!is_poly(63) && is_poly(64));
    }

    #[test]
    fn factory_arp_on() {
        let (progs, _) = sysex::parse_all(include_bytes!("../assets/total.mid"));
        let get = |b: u8, i: u8| progs.iter().find(|p| p.bank == Some(b) && p.idx == i).unwrap();
        assert_eq!(get(5, 1).name(), "Skewed Arp");
        assert_eq!(get(5, 1).body[ON], 72);
        assert_eq!(get(5, 14).name(), "Synthi Harp M-Wh");
        assert_eq!(get(5, 14).body[ON], 73);
    }
}
