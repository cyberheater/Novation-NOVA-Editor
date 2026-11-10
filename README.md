# Novation NOVA Editor

A front-panel editor and librarian for the **Novation Nova** synthesiser (OS 4.1), for macOS.
Written in Rust with egui.


## What it does

- **Front panel** that looks and works like the hardware: knobs, buttons, LEDs and display for
  every section (oscillators, filters, LFOs, envelopes, arpeggiator, Part edit, effects).
- **Live edit**: every knob and button change is sent to the Nova as CC/NRPN straight away.
  In Performance mode, edits go to the selected Part on its own MIDI channel. Only the Nova's
  edit buffer changes; press Write on the Nova to store a sound.
- **Mirror**: moving controls on the Nova updates the editor.
- **Display like the hardware**: name on top, `Prog A000` / `Perf A000` right-aligned underneath.
- **Choosing programs**: Bank/Prog up/down and the lower Fast Data knob select programs. In
  Performance mode, press a Part button and they change that Part's program (the Nova follows on
  the Part's MIDI channel); press perf again to step through performances.
- **Library kept between launches**: retrieved or loaded programs and performances are saved
  automatically and loaded at the next start.
- **Connects by itself**: remembers your MIDI ports, reconnects at start and when the interface
  is plugged back in, and reads the Nova's global MIDI channel on connect. A lamp shows the
  connection state.
- **Dump** menu:
  - Retrieve from the Nova: current program / performance, all programs, all performances,
    everything (about 1-2 minutes).
  - Send to the Nova: current item (edit buffer), a program bank, all programs, all performances,
    everything (with a confirmation; bank sends overwrite stored slots).
  - Save and load `.syx` / `.mid` files, merge files into the library, or reload the factory set.
- **Performance sets** for multitimbral use: give each of 42 performances (A000-A041) its own six
  programs in banks C/D, fill A001-A041 from A000, export or send the whole set.
- **Unlock banks C/D** (the firmware's hidden memory-expansion switch), with a confirmation.
- **Step sequencer** (the **seq** button): 8 tracks on MIDI channels 1-8, laid out like a
  Launchpad. 32 projects, each with 16 patterns of up to 64 steps (a pattern holds all 8 tracks);
  chords of up to 8 notes, velocity, gate and probability per step, swing, per-track step rate and
  transpose, live and step recording from a MIDI keyboard, tap tempo and optional MIDI clock out.
  It plays through the editor's MIDI output and keeps playing while you edit sounds on the panel.
  Projects are saved automatically. A **Launchpad Pro MK3** connected over USB is picked up and
  used as the sequencer's controller (it is put into Programmer mode).

## Download

Get `Novation-NOVA-Editor-macOS.zip` from the [Releases](../../releases) page, unzip it and move
**Nova Editor.app** to Applications. The app is not notarised: the first time, right-click it and
choose **Open**, then **Open** again (or allow it under System Settings > Privacy & Security).
Built for Apple silicon Macs.

## Connecting the Nova

Connect the Nova's MIDI In and Out to a MIDI interface. Open the editor, press **midi** on the
panel and choose the interface for Input and Output; it connects at once and is remembered.
For dumps and sends the Nova needs Global > Memory protect = Off and Sysex reception = Normal.

## Build from source

Requires a Rust toolchain (edition 2024).

```
cargo run --release                  # run directly
cargo run --release -- file.syx      # load given .syx/.mid files at startup
cargo test                           # unit tests
scripts/make_app.sh                  # builds dist/Nova Editor.app (ad-hoc signed)
```

## Notes

- `assets/total.mid` is the Nova OS 4.1 factory data (programs, performances, arpeggio patterns),
  used as the built-in library. It belongs to Novation.
- Sending to the Nova uses slow byte pacing and adds the per-performance companion message the
  Nova expects; faster or incomplete dumps make the Nova show "Missing data packet".
- Not affiliated with or endorsed by Novation / Focusrite.
