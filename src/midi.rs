//! MIDI ports (CoreMIDI via midir). Receive SysEx on the callback thread and forward through a channel.
use eframe::egui;
use midir::{MidiInput, MidiInputConnection, MidiOutput, MidiOutputConnection};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};

/// Pause between consecutive SysEx messages when sending.
/// Gap after each message, and byte pacing inside one (the Nova drops fast streams: "Missing data packet").
pub const SEND_GAP_MS: u64 = 300;
pub const SEND_CHUNK: usize = 8;
pub const SEND_CHUNK_MS: u64 = 10;

/// A background send in progress.
pub struct SendJob {
    pub total: usize,
    pub done: Arc<AtomicUsize>,
    pub cancel: Arc<AtomicBool>,
    pub finished: Arc<AtomicBool>,
    pub error: Arc<Mutex<Option<String>>>,
}

/// Receive counters shared with the MIDI callback thread (counted even when Mirror is off).
#[derive(Default)]
pub struct Stats {
    pub chan_msgs: AtomicU64,
    pub sysex_msgs: AtomicU64,
    /// unix time in ms of the last message (0 = none)
    pub last_ms: AtomicU64,
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

impl Stats {
    pub fn note(&self, channel: bool) {
        let c = if channel { &self.chan_msgs } else { &self.sysex_msgs };
        c.fetch_add(1, Ordering::Relaxed);
        self.last_ms.store(now_ms(), Ordering::Relaxed);
    }
    pub fn reset(&self) {
        self.chan_msgs.store(0, Ordering::Relaxed);
        self.sysex_msgs.store(0, Ordering::Relaxed);
        self.last_ms.store(0, Ordering::Relaxed);
    }
    /// (channel msgs, sysex msgs, ms since last message or None)
    pub fn snapshot(&self) -> (u64, u64, Option<u64>) {
        let last = self.last_ms.load(Ordering::Relaxed);
        (
            self.chan_msgs.load(Ordering::Relaxed),
            self.sysex_msgs.load(Ordering::Relaxed),
            (last != 0).then(|| now_ms().saturating_sub(last)),
        )
    }
}

pub fn ago_text(ago_ms: Option<u64>) -> String {
    match ago_ms {
        None => "never".into(),
        Some(d) if d < 1000 => "<1 s ago".into(),
        Some(d) if d < 60_000 => format!("{} s ago", d / 1000),
        Some(d) => format!("{} min ago", d / 60_000),
    }
}

/// Persisted MIDI settings (midi_ports.json). Sending is never persisted.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct PortsConfig {
    pub input: Option<String>,
    pub output: Option<String>,
    pub auto_connect: bool,
    pub mirror_on: bool,
    pub mirror_chan: crate::mirror::ChanMode,
    /// last program selected in the app (restored at start; the Nova cannot be asked)
    pub last_bank: Option<u8>,
    pub last_idx: Option<u8>,
}
impl Default for PortsConfig {
    fn default() -> Self {
        PortsConfig { input: None, output: None, auto_connect: true, mirror_on: false, mirror_chan: crate::mirror::ChanMode::Any, last_bank: None, last_idx: None }
    }
}
impl PortsConfig {
    /// Parses the current struct format, or the old `[input, output]` array format.
    pub fn parse(s: &str) -> PortsConfig {
        if let Ok(c) = serde_json::from_str::<PortsConfig>(s) {
            return c;
        }
        if let Ok((i, o)) = serde_json::from_str::<(Option<String>, Option<String>)>(s) {
            return PortsConfig { input: i, output: o, ..Default::default() };
        }
        PortsConfig::default()
    }
    fn path() -> Option<std::path::PathBuf> {
        crate::files::config_dir().map(|d| d.join("novaedit").join("midi_ports.json"))
    }
    pub fn load() -> PortsConfig {
        Self::path().and_then(|p| std::fs::read_to_string(p).ok()).map(|s| Self::parse(&s)).unwrap_or_default()
    }
    pub fn save(&self) {
        if let Some(p) = Self::path() {
            if let Some(dir) = p.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            if let Ok(s) = serde_json::to_string_pretty(self) {
                let _ = std::fs::write(p, s);
            }
        }
    }
}

/// Index of `old` (a remembered port name) in a refreshed list.
pub fn select_by_name(old: Option<&str>, list: &[String]) -> Option<usize> {
    let old = old?;
    list.iter().position(|n| n == old)
}

pub struct Midi {
    pub stats: Arc<Stats>,
    /// names of the currently connected ports
    pub conn_in: Option<String>,
    pub conn_out: Option<String>,
    pub inputs: Vec<String>,
    pub outputs: Vec<String>,
    in_conn: Option<MidiInputConnection<()>>,
    out_conn: Option<Arc<Mutex<MidiOutputConnection>>>,
    pub rx: Option<Receiver<Vec<u8>>>,
    pub rx_ch: Option<Receiver<crate::mirror::ChannelMsg>>,
    /// number of Identity replies seen (used by the output-port scan)
    pub id_replies: Arc<AtomicUsize>,
}

/// Progress of an output-port scan.
#[derive(Default)]
pub struct Scan {
    pub lines: Mutex<Vec<String>>,
    pub finished: AtomicBool,
}

impl Midi {
    pub fn new() -> Self {
        Midi { stats: Arc::new(Stats::default()), conn_in: None, conn_out: None, inputs: vec![], outputs: vec![], in_conn: None, out_conn: None, rx: None, rx_ch: None, id_replies: Arc::new(AtomicUsize::new(0)) }
    }

    pub fn refresh(&mut self) {
        self.inputs = MidiInput::new("novaedit-list")
            .map(|m| m.ports().iter().filter_map(|p| m.port_name(p).ok()).collect())
            .unwrap_or_default();
        self.outputs = MidiOutput::new("novaedit-list")
            .map(|m| m.ports().iter().filter_map(|p| m.port_name(p).ok()).collect())
            .unwrap_or_default();
    }

    /// Connect by port index (None = leave that direction unconnected).
    pub fn connect(
        &mut self,
        input: Option<usize>,
        output: Option<usize>,
        ctx: egui::Context,
    ) -> Result<String, String> {
        self.in_conn = None;
        self.out_conn = None;
        self.rx = None;
        self.rx_ch = None;
        if let Some(i) = input {
            let mut mi = MidiInput::new("novaedit-in").map_err(|e| e.to_string())?;
            mi.ignore(midir::Ignore::None);
            let port = mi.ports().get(i).cloned().ok_or("input port gone")?;
            let (tx, rx): (Sender<Vec<u8>>, Receiver<Vec<u8>>) = channel();
            let (tx_ch, rx_ch) = channel::<crate::mirror::ChannelMsg>();
            let mut acc: Vec<u8> = Vec::new();
            let mut parser = crate::mirror::MsgParser::default();
            self.stats.reset();
            let stats = self.stats.clone();
            let conn = mi
                .connect(
                    &port,
                    "novaedit-in",
                    move |_ts, data, _| {
                        for &b in data {
                            let in_sysex = !acc.is_empty();
                            if b == 0xf0 {
                                acc.clear();
                                acc.push(b);
                                parser.push(b);
                            } else if in_sysex {
                                if b >= 0xf8 {
                                    continue; // realtime bytes inside sysex
                                }
                                acc.push(b);
                                if b == 0xf7 {
                                    let _ = tx.send(std::mem::take(&mut acc));
                                    stats.note(false);
                                    ctx.request_repaint();
                                } else if b & 0x80 != 0 {
                                    acc.clear(); // aborted by another status
                                }
                            } else if let Some(m) = parser.push(b) {
                                let _ = tx_ch.send(m);
                                stats.note(true);
                                ctx.request_repaint();
                            }
                        }
                    },
                    (),
                )
                .map_err(|e| e.to_string())?;
            self.in_conn = Some(conn);
            self.rx = Some(rx);
            self.rx_ch = Some(rx_ch);
        }
        if let Some(o) = output {
            let mo = MidiOutput::new("novaedit-out").map_err(|e| e.to_string())?;
            let port = mo.ports().get(o).cloned().ok_or("output port gone")?;
            self.out_conn = Some(Arc::new(Mutex::new(mo.connect(&port, "novaedit-out").map_err(|e| e.to_string())?)));
        }
        self.conn_in = input.and_then(|i| self.inputs.get(i).cloned());
        self.conn_out = output.and_then(|i| self.outputs.get(i).cloned());
        Ok(format!(
            "MIDI ready - in: {}, out: {}",
            input.and_then(|i| self.inputs.get(i)).map(String::as_str).unwrap_or("none"),
            output.and_then(|i| self.outputs.get(i)).map(String::as_str).unwrap_or("none"),
        ))
    }

    /// Drop both connections and clear the receive queues.
    pub fn disconnect(&mut self) {
        self.in_conn = None;
        self.out_conn = None;
        self.rx = None;
        self.rx_ch = None;
        self.conn_in = None;
        self.conn_out = None;
    }

    /// Send one Identity Request to each output port in turn (700 ms apart) and report which
    /// port the Nova answers on. Only ever started by an explicit button click.
    pub fn scan_outputs(&self, ctx: egui::Context) -> Arc<Scan> {
        let scan = Arc::new(Scan::default());
        let (s, replies) = (scan.clone(), self.id_replies.clone());
        std::thread::spawn(move || {
            let n = MidiOutput::new("novaedit-scan").map(|m| m.ports().len()).unwrap_or(0);
            for i in 0..n {
                let Ok(mo) = MidiOutput::new("novaedit-scan") else { break };
                let Some(port) = mo.ports().get(i).cloned() else { break };
                let name = mo.port_name(&port).unwrap_or_else(|_| format!("port {i}"));
                let line = match mo.connect(&port, "novaedit-scan") {
                    Ok(mut c) => {
                        let before = replies.load(Ordering::SeqCst);
                        let _ = c.send(&[0xf0, 0x7e, 0x7f, 0x06, 0x01, 0xf7]);
                        std::thread::sleep(std::time::Duration::from_millis(700));
                        if replies.load(Ordering::SeqCst) > before { format!("{name}: REPLY - the Nova is on this output") } else { format!("{name}: no reply") }
                    }
                    Err(e) => format!("{name}: could not open ({e})"),
                };
                s.lines.lock().unwrap().push(line);
                ctx.request_repaint();
            }
            s.finished.store(true, Ordering::SeqCst);
            ctx.request_repaint();
        });
        scan
    }

    pub fn has_output(&self) -> bool {
        self.out_conn.is_some()
    }

    pub fn send(&mut self, data: &[u8]) -> Result<(), String> {
        match self.out_conn.as_ref() {
            Some(c) => c.lock().map_err(|e| e.to_string())?.send(data).map_err(|e| e.to_string()),
            None => Err("no MIDI output connected".into()),
        }
    }

    /// Send messages on a background thread, SEND_GAP_MS apart. Only ever called after an explicit
    /// user confirmation.
    pub fn start_send(&self, msgs: Vec<Vec<u8>>, ctx: egui::Context) -> Result<SendJob, String> {
        let conn = self.out_conn.clone().ok_or("no MIDI output connected")?;
        let job = SendJob {
            total: msgs.len(),
            done: Arc::new(AtomicUsize::new(0)),
            cancel: Arc::new(AtomicBool::new(false)),
            finished: Arc::new(AtomicBool::new(false)),
            error: Arc::new(Mutex::new(None)),
        };
        let (done, cancel, finished, error) = (job.done.clone(), job.cancel.clone(), job.finished.clone(), job.error.clone());
        std::thread::spawn(move || {
            for m in msgs {
                if cancel.load(Ordering::SeqCst) {
                    break;
                }
                let mut r = Ok(());
                for chunk in m.chunks(SEND_CHUNK) {
                    if cancel.load(Ordering::SeqCst) {
                        break;
                    }
                    r = conn.lock().map_err(|e| e.to_string()).and_then(|mut c| c.send(chunk).map_err(|e| e.to_string()));
                    if r.is_err() {
                        break;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(SEND_CHUNK_MS));
                }
                if let Err(e) = r {
                    *error.lock().unwrap() = Some(e);
                    break;
                }
                done.fetch_add(1, Ordering::SeqCst);
                ctx.request_repaint();
                let mut waited = 0;
                while waited < SEND_GAP_MS && !cancel.load(Ordering::SeqCst) {
                    std::thread::sleep(std::time::Duration::from_millis(10));
                    waited += 10;
                }
            }
            finished.store(true, Ordering::SeqCst);
            ctx.request_repaint();
        });
        Ok(job)
    }

    pub fn poll_channel(&mut self) -> Vec<crate::mirror::ChannelMsg> {
        match &self.rx_ch {
            Some(rx) => rx.try_iter().collect(),
            None => vec![],
        }
    }

    pub fn poll(&mut self) -> Vec<Vec<u8>> {
        match &self.rx {
            Some(rx) => rx.try_iter().collect(),
            None => vec![],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counters_and_ago() {
        let s = Stats::default();
        assert_eq!(s.snapshot(), (0, 0, None));
        s.note(true);
        s.note(true);
        s.note(false);
        let (c, x, ago) = s.snapshot();
        assert_eq!((c, x), (2, 1));
        assert!(ago.unwrap() < 1000);
        s.reset();
        assert_eq!(s.snapshot(), (0, 0, None));
        assert_eq!(ago_text(None), "never");
        assert_eq!(ago_text(Some(400)), "<1 s ago");
        assert_eq!(ago_text(Some(12_000)), "12 s ago");
        assert_eq!(ago_text(Some(125_000)), "2 min ago");
    }

    #[test]
    fn ports_config_roundtrip() {
        let c = PortsConfig {
            input: Some("U6MIDI Pro Port 1".into()),
            output: None,
            auto_connect: true,
            mirror_on: true,
            mirror_chan: crate::mirror::ChanMode::Ch(7),
            last_bank: Some(5),
            last_idx: Some(4),
        };
        let s = serde_json::to_string(&c).unwrap();
        assert_eq!(PortsConfig::parse(&s), c);
        // old tuple format still loads, defaults elsewhere
        let old = PortsConfig::parse(r#"["In A", null]"#);
        assert_eq!(old.input.as_deref(), Some("In A"));
        assert!(old.auto_connect && !old.mirror_on);
        assert_eq!(PortsConfig::parse("garbage"), PortsConfig::default());
    }

    #[test]
    fn selection_by_name() {
        let list = vec!["U6MIDI Pro Port 1".to_string(), "U6MIDI Pro Port 2".to_string(), "U6MIDI Pro Port 3".to_string()];
        assert_eq!(select_by_name(Some("U6MIDI Pro Port 2"), &list), Some(1));
        assert_eq!(select_by_name(Some("Gone"), &list), None);
        assert_eq!(select_by_name(None, &list), None);
        assert_eq!(select_by_name(Some("x"), &[]), None);
    }
}
