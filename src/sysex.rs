//! Standard MIDI File / raw .syx parsing and Nova program dump building.
use crate::params::N;

#[derive(Debug, Clone)]
pub struct Program {
    pub body: Vec<u8>,
    pub bank: Option<u8>,
    pub idx: u8,
    pub label: String,
}

impl Program {
    pub fn name(&self) -> String {
        self.body[..16]
            .iter()
            .map(|&b| if (0x20..0x7f).contains(&b) { b as char } else { ' ' })
            .collect::<String>()
            .trim()
            .to_string()
    }
}

/// Extra data of a "Single performance" transmit: the embedded part programs and raw extra messages.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Bundle {
    pub chan: u8,
    /// Type-0 part dumps (286-byte bodies) by part index 0..7.
    pub parts: [Option<Vec<u8>>; 8],
    /// Other Nova messages (e.g. type 0x1b), full bytes F0..F7, in original order after the type-5.
    pub extra: Vec<Vec<u8>>,
}

#[derive(Debug, Clone)]
pub struct Performance {
    pub body: Vec<u8>,
    pub bank: u8,
    pub idx: u8,
    /// Some for a type-5 "Nova buffer" performance with embedded programs.
    pub bundle: Option<Bundle>,
}

impl Performance {
    pub fn name(&self) -> String {
        self.body[..16]
            .iter()
            .map(|&b| if (0x20..0x7f).contains(&b) { b as char } else { ' ' })
            .collect::<String>()
            .trim()
            .to_string()
    }
}

/// Parse type-7 performance dumps (382 bytes after F0).
pub fn parse_perf_messages(msgs: &[Vec<u8>]) -> Vec<Performance> {
    msgs.iter()
        .filter(|m| {
            m.len() == 382 && m[..5] == [0x00, 0x20, 0x29, 0x01, 0x21] && m[6] == 7
        })
        .map(|m| Performance { body: m[9..381].to_vec(), bank: m[7], idx: m[8], bundle: None })
        .collect()
}

pub fn messages_of(d: &[u8]) -> Vec<Vec<u8>> {
    if d.first() == Some(&0xf0) { parse_syx(d) } else { parse_smf(d) }
}

/// Message type of a Nova message (bytes after F0).
pub fn nova_type(m: &[u8]) -> Option<u8> {
    (m.len() >= 7 && m[..5] == [0x00, 0x20, 0x29, 0x01, 0x21]).then(|| m[6])
}

/// Type-5 performance dump (380 bytes after F0): returns (channel, 372-byte body).
pub fn parse_type5(m: &[u8]) -> Option<(u8, Vec<u8>)> {
    (m.len() == 380 && nova_type(m) == Some(5)).then(|| (m[5], m[7..379].to_vec()))
}

pub fn parse_live_type5(full: &[u8]) -> Option<(u8, Vec<u8>)> {
    if full.first() != Some(&0xf0) {
        return None;
    }
    parse_type5(&full[1..])
}

/// Parse a message sequence. Type-0 dumps to parts 0..7 followed by a type-5 are folded into a
/// performance bundle (together with directly following type-0x1b messages); other type-0 dumps
/// stay ordinary programs.
pub fn parse_stream(msgs: &[Vec<u8>]) -> (Vec<Program>, Vec<Performance>) {
    let all_progs = parse_messages(msgs);
    let mut perfs = parse_perf_messages(msgs);
    // walk again to know order: indices into all_progs for type-0 dumps
    let mut progs: Vec<Option<Program>> = Vec::new();
    let mut pending: [Option<usize>; 8] = Default::default();
    let mut it = all_progs.into_iter();
    let mut last_bundle: Option<usize> = None;
    let mut bundles: Vec<Performance> = Vec::new();
    for m in msgs {
        let ty = nova_type(m);
        if ty != Some(0x1b) {
            last_bundle = None;
        }
        match ty {
            Some(0) if m.len() == 295 => {
                if let Some(p) = it.next() {
                    if m[7] < 8 {
                        pending[m[7] as usize] = Some(progs.len());
                    }
                    progs.push(Some(p));
                }
            }
            Some(2) if m.len() == 296 => {
                if let Some(p) = it.next() {
                    progs.push(Some(p));
                }
            }
            Some(5) => {
                if let Some((chan, body)) = parse_type5(m) {
                    let mut b = Bundle { chan, ..Default::default() };
                    for (i, slot) in pending.iter_mut().enumerate() {
                        if let Some(pi) = slot.take() {
                            b.parts[i] = progs[pi].take().map(|p| p.body);
                        }
                    }
                    bundles.push(Performance { body, bank: 1, idx: 0, bundle: Some(b) });
                    last_bundle = Some(bundles.len() - 1);
                }
            }
            Some(0x1b) => {
                if let Some(i) = last_bundle {
                    let mut full = vec![0xf0];
                    full.extend_from_slice(m);
                    bundles[i].bundle.as_mut().unwrap().extra.push(full);
                }
            }
            _ => {}
        }
    }
    perfs.extend(bundles);
    (progs.into_iter().flatten().collect(), perfs)
}

/// Arp pattern dump (type 0x0c): idx 0-63 mono, 64-127 polyphonic; 194-byte body.
#[derive(Debug, Clone, PartialEq)]
pub struct ArpPattern {
    pub idx: u8,
    pub body: Vec<u8>,
    pub chan: u8,
}

/// Global parameters dump (type 0x0f): 272-byte body.
#[derive(Debug, Clone, PartialEq)]
pub struct Globals {
    pub body: Vec<u8>,
    pub chan: u8,
}

pub const PATTERN_LEN: usize = 194;
pub const GLOBALS_LEN: usize = 272;

/// Pick arp pattern (0x0c, 203 bytes after F0) and globals (0x0f, 280 bytes after F0) dumps out of a message list.
pub fn parse_extras(msgs: &[Vec<u8>]) -> (Vec<ArpPattern>, Option<Globals>) {
    let mut pats = Vec::new();
    let mut glob = None;
    for m in msgs {
        match nova_type(m) {
            Some(0x0c) if m.len() == 203 => {
                pats.push(ArpPattern { idx: m[7], body: m[8..8 + PATTERN_LEN].to_vec(), chan: m[5] })
            }
            Some(0x0f) if m.len() == 280 => glob = Some(Globals { body: m[7..7 + GLOBALS_LEN].to_vec(), chan: m[5] }),
            _ => {}
        }
    }
    (pats, glob)
}

pub fn parse_extras_file(d: &[u8]) -> (Vec<ArpPattern>, Option<Globals>) {
    parse_extras(&messages_of(d))
}

pub fn build_type0c(idx: u8, body: &[u8], chan: u8) -> Vec<u8> {
    let mut v = vec![0xf0, 0x00, 0x20, 0x29, 0x01, 0x21, chan, 0x0c, idx & 0x7f];
    v.extend_from_slice(&body[..PATTERN_LEN]);
    v.push(0xf7);
    v
}

pub fn build_type0f(body: &[u8], chan: u8) -> Vec<u8> {
    let mut v = vec![0xf0, 0x00, 0x20, 0x29, 0x01, 0x21, chan, 0x0f];
    v.extend_from_slice(&body[..GLOBALS_LEN]);
    v.push(0xf7);
    v
}

/// Parse a file into program and performance dumps (bundles included).
pub fn parse_all(d: &[u8]) -> (Vec<Program>, Vec<Performance>) {
    parse_stream(&messages_of(d))
}

pub fn build_type5(body: &[u8], chan: u8) -> Vec<u8> {
    let mut v = vec![0xf0, 0x00, 0x20, 0x29, 0x01, 0x21, chan, 0x05];
    v.extend_from_slice(&body[..crate::perf::PERF_LEN]);
    v.push(0xf7);
    v
}

/// Emit a bundle: present part dumps in index order, the type-5, then the extra messages.
pub fn bundle_to_syx(body: &[u8], b: &Bundle) -> Vec<u8> {
    let mut v = Vec::new();
    for (i, p) in b.parts.iter().enumerate() {
        if let Some(p) = p {
            v.extend(build_type0(p, i as u8, b.chan));
        }
    }
    v.extend(build_type5(body, b.chan));
    for e in &b.extra {
        v.extend_from_slice(e);
    }
    v
}

/// Parse a live-received performance dump (full message incl. F0/F7).
pub fn parse_live_perf(full: &[u8]) -> Option<Performance> {
    if full.first() != Some(&0xf0) {
        return None;
    }
    parse_perf_messages(&[full[1..].to_vec()]).into_iter().next()
}

pub fn build_type7(body: &[u8], bank: u8, idx: u8, chan: u8) -> Vec<u8> {
    let mut v = vec![0xf0, 0x00, 0x20, 0x29, 0x01, 0x21, chan, 0x07, bank & 0x7f, idx & 0x7f];
    v.extend_from_slice(&body[..crate::perf::PERF_LEN]);
    v.push(0xf7);
    v
}

/// The 27-byte companion (type 0x1c) that must follow every performance bank dump, or the Nova
/// reports "Missing data packet". Bytes after the index are the stock Nova OS 4.1 values;
/// the 5th of them is the vocoder balance (0 = no "[V]").
pub fn build_perf_companion(bank: u8, idx: u8, chan: u8) -> Vec<u8> {
    let mut v = vec![0xf0, 0x00, 0x20, 0x29, 0x01, 0x21, chan, 0x1c, bank & 0x7f, idx & 0x7f];
    v.extend_from_slice(&[0x21, 0x12, 0x1c, 0x7f, 0x00, 0x03, 0x6f]);
    v.extend_from_slice(&[0; 9]);
    v.push(0xf7);
    v
}

/// Dump requests found in the firmware (SysEx dispatcher 0x15c52, 9-byte messages). Each one makes the Nova run the
/// matching Global > Sysex transmission action; nothing on the Nova is changed. Request type -> action:
/// 3 single program, 4 all programs, 8 single performance, 9 all performances (13/14 arp patterns,
/// 16 globals, 17 total data are not exposed).
pub fn dump_request(kind: u8, chan: u8) -> [u8; 9] {
    [0xf0, 0x00, 0x20, 0x29, 0x01, 0x21, chan & 0x7f, kind, 0xf7]
}

/// Extract SysEx messages (bytes after F0, ending with F7) from an SMF.
pub fn parse_smf(d: &[u8]) -> Vec<Vec<u8>> {
    let mut msgs = Vec::new();
    let mut i = 0usize;
    // Skip the header chunk(s) and walk all chunks.
    while i + 8 <= d.len() {
        let id = &d[i..i + 4];
        let len = u32::from_be_bytes([d[i + 4], d[i + 5], d[i + 6], d[i + 7]]) as usize;
        let start = i + 8;
        let end = (start + len).min(d.len());
        if id == b"MTrk" {
            parse_track(&d[start..end], &mut msgs);
        }
        i = start + len;
    }
    msgs
}

fn parse_track(t: &[u8], msgs: &mut Vec<Vec<u8>>) {
    let mut i = 0usize;
    let mut status = 0u8;
    let vlq = |i: &mut usize| -> Option<usize> {
        let mut v = 0usize;
        loop {
            let b = *t.get(*i)?;
            *i += 1;
            v = (v << 7) | (b & 0x7f) as usize;
            if b & 0x80 == 0 {
                return Some(v);
            }
        }
    };
    while i < t.len() {
        if vlq(&mut i).is_none() {
            return;
        }
        let Some(&s) = t.get(i) else { return };
        match s {
            0xff => {
                i += 2;
                let Some(l) = vlq(&mut i) else { return };
                i += l;
            }
            0xf0 | 0xf7 => {
                i += 1;
                let Some(l) = vlq(&mut i) else { return };
                if i + l > t.len() {
                    return;
                }
                msgs.push(t[i..i + l].to_vec());
                i += l;
            }
            _ => {
                // channel event (with running status support)
                if s & 0x80 != 0 {
                    status = s;
                    i += 1;
                }
                i += match status & 0xf0 {
                    0xc0 | 0xd0 => 1,
                    _ => 2,
                };
            }
        }
    }
}

/// Raw .syx: split on F0..F7.
pub fn parse_syx(d: &[u8]) -> Vec<Vec<u8>> {
    let mut msgs = Vec::new();
    let mut start: Option<usize> = None;
    for (i, &b) in d.iter().enumerate() {
        if b == 0xf0 {
            start = Some(i + 1);
        } else if b == 0xf7 {
            if let Some(s) = start.take() {
                msgs.push(d[s..=i].to_vec());
            }
        }
    }
    msgs
}

/// Interpret messages (bytes after F0) as Nova program dumps; other types are ignored.
pub fn parse_messages(msgs: &[Vec<u8>]) -> Vec<Program> {
    let mut out = Vec::new();
    for m in msgs {
        if m.len() < 8 || m[0] != 0x00 || m[1] != 0x20 || m[2] != 0x29 || m[3] != 0x01 || m[4] != 0x21 {
            continue;
        }
        let ty = m[6];
        if ty == 2 && m.len() == 296 {
            out.push(Program {
                body: m[9..295].to_vec(),
                bank: Some(m[7]),
                idx: m[8],
                label: format!("bank {} #{}", m[7], m[8]),
            });
        } else if ty == 0 && m.len() == 295 {
            out.push(Program {
                body: m[8..294].to_vec(),
                bank: None,
                idx: m[7],
                label: format!("part {}", m[7]),
            });
        }
    }
    out
}

/// Parse a whole file (SMF or raw .syx, detected by the first byte).
pub fn parse_file(d: &[u8]) -> Vec<Program> {
    let msgs = if d.first() == Some(&0xf0) { parse_syx(d) } else { parse_smf(d) };
    parse_messages(&msgs)
}

/// Parse a live-received SysEx (full message including F0 and F7).
pub fn parse_live(full: &[u8]) -> Option<Program> {
    if full.first() != Some(&0xf0) {
        return None;
    }
    parse_messages(&[full[1..].to_vec()]).into_iter().next()
}

pub fn build_type2(body: &[u8], bank: u8, idx: u8, chan: u8) -> Vec<u8> {
    let mut v = vec![0xf0, 0x00, 0x20, 0x29, 0x01, 0x21, chan, 0x02, bank & 0x7f, idx & 0x7f];
    v.extend_from_slice(&body[..N]);
    v.push(0xf7);
    v
}

pub fn build_type0(body: &[u8], part: u8, chan: u8) -> Vec<u8> {
    let mut v = vec![0xf0, 0x00, 0x20, 0x29, 0x01, 0x21, chan, 0x00, part & 0x7f];
    v.extend_from_slice(&body[..N]);
    v.push(0xf7);
    v
}

/// Type-0 part dump for sending. Refuses wrong body sizes and the parts the firmware rejects (6, 7).
pub fn program_send_message(body: &[u8], part: u8, chan: u8) -> Result<Vec<u8>, String> {
    if body.len() != N {
        return Err(format!("program body is {} bytes, expected {N}", body.len()));
    }
    if part == 6 || part == 7 || part > 9 {
        return Err(format!("part {part} is not accepted by the Nova (use 0-5, 8 or 9)"));
    }
    if chan > 15 {
        return Err("SysEx channel must be 0-15".into());
    }
    Ok(build_type0(body, part, chan))
}

/// Messages for sending a performance bundle: type-0 dumps for parts 0..5 (those present), then the type-5.
/// Parts 6/7 and the extra (0x1b) messages are not sent.
pub fn bundle_send_messages(body: &[u8], b: &Bundle, chan: u8) -> Result<Vec<Vec<u8>>, String> {
    if body.len() != crate::perf::PERF_LEN {
        return Err(format!("performance body is {} bytes, expected {}", body.len(), crate::perf::PERF_LEN));
    }
    if chan > 15 {
        return Err("SysEx channel must be 0-15".into());
    }
    let mut v = Vec::new();
    for (i, p) in b.parts.iter().enumerate().take(6) {
        if let Some(p) = p {
            if p.len() != N {
                return Err(format!("embedded program for part {} is {} bytes, expected {N}", i + 1, p.len()));
            }
            v.push(build_type0(p, i as u8, chan));
        }
    }
    v.push(build_type5(body, chan));
    Ok(v)
}

/// Split a message into pieces of at most `size` bytes. Not used for sending: partial SysEx
/// across several send() calls is not reliable with every backend, so messages go out whole.
#[allow(dead_code)]
pub fn chunks(msg: &[u8], size: usize) -> Vec<&[u8]> {
    msg.chunks(size.max(1)).collect()
}

#[allow(dead_code)]
/// Bank C/D unlock found in the OS 4.1 firmware (handler 0x1675a): SysEx type 0x1d with data 0x7f,
/// accepted only after a CC99 (NRPN MSB) = 127 and only for model 0x21. See tools/NOVA_SYSEX.md.
pub fn unlock_banks_messages(chan: u8) -> ([u8; 3], [u8; 10]) {
    let c = chan & 0x0f;
    ([0xb0 | c, 99, 0x7f], [0xf0, 0x00, 0x20, 0x29, 0x01, 0x21, c, 0x1d, 0x7f, 0xf7])
}

#[allow(dead_code)]
pub const REQUEST_CURRENT: [u8; 12] =
    [0xf0, 0x00, 0x20, 0x29, 0x01, 0x21, 0x00, 0x01, 0x09, 0x00, 0x00, 0xf7];

/// Learn: compare two bodies (bytes 16..286). Returns the byte with the largest
/// change plus all changed offsets, or None when nothing changed.
pub fn learn_diff(prev: &[u8], new: &[u8]) -> Option<(usize, Vec<usize>)> {
    let diffs: Vec<usize> = (16..N.min(prev.len()).min(new.len())).filter(|&i| prev[i] != new[i]).collect();
    let best = diffs
        .iter()
        .copied()
        .max_by_key(|&i| (prev[i] as i32 - new[i] as i32).abs())?;
    Some((best, diffs))
}

#[cfg(test)]
mod tests {
    #[test]
    fn companion_matches_stock_layout() {
        let c = super::build_perf_companion(1, 0, 0);
        assert_eq!(c.len(), 27);
        assert_eq!(&c[..14], &[0xf0, 0, 0x20, 0x29, 1, 0x21, 0, 0x1c, 1, 0, 0x21, 0x12, 0x1c, 0x7f]);
    }

    use super::*;

    const TOTAL: &[u8] = include_bytes!("../assets/total.mid");

    #[test]
    fn total_mid_parses() {
        let progs = parse_file(TOTAL);
        assert_eq!(progs.len(), 256);
        assert_eq!(progs.iter().filter(|p| p.bank == Some(5)).count(), 128);
        assert_eq!(progs.iter().filter(|p| p.bank == Some(6)).count(), 128);
        assert!(progs.iter().all(|p| p.body.len() == 286));
    }

    #[test]
    fn type2_roundtrip() {
        let progs = parse_file(TOTAL);
        let p = &progs[7];
        let msg = build_type2(&p.body, 6, 42, 0);
        assert_eq!(msg.len(), 297);
        let back = parse_file(&msg);
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].body, p.body);
        assert_eq!(back[0].bank, Some(6));
        assert_eq!(back[0].idx, 42);
        let t0 = build_type0(&p.body, 9, 0);
        assert_eq!(t0.len(), 296);
        let back = parse_live(&t0).unwrap();
        assert_eq!(back.body, p.body);
    }

    #[test]
    fn performances_parse_and_roundtrip() {
        let (progs, perfs) = parse_all(TOTAL);
        assert_eq!(progs.len(), 256);
        assert_eq!(perfs.len(), 128);
        assert!(perfs.iter().all(|p| p.body.len() == 372));
        let msg = build_type7(&perfs[3].body, 1, 3, 0);
        assert_eq!(msg.len(), 383);
        let back = parse_live_perf(&msg).unwrap();
        assert_eq!(back.body, perfs[3].body);
        assert_eq!((back.bank, back.idx), (1, 3));
        let (p2, f2) = parse_all(&msg);
        assert!(p2.is_empty());
        assert_eq!(f2.len(), 1);
    }

    const FIXTURE: &[u8] = include_bytes!("../tests/data/nova_single_performance_multi_ch_1-6.syx");

    #[test]
    fn bundle_fixture() {
        let (progs, perfs) = parse_all(FIXTURE);
        assert!(progs.is_empty());
        assert_eq!(perfs.len(), 1);
        let p = &perfs[0];
        let b = p.bundle.as_ref().unwrap();
        assert_eq!(p.name(), "Multi Ch 1-6");
        assert_eq!(b.chan, 0x0f);
        assert_eq!(b.extra.len(), 1);
        let names: Vec<String> = b
            .parts
            .iter()
            .map(|x| String::from_utf8_lossy(&x.as_ref().unwrap()[..16]).trim().to_string())
            .collect();
        assert_eq!(
            names,
            ["Sonar (C3)  M-Wh", "Choral Pad  M-Wh", "Simple Arp  M-Wh", "SimpleEnvVeloArp", "Synthi Harp M-Wh", "FM EP", "Synthi Harp M-Wh", "FM EP"]
        );
        let o = crate::perf::part_off(0);
        assert_eq!(p.body[o + crate::perf::rel::PROG_NUM], 53);
        assert_eq!(p.body[o + crate::perf::rel::PROG_BANK], 5);
        assert_eq!(p.body[o + crate::perf::rel::CHAN], 2);
        assert_eq!(p.body[o + crate::perf::rel::LEVEL], 122);
        assert_eq!(crate::perf::poly_get(&p.body, 0), 1);
        assert_eq!(bundle_to_syx(&p.body, b), FIXTURE);
        // editing one embedded byte changes exactly that byte
        let mut b2 = b.clone();
        b2.parts[2].as_mut().unwrap()[0xd3] ^= 0x10;
        let out = bundle_to_syx(&p.body, &b2);
        let diff: Vec<usize> = (0..out.len()).filter(|&i| out[i] != FIXTURE[i]).collect();
        assert_eq!(diff.len(), 1);
    }

    #[test]
    fn backup_file() {
        let d = std::fs::read("tests/data/nova_backup_all.syx").unwrap();
        let (progs, perfs) = parse_all(&d);
        assert_eq!(progs.len(), 256);
        assert_eq!(progs.iter().filter(|p| p.bank == Some(5)).count(), 128);
        assert_eq!(progs.iter().filter(|p| p.bank == Some(6)).count(), 128);
        assert_eq!(perfs.len(), 128);
        assert!(perfs.iter().all(|p| p.bank == 1 && p.bundle.is_none()));
        let find = |b: u8, i: u8| progs.iter().find(|p| p.bank == Some(b) && p.idx == i).unwrap();
        assert_eq!(find(5, 0).name(), "Sintillator M-Wh");
        assert_eq!(find(6, 122).name(), "Simple Arp  M-Wh");
        assert_eq!(find(6, 122).body[0xd3], 53);
        assert_eq!(perfs[0].idx, 0);
        assert_eq!(perfs[0].name(), "Multi Ch 1-6");
    }

    #[test]
    fn unlock_message_layout() {
        let (cc, sx) = unlock_banks_messages(15);
        assert_eq!(cc, [0xbf, 99, 127]);
        assert_eq!(sx.len(), 10);
        assert_eq!(&sx[..8], &[0xf0, 0, 0x20, 0x29, 1, 0x21, 15, 0x1d]);
        assert_eq!(sx[8..], [0x7f, 0xf7]);
    }

    #[test]
    fn send_builders() {
        let body = vec![1u8; 286];
        let m = program_send_message(&body, 9, 0x0f).unwrap();
        assert_eq!(m.len(), 296);
        assert_eq!(&m[..9], &[0xf0, 0x00, 0x20, 0x29, 0x01, 0x21, 0x0f, 0x00, 9]);
        assert_eq!(*m.last().unwrap(), 0xf7);
        assert!(program_send_message(&body, 6, 0).is_err());
        assert!(program_send_message(&body, 7, 0).is_err());
        assert!(program_send_message(&body[..285], 0, 0).is_err());
        assert!(program_send_message(&body, 0, 16).is_err());
        let (_, perfs) = parse_all(FIXTURE);
        let b = perfs[0].bundle.as_ref().unwrap();
        let msgs = bundle_send_messages(&perfs[0].body, b, 0x0f).unwrap();
        assert_eq!(msgs.len(), 7); // parts 1-6 then the type-5
        assert!(msgs[..6].iter().enumerate().all(|(i, m)| m.len() == 296 && m[7] == 0 && m[8] == i as u8));
        assert_eq!((msgs[6].len(), msgs[6][7]), (381, 5));
        assert!(bundle_send_messages(&perfs[0].body[..371], b, 0).is_err());
        let c = chunks(&msgs[0], 64);
        assert_eq!(c.len(), 5);
        assert_eq!(c.concat(), msgs[0]);
    }

    #[test]
    fn backup_patterns_and_globals() {
        let d = std::fs::read("tests/data/nova_backup_all.syx").unwrap();
        let (pats, glob) = parse_extras_file(&d);
        assert_eq!(pats.len(), 102);
        assert_eq!(pats.iter().map(|p| p.idx).min(), Some(26));
        assert_eq!(pats.iter().map(|p| p.idx).max(), Some(127));
        assert!(pats.iter().all(|p| p.body.len() == 194));
        let g = glob.unwrap();
        assert_eq!(g.body.len(), 272);
        let msg = build_type0f(&g.body, g.chan);
        assert_eq!(msg.len(), 281);
        assert_eq!(parse_extras(&[msg[1..].to_vec()]).1.unwrap(), g);
        let pm = build_type0c(pats[0].idx, &pats[0].body, pats[0].chan);
        assert_eq!(pm.len(), 204);
        assert_eq!(parse_extras(&[pm[1..].to_vec()]).0[0], pats[0]);
    }

    #[test]
    fn learn_picks_changed_byte() {
        let a = vec![0u8; 286];
        let mut b = a.clone();
        b[0x40] = 5;
        b[0x90] = 100;
        b[3] = 99; // header name byte is ignored
        let (best, all) = learn_diff(&a, &b).unwrap();
        assert_eq!(best, 0x90);
        assert_eq!(all, vec![0x40, 0x90]);
        let mut c = a.clone();
        c[0x44] = 1;
        assert_eq!(learn_diff(&a, &c).unwrap().0, 0x44);
        assert!(learn_diff(&a, &a).is_none());
    }
}
