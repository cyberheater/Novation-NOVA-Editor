//! Effect parameter groups (program offsets 0xfe..0x11d) and their mapping into performance part blocks.
use crate::perf;

pub const FX_START: usize = 0xfe;
pub const FX_END: usize = 0x11d;

pub struct Group {
    pub key: &'static str,
    pub title: &'static str,
    /// (name, program offset); the first entry is the one the 'effect' knob edits.
    pub params: &'static [(&'static str, usize)],
    /// true: stored only in the program (vocoder balance), never in a performance part.
    pub program_only: bool,
}

pub const GROUPS: [Group; 6] = [
    Group {
        key: "delay",
        title: "Delay",
        params: &[
            ("Send level", 0x114),
            ("Delay time", 0x116),
            ("Feedback", 0x117),
            ("HF damp", 0x118),
            ("Mod wheel depth", 0x115),
            ("Stereo width", 0x11a),
        ],
        program_only: false,
    },
    Group {
        key: "reverb",
        title: "Reverb",
        params: &[
            ("Send level", 0x109),
            ("Mod wheel depth", 0x10a),
            ("Decay", 0x10b),
            ("HF damp", 0x10c),
            ("Type / early ref level", 0x10d),
        ],
        program_only: false,
    },
    Group {
        key: "chorus",
        title: "Chorus",
        params: &[
            ("Send level", 0x10e),
            ("Mod wheel depth", 0x10f),
            ("Speed", 0x110),
            ("Mod depth", 0x111),
            ("Feedback", 0x112),
        ],
        program_only: false,
    },
    Group {
        key: "dist",
        title: "Dist / EQ / Config",
        params: &[
            ("Distortion level", 0xfe),
            ("Distortion mod wheel", 0xff),
            ("EQ bass", 0x100),
            ("EQ treble", 0x101),
            ("Comb frequency", 0x102),
            ("Comb boost", 0x103),
            ("Effects dry level", 0x104),
            ("Config morph amount", 0x11d),
        ],
        program_only: false,
    },
    Group {
        key: "pan",
        title: "Pan",
        params: &[("Pan position", 0x105), ("Panning speed", 0x106), ("Panning depth", 0x107)],
        program_only: false,
    },
    Group { key: "vocoder", title: "Vocoder", params: &[("Vocoder balance", 0x10)], program_only: true },
];

pub fn group(key: &str) -> Option<&'static Group> {
    GROUPS.iter().find(|g| g.key == key)
}

/// Index into a performance body of the effect byte `off` (program offset) for `part`.
pub fn part_index(part: usize, off: usize) -> usize {
    debug_assert!((FX_START..=FX_END).contains(&off));
    perf::part_off(part) + perf::rel::FX + (off - FX_START)
}

/// Copy the group's named bytes from `program` into the part's own effects area and set
/// "Effects used" to Part. Returns false (and changes nothing) for the vocoder.
pub fn copy_effect(perf_body: &mut [u8], part: usize, program: &[u8], g: &Group) -> bool {
    if g.program_only {
        return false;
    }
    for &(_, off) in g.params {
        perf_body[part_index(part, off)] = program[off];
    }
    perf::effects_used_set(perf_body, part, false);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offsets_and_mapping() {
        assert_eq!(group("delay").unwrap().params[0].1, 0x114);
        assert_eq!(group("dist").unwrap().params[0].1, 0xfe);
        assert_eq!(group("vocoder").unwrap().params[0].1, 0x10);
        for g in GROUPS.iter().filter(|g| !g.program_only) {
            for &(_, o) in g.params {
                assert!((FX_START..=FX_END).contains(&o));
            }
        }
        assert_eq!(part_index(0, 0xfe), 20 + 12);
        assert_eq!(part_index(1, 0x114), 20 + 44 + 12 + 22);
        // last effect byte of the last part is the last body byte
        assert_eq!(part_index(7, FX_END), perf::PERF_LEN - 1);
    }

    #[test]
    fn copy_touches_only_named_bytes() {
        let mut body: Vec<u8> = (0..perf::PERF_LEN).map(|i| (i % 100) as u8).collect();
        let packed = perf::part_off(2) + perf::rel::PACKED;
        body[packed] = 0x52;
        let before = body.clone();
        let mut prog = vec![0u8; 286];
        for (i, b) in prog.iter_mut().enumerate() {
            *b = 100 + (i % 20) as u8;
        }
        let g = group("delay").unwrap();
        assert!(copy_effect(&mut body, 2, &prog, g));
        let changed: Vec<usize> = (0..body.len()).filter(|&i| body[i] != before[i]).collect();
        let mut expect: Vec<usize> = g.params.iter().map(|&(_, o)| part_index(2, o)).collect();
        expect.push(packed);
        expect.sort();
        assert_eq!(changed, expect);
        assert_eq!(body[packed], 0x12); // bit 6 cleared, rest kept
        assert!(!perf::effects_used_get(&body, 2));
        assert!(!copy_effect(&mut body, 2, &prog, group("vocoder").unwrap()));
    }
}
