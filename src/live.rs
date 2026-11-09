//! App -> Nova live editing: turn a changed program byte into the controller / NRPN messages the
//! Nova itself uses (CC, or NRPN LSB via CC98 + data entry CC6; CC99 is never needed).
//! Pure functions only; sending is decided in main.rs and is opt-in.
use crate::mirror::packed_target;
use crate::params::Param;

/// Messages (raw 3-byte channel messages) for byte `off` changing to `val`.
/// `old` is the previous value of the byte (needed for the bit-0 flag byte).
/// Returns None when the byte has no plain CC / NRPN route.
pub fn messages_for(off: usize, old: u8, val: u8, params: &[Param], chan: u8) -> Option<Vec<[u8; 3]>> {
    let ch = chan & 0x0f;
    // packed-NRPN (LSB 1) enum bytes first: value -> code
    for code in 0u8..=127 {
        if let Some((o, v, bit0)) = packed_target(code) {
            if o != off {
                continue;
            }
            let hit = if bit0 { (val & 1 == 1) == (code == 91) && (old & 1) != (val & 1) } else { v == val };
            if hit {
                return Some(nrpn(ch, 1, code));
            }
        }
    }
    let p = params.iter().find(|p| p.offset == off)?;
    if let Some(cc) = p.sources.iter().find_map(|s| s.strip_prefix("CC ").and_then(|r| r.split_whitespace().next()?.parse::<u8>().ok())) {
        // controllers that are really modes / data entry are never program bytes
        if !matches!(cc, 0 | 6 | 32 | 38 | 98..=101 | 120..=127) {
            return Some(vec![[0xb0 | ch, cc, val & 0x7f]]);
        }
    }
    let n = p.sources.iter().find_map(|s| {
        let r = s.strip_prefix("NRPN ")?;
        if s.contains(" sub ") {
            return None;
        }
        r.split_whitespace().next()?.parse::<u8>().ok()
    })?;
    if n == 1 || [84, 100, 109, 110, 113].contains(&n) {
        return None;
    }
    Some(nrpn(ch, n, val & 0x7f))
}

fn nrpn(ch: u8, lsb: u8, data: u8) -> Vec<[u8; 3]> {
    vec![[0xb0 | ch, 98, lsb], [0xb0 | ch, 6, data]]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params() -> Vec<Param> {
        serde_json::from_str(include_str!("../assets/nova_params.json")).unwrap()
    }

    #[test]
    fn cutoff_is_cc105() {
        let m = messages_for(0xd3, 10, 55, &params(), 7).unwrap();
        assert_eq!(m, vec![[0xb7, 105, 55]]);
    }

    #[test]
    fn filter_type_uses_packed_code() {
        let m = messages_for(0xcf, 3, 9, &params(), 0).unwrap();
        assert_eq!(m, vec![[0xb0, 98, 1], [0xb0, 6, 9]]);
    }

    #[test]
    fn arp_mute_bit() {
        let m = messages_for(0xf9, 4, 5, &params(), 0).unwrap();
        assert_eq!(m, vec![[0xb0, 98, 1], [0xb0, 6, 91]]);
        assert!(messages_for(0xf9, 4, 6, &params(), 0).is_none()); // bit 0 unchanged
    }

    #[test]
    fn name_bytes_have_no_route() {
        assert!(messages_for(0, b'A', b'B', &params(), 0).is_none());
    }
}
