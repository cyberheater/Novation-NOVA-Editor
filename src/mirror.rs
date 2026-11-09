//! Receive-only "mirror hardware": maps incoming CC / NRPN messages to program bytes.
use crate::params::Param;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ChannelMsg {
    pub status: u8,
    pub d1: u8,
    pub d2: u8,
}
impl ChannelMsg {
    pub fn chan(&self) -> u8 {
        self.status & 0x0f
    }
}

/// Byte-stream parser with running status; real-time bytes are skipped, SysEx/system bytes clear status.
#[derive(Default)]
pub struct MsgParser {
    status: u8,
    data: [u8; 2],
    n: usize,
}
impl MsgParser {
    pub fn push(&mut self, b: u8) -> Option<ChannelMsg> {
        if b >= 0xf8 {
            return None;
        }
        if b >= 0xf0 {
            self.status = 0;
            self.n = 0;
            return None;
        }
        if b & 0x80 != 0 {
            self.status = b;
            self.n = 0;
            return None;
        }
        if self.status == 0 {
            return None;
        }
        self.data[self.n] = b;
        self.n += 1;
        let need = if matches!(self.status & 0xf0, 0xc0 | 0xd0) { 1 } else { 2 };
        if self.n == need {
            self.n = 0;
            Some(ChannelMsg { status: self.status, d1: self.data[0], d2: if need == 2 { self.data[1] } else { 0 } })
        } else {
            None
        }
    }
}

/// Which channel(s) to accept.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Accept {
    Any,
    Chan(u8),
}

/// Mirror channel setting.
#[derive(Clone, Copy, PartialEq, Debug, serde::Serialize, serde::Deserialize)]
pub enum ChanMode {
    Any,
    Global,
    /// 0..=15
    Ch(u8),
    /// Performance mode: the selected part's MIDI channel (Program mode: global channel)
    Follow,
}

impl ChanMode {
    pub fn label(&self) -> String {
        match self {
            ChanMode::Any => "Any".into(),
            ChanMode::Global => "Global".into(),
            ChanMode::Ch(c) => format!("Ch {}", c + 1),
            ChanMode::Follow => "Follow part".into(),
        }
    }
}

/// Resolve the setting to a filter; None = ignore everything (Follow without a selected part).
/// `part_chan` is the selected part's raw channel byte (0..15, 16 omni, 17 global).
pub fn accept_for(mode: ChanMode, global: u8, part_chan: Option<u8>, perf: bool) -> Option<Accept> {
    match mode {
        ChanMode::Any => Some(Accept::Any),
        ChanMode::Global => Some(Accept::Chan(global)),
        ChanMode::Ch(c) => Some(Accept::Chan(c)),
        ChanMode::Follow if perf => match part_chan? {
            c @ 0..=15 => Some(Accept::Chan(c)),
            17 => Some(Accept::Chan(global)),
            _ => Some(Accept::Any),
        },
        ChanMode::Follow => Some(Accept::Chan(global)),
    }
}

/// Packed NRPN (LSB 1) code -> (program byte, value, bit-0-only?).
pub fn packed_target(code: u8) -> Option<(usize, u8, bool)> {
    let (off, bit0) = match code {
        0..=2 => (0xd0, false),
        3..=14 => (0xcf, false),
        16..=19 => (0xa1, false),
        20..=23 => (0xab, false),
        24 | 25 => (0xa8, false),
        26 | 27 => (0xb2, false),
        34..=41 => (0xeb, false),
        42..=51 => (0xe8, false),
        72 | 73 => (0xfa, false),
        74 | 75 => (0xf3, false),
        76..=78 => (0xf0, false),
        79 | 80 => (0xf8, false),
        81..=85 => (0xf5, false),
        86..=89 => (0xf2, false),
        90 | 91 => (0xf9, true),
        92 | 93 => (0xfb, false),
        _ => return None,
    };
    Some((off, code, bit0))
}

#[derive(Default)]
pub struct Engine {
    lsb: [Option<u8>; 16],
}

fn name_of(params: &[Param], off: usize) -> String {
    params.get(off).map(|p| p.name.clone()).filter(|s| !s.is_empty()).unwrap_or_else(|| format!("byte 0x{off:02x}"))
}

impl Engine {
    /// Process one message. Returns a log line (None = silently ignored). Writes into `prog`
    /// when mapped (and `writable`).
    pub fn process(&mut self, m: ChannelMsg, accept: Accept, prog: &mut [u8], params: &[Param], writable: bool) -> Option<String> {
        if let Accept::Chan(c) = accept {
            if m.chan() != c {
                // only controller / program-change traffic is worth a log line
                return matches!(m.status & 0xf0, 0xb0 | 0xc0).then(|| "ignored (channel filter)".to_string());
            }
        }
        match m.status & 0xf0 {
            0xc0 => Some(format!("Program change {} (ignored)", m.d1)),
            0xb0 => self.control(m, prog, params, writable),
            _ => None,
        }
    }

    fn control(&mut self, m: ChannelMsg, prog: &mut [u8], params: &[Param], writable: bool) -> Option<String> {
        let ch = m.chan() as usize;
        match m.d1 {
            0 | 32 => Some(format!("Bank select CC {} = {} (ignored)", m.d1, m.d2)),
            99 => Some(format!("NRPN MSB {} (not needed, ignored)", m.d2)),
            100 | 101 => Some(format!("RPN CC {} = {} (ignored)", m.d1, m.d2)),
            38 => Some(format!("Data entry LSB {} (ignored)", m.d2)),
            98 => {
                self.lsb[ch] = Some(m.d2);
                Some(format!("NRPN LSB = {} (stored)", m.d2))
            }
            6 => {
                let Some(n) = self.lsb[ch] else { return Some(format!("Data entry {} without NRPN LSB (unmapped)", m.d2)) };
                Some(self.nrpn(n, m.d2, prog, params, writable))
            }
            cc => {
                let key = format!("CC {cc} ");
                let row = params.iter().find(|p| p.sources.iter().any(|s| s.starts_with(&key)));
                Some(match row {
                    Some(p) => Self::apply(p.offset, m.d2, false, &format!("CC {cc}"), prog, params, writable),
                    None => format!("CC {cc} = {} -> unmapped", m.d2),
                })
            }
        }
    }

    fn nrpn(&mut self, n: u8, data: u8, prog: &mut [u8], params: &[Param], writable: bool) -> String {
        let label = format!("NRPN {n}");
        if n == 1 {
            return match packed_target(data) {
                Some((off, v, bit0)) => Self::apply(off, v, bit0, &format!("NRPN 1 code {data}"), prog, params, writable),
                None => format!("NRPN 1 code {data} -> unmapped (logged only)"),
            };
        }
        if [84, 100, 109, 110, 113].contains(&n) {
            return format!("{label} = {data} -> unmapped (packed, logged only)");
        }
        let key = format!("NRPN {n} (");
        match params.iter().find(|p| p.sources.iter().any(|s| s.starts_with(&key))) {
            Some(p) => Self::apply(p.offset, data, false, &label, prog, params, writable),
            None => format!("{label} = {data} -> unmapped"),
        }
    }

    fn apply(off: usize, v: u8, bit0: bool, what: &str, prog: &mut [u8], params: &[Param], writable: bool) -> String {
        let name = name_of(params, off);
        if !writable {
            return format!("{what} = {v} -> {name} (byte 0x{off:02x}) not applied (read-only)");
        }
        if bit0 {
            let on = v == 91;
            prog[off] = (prog[off] & !1) | on as u8;
            format!("{what} -> {name} (byte 0x{off:02x}) bit0 = {}", on as u8)
        } else {
            prog[off] = v.min(127);
            format!("{what} = {v} -> {name} (byte 0x{off:02x})")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::params;

    fn cc(chan: u8, n: u8, v: u8) -> ChannelMsg {
        ChannelMsg { status: 0xb0 | chan, d1: n, d2: v }
    }

    #[test]
    fn cc_maps_to_byte() {
        let p = params::load();
        let mut e = Engine::default();
        let mut prog = vec![0u8; 286];
        let log = e.process(cc(15, 105, 53), Accept::Chan(15), &mut prog, &p, true).unwrap();
        assert_eq!(prog[0xd3], 53);
        assert!(log.contains("Filter Cutoff Freq"));
        // other channel ignored
        let log = e.process(cc(3, 105, 99), Accept::Chan(15), &mut prog, &p, true).unwrap();
        assert!(log.contains("ignored (channel filter)"));
        assert_eq!(prog[0xd3], 53);
        // unmapped CC is logged
        let log = e.process(cc(15, 9, 1), Accept::Chan(15), &mut prog, &p, true).unwrap();
        assert!(log.contains("unmapped"));
    }

    #[test]
    fn channel_modes() {
        let p = params::load();
        let run = |mode: ChanMode, global: u8| {
            let mut e = Engine::default();
            let mut prog = vec![0u8; 286];
            let acc = accept_for(mode, global, None, false).unwrap();
            let log = e.process(cc(7, 105, 53), acc, &mut prog, &p, true);
            (prog[0xd3], log)
        };
        assert_eq!(run(ChanMode::Any, 15).0, 53); // channel 8 applied
        let (v, log) = run(ChanMode::Global, 15); // global = channel 16
        assert_eq!(v, 0);
        assert!(log.unwrap().contains("channel filter"));
        assert_eq!(run(ChanMode::Ch(7), 15).0, 53); // Ch 8
        assert_eq!(run(ChanMode::Ch(2), 15).0, 0);
        assert_eq!(accept_for(ChanMode::Follow, 15, Some(17), true), Some(Accept::Chan(15)));
        assert_eq!(accept_for(ChanMode::Follow, 15, Some(5), true), Some(Accept::Chan(5)));
        assert_eq!(accept_for(ChanMode::Follow, 15, Some(16), true), Some(Accept::Any));
        assert_eq!(accept_for(ChanMode::Follow, 15, None, true), None);
    }

    #[test]
    fn nrpn_paths() {
        let p = params::load();
        let mut e = Engine::default();
        let mut prog = vec![0u8; 286];
        // 0x54 (84) packed: logged unmapped, nothing written
        assert!(e.process(cc(15, 98, 0x54), Accept::Any, &mut prog, &p, true).unwrap().contains("stored"));
        let log = e.process(cc(15, 6, 0x59), Accept::Any, &mut prog, &p, true).unwrap();
        assert!(log.contains("unmapped"));
        assert!(prog.iter().all(|&b| b == 0));
        // LSB 1 data 2 -> slope
        e.process(cc(15, 98, 1), Accept::Any, &mut prog, &p, true);
        e.process(cc(15, 6, 2), Accept::Any, &mut prog, &p, true);
        assert_eq!(prog[0xd0], 2);
        // 91 -> bit 0 of 0xf9 set, other bits kept
        prog[0xf9] = 4;
        e.process(cc(15, 6, 91), Accept::Any, &mut prog, &p, true);
        assert_eq!(prog[0xf9], 5);
        e.process(cc(15, 6, 90), Accept::Any, &mut prog, &p, true);
        assert_eq!(prog[0xf9], 4);
        // direct NRPN row
        e.process(cc(15, 98, 87), Accept::Any, &mut prog, &p, true); // Filter Mod knob Wh frequency
        e.process(cc(15, 6, 33), Accept::Any, &mut prog, &p, true);
        assert_eq!(prog[0xd4], 33);
        // data entry without LSB on another channel
        let log = e.process(cc(2, 6, 5), Accept::Any, &mut prog, &p, true).unwrap();
        assert!(log.contains("without"));
    }

    #[test]
    fn parser_running_status_and_interleave() {
        let mut ps = MsgParser::default();
        let bytes = [0xbf, 98, 1, 6, 0xf8, 2, 0xcf, 5, 0xf0, 1, 2, 0xf7, 3];
        let msgs: Vec<ChannelMsg> = bytes.iter().filter_map(|&b| ps.push(b)).collect();
        assert_eq!(
            msgs,
            vec![
                ChannelMsg { status: 0xbf, d1: 98, d2: 1 },
                ChannelMsg { status: 0xbf, d1: 6, d2: 2 },
                ChannelMsg { status: 0xcf, d1: 5, d2: 0 },
            ]
        );
    }
}
