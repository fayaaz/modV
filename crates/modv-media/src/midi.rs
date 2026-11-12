//! Raw MIDI ingestion (WS-11), ported from
//! `src/renderer/src/application/setup-midi.js` and
//! `src/renderer/src/application/worker/store/modules/midi.js`.
//!
//! The device model stays in Electron: Web MIDI owns enumeration, hot-plug and
//! the identity `${id}-${name}-${manufacturer}` (`setup-midi.js:141-146`,
//! `store/modules/midi.js:31-42`). The renderer forwards every message over the
//! control socket as the `midi/raw` payload (`docs/rust-hybrid.md`, "The IPC
//! path"), and the engine applies the legacy `midi/WRITE_DATA` semantics:
//!
//! ```js
//! // setup-midi.js:89-114
//! if (_type === TYPE_CC && ccLatch) _type = TYPE_NOTEON;
//! if (_type === TYPE_NOTEON && data > 0) {
//!   _data = device.channelData[channel][_type] === 1 ? 0 : 1;
//! }
//! if (_type === TYPE_CC) _data = _data / 127;
//! // store/modules/midi.js:48-54
//! state.devices[id].channelData[channel][type] = data;
//! ```
//!
//! so [`apply_raw_message`] returns [`MidiEvent`] records whose `address` is the
//! store path of the written value (`devices.<key>.channelData.<ch>.<type>`) and
//! whose `value` is the stored number. The engine maps those onto input links.
//!
//! The clock half (type 248) is *not* a `channelData` value: it feeds the
//! shared [`MidiClockTracker`] (WS-9, `audio/midi_clock.rs`) and an emitted BPM
//! is reported as [`MidiEventKind::Clock`] with `bpm` set (see
//! `setup-midi.js:38-80`; `docs/rust-hybrid.md` moves the clock into the
//! engine because the engine owns the timebase).
//!
//! Deviations from the JS, each deliberate:
//!
//! * **Unknown devices** are created on their first message with the defaults
//!   from `store/modules/midi.js:32-41`. The JS returns early for an unknown
//!   device (`setup-midi.js:29-33`) because enumeration always runs first; the
//!   `midi/raw` IPC must stand on its own if the UI's `ADD_DEVICE` lost the
//!   race (or the device was hot-plugged and the list has not refreshed yet).
//!   `ADD_DEVICE` remains the author of the flags.
//! * **Clock messages never write `channelData`.** The JS `else if` at
//!   `setup-midi.js:82` means a type-248 message from a device with
//!   `listenForClock: false` (the default) would fall through and store
//!   `channelData[ch][248] = 0`. That is an accident of the branch structure
//!   and it is exactly what the plan's "clock → BPM" split removes; a clock is
//!   a transport message, not input data.
//! * **Note-off (128)** is not special-cased, matching the JS: only 144 and 176
//!   have semantics, so a note-off stores its raw `data` under
//!   `channelData[ch][128]` and does not release the note-on latch.
//! * **Values are `f64`** ([`MidiChannelValue`]) because JS numbers have no
//!   int/float split. Note-on and latched CC values are exactly `0.0`/`1.0`,
//!   a plain CC is `data / 127`, and every other message type stores the raw
//!   `data` byte unnormalised.

use std::collections::BTreeMap;
use std::fmt;
use std::time::Instant;

use serde::{Deserialize, Serialize};

use crate::audio::midi_clock::MidiClockTracker;

/// Note-off status byte (`0x80`). Only recorded by the raw path — see the
/// module docs.
pub const TYPE_NOTE_OFF: u8 = 128;

/// Note-on status byte (`0x90`, `setup-midi.js:6`).
pub const TYPE_NOTE_ON: u8 = 144;

/// Control-change status byte (`0xB0`, `setup-midi.js:7`).
pub const TYPE_CONTROL_CHANGE: u8 = 176;

/// MIDI clock status byte (`0xF8`, 24 PPQN, `setup-midi.js:38`).
pub const TYPE_CLOCK: u8 = 248;

/// CC values are normalised by dividing by this (`_data / 127`,
/// `setup-midi.js:112-114`).
pub const CC_VALUE_MAX: f64 = 127.0;

/// The `midi/raw` IPC payload (`docs/rust-hybrid.md`, "The IPC path"):
///
/// ```json
/// { "device": "<id>-<name>-<manufacturer>", "type": 248, "channel": 0, "data": 0 }
/// ```
///
/// `type` carries the raw status byte (`message.data[0]` in `setup-midi.js:25`),
/// `channel` the Web MIDI channel (0-based), and `data` the second data byte
/// (the Web MIDI event's `data[2]`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MidiRawMessage {
    /// The store identity `${id}-${name}-${manufacturer}` (`setup-midi.js:29`).
    pub device: String,
    /// The raw status byte (`setup-midi.js:25`). Serialised as `type` on the wire.
    #[serde(rename = "type")]
    pub type_: u8,
    /// The MIDI channel, 0-based.
    pub channel: u8,
    /// The value / velocity / second data byte.
    pub data: u8,
}

impl MidiRawMessage {
    /// Builds a raw message for the `midi/raw` method.
    pub fn new(device: impl Into<String>, type_: u8, channel: u8, data: u8) -> Self {
        Self {
            device: device.into(),
            type_,
            channel,
            data,
        }
    }

    /// Whether this message is a MIDI clock tick (`setup-midi.js:38`).
    pub fn is_clock(&self) -> bool {
        self.type_ == TYPE_CLOCK
    }
}

/// The store identity `${id}-${name}-${manufacturer}` (`setup-midi.js:29`,
/// `store/modules/midi.js:32`).
pub fn device_key(id: &str, name: &str, manufacturer: &str) -> String {
    format!("{id}-{name}-{manufacturer}")
}

/// A number stored in `channelData[channel][type]` (`store/modules/midi.js:48-54`).
///
/// Serialised as a bare JSON number so the engine's delta stream can replay it
/// into the Vue mirror store, whose shape is
/// `{ devices: { key: { channelData: { channel: { type: number } } } } }`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MidiChannelValue {
    /// The stored JS number.
    pub value: f64,
}

impl MidiChannelValue {
    /// Wraps a stored number.
    pub const fn new(value: f64) -> Self {
        Self { value }
    }

    /// The number as `f64`.
    pub const fn as_f64(self) -> f64 {
        self.value
    }

    /// The JS latch test `channelData[channel][type] === 1`
    /// (`setup-midi.js:97`).
    pub fn is_on(self) -> bool {
        self.value == 1.0
    }
}

impl From<f64> for MidiChannelValue {
    fn from(value: f64) -> Self {
        Self::new(value)
    }
}

impl fmt::Display for MidiChannelValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.value)
    }
}

impl Serialize for MidiChannelValue {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_f64(self.value)
    }
}

impl<'de> Deserialize<'de> for MidiChannelValue {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        f64::deserialize(deserializer).map(Self::new)
    }
}

/// One device of `state.midi.devices` (`store/modules/midi.js:31-42`).
///
/// `id`/`name`/`manufacturer` are not repeated: the map key *is* their
/// concatenation, and the UI plane owns the individual fields.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MidiDeviceState {
    /// `channelData[channel][type]` — only channels/types that received a
    /// message are present (`store/modules/midi.js:48-54`).
    pub channel_data: BTreeMap<i32, BTreeMap<i32, MidiChannelValue>>,
    /// `listenForInput`; default `true` (`store/modules/midi.js:37`).
    pub listen_for_input: bool,
    /// `listenForClock`; default `false` (`store/modules/midi.js:38`).
    pub listen_for_clock: bool,
    /// `ccLatch`: make CCs behave like note-ons (`setup-midi.js:89-91`).
    pub cc_latch: bool,
    /// `noteOnLatch`: ignore note-off so a button stays latched
    /// (`setup-midi.js:107-109`).
    pub note_on_latch: bool,
}

impl MidiDeviceState {
    /// A device with the store's defaults (`store/modules/midi.js:32-41`).
    pub fn new() -> Self {
        Self {
            channel_data: BTreeMap::new(),
            listen_for_input: true,
            listen_for_clock: false,
            cc_latch: false,
            note_on_latch: false,
        }
    }

    /// Writes `channelData[channel][type] = value` (`store/modules/midi.js:48-54`).
    fn write(&mut self, channel: u8, type_: u8, value: f64) {
        self.channel_data
            .entry(i32::from(channel))
            .or_default()
            .insert(i32::from(type_), MidiChannelValue::new(value));
    }

    /// The stored value, if the channel/type pair has been written.
    pub fn value(&self, channel: u8, type_: u8) -> Option<MidiChannelValue> {
        self.channel_data
            .get(&i32::from(channel))
            .and_then(|types| types.get(&i32::from(type_)))
            .copied()
    }
}

impl Default for MidiDeviceState {
    fn default() -> Self {
        Self::new()
    }
}

/// Which legacy code path produced a [`MidiEvent`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MidiEventKind {
    /// An ordinary `channelData` write: a CC normalised by 127, or a raw value
    /// for any other message type.
    Value,
    /// A CC rewritten to a note-on because the device has `ccLatch`
    /// (`setup-midi.js:89-91`); the value is the 0/1 toggle and is stored under
    /// type 144.
    CcLatch,
    /// A note-on latch value (`setup-midi.js:93-110`); stored under type 144.
    NoteOnLatch,
    /// A MIDI clock tick that did not write `channelData`; `bpm` is set when
    /// the 500 ms throttle emitted a new tempo (`setup-midi.js:72-80`).
    Clock,
}

/// One value the engine should apply to its state / input links.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MidiEvent {
    /// Which branch produced the event.
    pub kind: MidiEventKind,
    /// The device identity the event belongs to.
    pub device: String,
    /// The MIDI channel (0-based).
    pub channel: u8,
    /// The type actually written — 144 for a latched CC (`setup-midi.js:89-91`).
    #[serde(rename = "type")]
    pub type_: u8,
    /// The stored number (`channelData[channel][type]`), or `0.0` for a clock.
    pub value: f64,
    /// The lodash path of the written value, e.g.
    /// `devices.<key>.channelData.<ch>.<type>`; empty for a clock event.
    pub address: String,
    /// The BPM a clock tick emitted, when the tracker crossed its throttle.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bpm: Option<u32>,
    /// Whether a MIDI-learn listener is armed (`setup-midi.js:125-130`).
    #[serde(default)]
    pub learning: bool,
}

impl MidiEvent {
    /// The lodash path of a `channelData` value
    /// (`devices.<key>.channelData.<ch>.<type>`).
    ///
    /// The device key is used verbatim; it is the store's own key, so a
    /// device named with a dot produces the same (ambiguous) path the JS
    /// mirror store would have.
    pub fn channel_data_address(device: &str, channel: u8, type_: u8) -> String {
        format!("devices.{device}.channelData.{channel}.{type_}")
    }
}

/// `state.midi`: the per-device latch state plus the shared clock tracker.
///
/// The JS keeps one module-global `clockHistory`/`diffHistory` pair for all
/// devices (`setup-midi.js:3-5`), so the tracker lives on the state, not on a
/// device.
#[derive(Debug)]
pub struct MidiState {
    /// `state.midi.devices`, keyed by `${id}-${name}-${manufacturer}`.
    pub devices: BTreeMap<String, MidiDeviceState>,
    /// `state.midi.learning` — a listener is waiting for the next input
    /// (`store/modules/midi.js:7-18`).
    pub learning: bool,
    /// MIDI-clock → BPM derivation shared by all devices (WS-9).
    clock: MidiClockTracker,
    /// Origin for the monotonic timestamps [`apply_raw_message`] feeds the
    /// clock tracker. [`apply_raw_message_at`] takes the timestamp explicitly.
    clock_origin: Instant,
}

impl MidiState {
    /// An empty device map with no learning listener (`store/modules/midi.js:1-4`).
    pub fn new() -> Self {
        Self {
            devices: BTreeMap::new(),
            learning: false,
            clock: MidiClockTracker::new(),
            clock_origin: Instant::now(),
        }
    }

    /// The device state, creating it with the store defaults if this is the
    /// first message from that device (see the module docs).
    pub fn ensure_device(&mut self, key: &str) -> &mut MidiDeviceState {
        self.devices
            .entry(key.to_string())
            .or_insert_with(MidiDeviceState::new)
    }

    /// Looks a device up without creating it.
    pub fn device(&self, key: &str) -> Option<&MidiDeviceState> {
        self.devices.get(key)
    }

    /// The stored `channelData[channel][type]` value, if any.
    pub fn channel_value(&self, device: &str, channel: u8, type_: u8) -> Option<f64> {
        self.device(device)
            .and_then(|state| state.value(channel, type_))
            .map(MidiChannelValue::as_f64)
    }

    /// The clock tracker (exposed for diagnostics/tests; feed it through
    /// [`apply_raw_message`]).
    pub fn clock(&self) -> &MidiClockTracker {
        &self.clock
    }
}

impl Default for MidiState {
    fn default() -> Self {
        Self::new()
    }
}

/// Applies one `midi/raw` message using the state's monotonic clock origin.
///
/// `device` is authoritative; `message.device` is the IPC copy of the same key
/// and is not read here.
pub fn apply_raw_message(
    state: &mut MidiState,
    device: &str,
    message: &MidiRawMessage,
) -> Vec<MidiEvent> {
    let now_seconds = state.clock_origin.elapsed().as_secs_f64();
    apply_raw_message_at(state, device, message, now_seconds)
}

/// [`apply_raw_message`] with an explicit timestamp (seconds, monotonic) for
/// the clock tracker, so a fixed clock trace produces a deterministic BPM
/// (`setup-midi.js:40`).
///
/// Every other message type ignores `now_seconds`.
pub fn apply_raw_message_at(
    state: &mut MidiState,
    device: &str,
    message: &MidiRawMessage,
    now_seconds: f64,
) -> Vec<MidiEvent> {
    let learning = state.learning;
    let device_state = state
        .devices
        .entry(device.to_string())
        .or_insert_with(MidiDeviceState::new);

    // Clock: transport, not input data (`setup-midi.js:38-81`). It never
    // touches `channelData`; a device that does not listen for the clock
    // ignores it entirely.
    if message.type_ == TYPE_CLOCK {
        if !device_state.listen_for_clock {
            return Vec::new();
        }

        let bpm = state.clock.push(now_seconds);

        return vec![MidiEvent {
            kind: MidiEventKind::Clock,
            device: device.to_string(),
            channel: message.channel,
            type_: TYPE_CLOCK,
            value: 0.0,
            address: String::new(),
            bpm,
            learning,
        }];
    }

    if !device_state.listen_for_input {
        return Vec::new();
    }

    // setup-midi.js:87-91 — a latched CC is a note-on from here on.
    let latched_cc = message.type_ == TYPE_CONTROL_CHANGE && device_state.cc_latch;
    let written_type = if latched_cc {
        TYPE_NOTE_ON
    } else {
        message.type_
    };

    let value = if written_type == TYPE_NOTE_ON {
        if message.data > 0 {
            // setup-midi.js:93-102 — toggle 0/1; a missing entry counts as 0.
            if device_state
                .value(message.channel, written_type)
                .is_some_and(MidiChannelValue::is_on)
            {
                0.0
            } else {
                1.0
            }
        } else if device_state.note_on_latch {
            // setup-midi.js:104-109 — keep the button latched on release.
            return Vec::new();
        } else {
            0.0
        }
    } else if written_type == TYPE_CONTROL_CHANGE {
        // setup-midi.js:112-114 — only a real (unlatched) CC is normalised.
        f64::from(message.data) / CC_VALUE_MAX
    } else {
        // Every other status byte (aftertouch, pitch bend, note-off, …) stores
        // the raw data byte, exactly as the JS commit does.
        f64::from(message.data)
    };

    device_state.write(message.channel, written_type, value);

    let kind = if written_type == TYPE_NOTE_ON {
        if latched_cc {
            MidiEventKind::CcLatch
        } else {
            MidiEventKind::NoteOnLatch
        }
    } else {
        MidiEventKind::Value
    };

    vec![MidiEvent {
        kind,
        device: device.to_string(),
        channel: message.channel,
        type_: written_type,
        value,
        address: MidiEvent::channel_data_address(device, message.channel, written_type),
        bpm: None,
        learning,
    }]
}

#[cfg(test)]
mod tests {
    //! Behaviour port of `setup-midi.js:23-132` and
    //! `store/modules/midi.js:48-54`.
    use super::*;
    use serde_json::json;

    const KEY: &str = "abc-Controller-Maker";

    fn cc(type_: u8, channel: u8, data: u8) -> MidiRawMessage {
        MidiRawMessage::new(KEY, type_, channel, data)
    }

    #[test]
    fn device_key_matches_the_store_identity() {
        // setup-midi.js:29 / :118
        assert_eq!(device_key("abc", "Controller", "Maker"), KEY);
    }

    #[test]
    fn devices_start_with_the_store_defaults() {
        // store/modules/midi.js:32-41
        let state = MidiDeviceState::new();
        assert!(state.channel_data.is_empty());
        assert!(state.listen_for_input);
        assert!(!state.listen_for_clock);
        assert!(!state.cc_latch);
        assert!(!state.note_on_latch);
    }

    // setup-midi.js:112-122 — CC 7 value 127 → 127/127 = 1.0 at channelData[0][176].
    #[test]
    fn cc_values_are_normalised_into_channel_data() {
        let mut state = MidiState::new();

        let events = apply_raw_message(&mut state, KEY, &cc(TYPE_CONTROL_CHANGE, 0, 127));

        assert_eq!(events.len(), 1);
        let event = &events[0];
        assert_eq!(event.kind, MidiEventKind::Value);
        assert_eq!(event.type_, TYPE_CONTROL_CHANGE);
        assert_eq!(event.channel, 0);
        assert_eq!(event.value, 1.0);
        assert_eq!(event.address, format!("devices.{KEY}.channelData.0.176"));
        assert!(!event.learning);

        assert_eq!(state.channel_value(KEY, 0, TYPE_CONTROL_CHANGE), Some(1.0));
        assert_eq!(
            state.devices[KEY].channel_data[&0][&i32::from(TYPE_CONTROL_CHANGE)],
            MidiChannelValue::new(1.0)
        );
    }

    // setup-midi.js:112-114 — a plain CC is normalised, unlike a note-on.
    #[test]
    fn cc_normalisation_differs_from_the_note_on_latch() {
        let mut state = MidiState::new();

        apply_raw_message(&mut state, KEY, &cc(TYPE_CONTROL_CHANGE, 1, 64));
        apply_raw_message(&mut state, KEY, &cc(TYPE_NOTE_ON, 1, 64));

        // 64 / 127 == 0.5039…, *not* 64.
        assert_eq!(
            state.channel_value(KEY, 1, TYPE_CONTROL_CHANGE),
            Some(64.0 / 127.0)
        );
        // A note-on stores the latch toggle (1), not its velocity.
        assert_eq!(state.channel_value(KEY, 1, TYPE_NOTE_ON), Some(1.0));
    }

    // setup-midi.js:93-102 — presses toggle between 1 and 0.
    #[test]
    fn note_on_presses_toggle_the_latch_value() {
        let mut state = MidiState::new();

        let first = apply_raw_message(&mut state, KEY, &cc(TYPE_NOTE_ON, 2, 100));
        let second = apply_raw_message(&mut state, KEY, &cc(TYPE_NOTE_ON, 2, 100));
        let third = apply_raw_message(&mut state, KEY, &cc(TYPE_NOTE_ON, 2, 100));

        assert_eq!(first[0].value, 1.0);
        assert_eq!(second[0].value, 0.0);
        assert_eq!(third[0].value, 1.0);
        assert_eq!(first[0].kind, MidiEventKind::NoteOnLatch);
        assert_eq!(first[0].type_, TYPE_NOTE_ON);
        assert_eq!(state.channel_value(KEY, 2, TYPE_NOTE_ON), Some(1.0));
    }

    // setup-midi.js:104-109 — `noteOnLatch` ignores the release.
    #[test]
    fn note_on_latch_suppresses_the_note_off_write() {
        let mut state = MidiState::new();
        state.ensure_device(KEY).note_on_latch = true;

        apply_raw_message(&mut state, KEY, &cc(TYPE_NOTE_ON, 0, 100));
        let released = apply_raw_message(&mut state, KEY, &cc(TYPE_NOTE_ON, 0, 0));

        assert!(released.is_empty());
        assert_eq!(state.channel_value(KEY, 0, TYPE_NOTE_ON), Some(1.0));
    }

    // setup-midi.js:104-110 — without `noteOnLatch` the release writes 0.
    #[test]
    fn note_off_without_latch_writes_zero() {
        let mut state = MidiState::new();

        apply_raw_message(&mut state, KEY, &cc(TYPE_NOTE_ON, 0, 100));
        let released = apply_raw_message(&mut state, KEY, &cc(TYPE_NOTE_ON, 0, 0));

        assert_eq!(released.len(), 1);
        assert_eq!(released[0].value, 0.0);
        assert_eq!(state.channel_value(KEY, 0, TYPE_NOTE_ON), Some(0.0));
    }

    // setup-midi.js:89-102 — a latched CC behaves like a note-on, stored under 144.
    #[test]
    fn cc_latch_rewrites_cc_to_note_on_and_toggles() {
        let mut state = MidiState::new();
        state.ensure_device(KEY).cc_latch = true;

        let pressed = apply_raw_message(&mut state, KEY, &cc(TYPE_CONTROL_CHANGE, 3, 127));
        let released = apply_raw_message(&mut state, KEY, &cc(TYPE_CONTROL_CHANGE, 3, 127));

        assert_eq!(pressed.len(), 1);
        assert_eq!(pressed[0].kind, MidiEventKind::CcLatch);
        assert_eq!(pressed[0].type_, TYPE_NOTE_ON);
        assert_eq!(pressed[0].value, 1.0);
        assert_eq!(
            pressed[0].address,
            format!("devices.{KEY}.channelData.3.144")
        );

        assert_eq!(released[0].kind, MidiEventKind::CcLatch);
        assert_eq!(released[0].value, 0.0);

        // Nothing is written under the CC type.
        assert_eq!(state.channel_value(KEY, 3, TYPE_CONTROL_CHANGE), None);
        assert_eq!(state.channel_value(KEY, 3, TYPE_NOTE_ON), Some(0.0));
    }

    // setup-midi.js:93-109 — a latched CC with data 0 is a release, so
    // `noteOnLatch` suppresses it.
    #[test]
    fn cc_latch_release_respects_note_on_latch() {
        let mut state = MidiState::new();
        {
            let device = state.ensure_device(KEY);
            device.cc_latch = true;
            device.note_on_latch = true;
        }

        apply_raw_message(&mut state, KEY, &cc(TYPE_CONTROL_CHANGE, 4, 127));
        let release = apply_raw_message(&mut state, KEY, &cc(TYPE_CONTROL_CHANGE, 4, 0));

        assert!(release.is_empty());
        assert_eq!(state.channel_value(KEY, 4, TYPE_NOTE_ON), Some(1.0));
    }

    // setup-midi.js:93-114 — only note-on (144) and CC (176) are special-cased.
    #[test]
    fn note_off_and_other_types_store_the_raw_byte() {
        let mut state = MidiState::new();

        apply_raw_message(&mut state, KEY, &cc(TYPE_NOTE_ON, 7, 90));
        let note_off = apply_raw_message(&mut state, KEY, &cc(TYPE_NOTE_OFF, 7, 64));
        let pressure = apply_raw_message(&mut state, KEY, &cc(208, 7, 80));

        assert_eq!(note_off[0].kind, MidiEventKind::Value);
        assert_eq!(note_off[0].value, 64.0);
        assert_eq!(note_off[0].type_, TYPE_NOTE_OFF);
        assert_eq!(pressure[0].value, 80.0);
        // The note-off does not release the note-on latch.
        assert_eq!(state.channel_value(KEY, 7, TYPE_NOTE_ON), Some(1.0));
        assert_eq!(state.channel_value(KEY, 7, TYPE_NOTE_OFF), Some(64.0));
        assert_eq!(state.channel_value(KEY, 7, 208), Some(80.0));
    }

    #[test]
    fn listen_for_input_false_ignores_input_but_not_the_clock() {
        let mut state = MidiState::new();
        {
            let device = state.ensure_device(KEY);
            device.listen_for_input = false;
            device.listen_for_clock = true;
        }

        let input = apply_raw_message(&mut state, KEY, &cc(TYPE_CONTROL_CHANGE, 0, 127));
        assert!(input.is_empty());
        assert!(state.devices[KEY].channel_data.is_empty());

        let clock = apply_raw_message(&mut state, KEY, &cc(TYPE_CLOCK, 0, 0));
        assert_eq!(clock[0].kind, MidiEventKind::Clock);
        assert!(state.devices[KEY].channel_data.is_empty());
    }

    // setup-midi.js:29-33 — the JS drops messages for unknown devices; the IPC
    // path auto-registers them (see the module docs).
    #[test]
    fn unknown_devices_are_created_on_the_first_message() {
        let mut state = MidiState::new();
        assert!(state.device(KEY).is_none());

        apply_raw_message(&mut state, KEY, &cc(TYPE_CONTROL_CHANGE, 5, 127));

        let device = state.device(KEY).expect("device created");
        assert_eq!(device.channel_data.len(), 1);
        assert!(device.listen_for_input);
        assert!(!device.cc_latch);
        assert_eq!(state.channel_value(KEY, 5, TYPE_CONTROL_CHANGE), Some(1.0));
    }

    // setup-midi.js:38-81 — a clock never writes channelData; it drives BPM.
    #[test]
    fn clock_messages_never_create_channel_data() {
        let mut state = MidiState::new();
        state.ensure_device(KEY).listen_for_clock = true;

        let first = apply_raw_message_at(&mut state, KEY, &cc(TYPE_CLOCK, 0, 0), 0.0);
        let second = apply_raw_message_at(&mut state, KEY, &cc(TYPE_CLOCK, 0, 0), 0.020);

        assert_eq!(first[0].kind, MidiEventKind::Clock);
        assert_eq!(first[0].bpm, None);
        assert_eq!(first[0].address, "");

        // 20 ms apart at 24 PPQN → (1000 / 20 / 24) * 60 = 125 BPM.
        assert_eq!(second[0].kind, MidiEventKind::Clock);
        assert_eq!(second[0].bpm, Some(125));

        assert!(state.devices[KEY].channel_data.is_empty());
    }

    #[test]
    fn clock_is_ignored_when_the_device_does_not_listen_for_it() {
        let mut state = MidiState::new();

        let events = apply_raw_message_at(&mut state, KEY, &cc(TYPE_CLOCK, 0, 0), 0.0);

        assert!(events.is_empty());
        assert!(state.devices[KEY].channel_data.is_empty());
    }

    #[test]
    fn learning_is_reported_on_the_event_with_the_written_type() {
        // setup-midi.js:125-130 — the learning listener sees the rewritten type.
        let mut state = MidiState::new();
        state.learning = true;
        state.ensure_device(KEY).cc_latch = true;

        let events = apply_raw_message(&mut state, KEY, &cc(TYPE_CONTROL_CHANGE, 1, 100));

        assert!(events[0].learning);
        assert_eq!(events[0].type_, TYPE_NOTE_ON);
    }

    #[test]
    fn clock_events_also_report_learning() {
        let mut state = MidiState::new();
        state.learning = true;
        state.ensure_device(KEY).listen_for_clock = true;

        let events = apply_raw_message_at(&mut state, KEY, &cc(TYPE_CLOCK, 0, 0), 0.0);

        assert!(events[0].learning);
    }

    #[test]
    fn channels_are_independent() {
        let mut state = MidiState::new();

        apply_raw_message(&mut state, KEY, &cc(TYPE_CONTROL_CHANGE, 0, 127));
        apply_raw_message(&mut state, KEY, &cc(TYPE_CONTROL_CHANGE, 1, 64));

        assert_eq!(state.channel_value(KEY, 0, TYPE_CONTROL_CHANGE), Some(1.0));
        assert_eq!(
            state.channel_value(KEY, 1, TYPE_CONTROL_CHANGE),
            Some(64.0 / 127.0)
        );
    }

    // docs/rust-hybrid.md, "The IPC path" — the wire payload.
    #[test]
    fn raw_message_serialises_with_a_type_field() {
        let message = MidiRawMessage::new(KEY, TYPE_CLOCK, 0, 0);

        assert_eq!(
            serde_json::to_value(&message).expect("serialise"),
            json!({ "device": KEY, "type": 248, "channel": 0, "data": 0 })
        );

        let parsed: MidiRawMessage = serde_json::from_value(json!({
            "device": KEY,
            "type": 176,
            "channel": 2,
            "data": 64
        }))
        .expect("deserialise");
        assert_eq!(parsed, cc(TYPE_CONTROL_CHANGE, 2, 64));
    }

    // store/modules/midi.js:48-54 — the mirror store sees bare numbers.
    #[test]
    fn channel_values_serialise_as_numbers() {
        let mut state = MidiState::new();
        apply_raw_message(&mut state, KEY, &cc(TYPE_CONTROL_CHANGE, 0, 127));

        let device = serde_json::to_value(&state.devices[KEY]).expect("serialise");
        assert_eq!(device["channelData"]["0"]["176"], json!(1.0));
        assert_eq!(device["listenForInput"], json!(true));
        assert_eq!(device["listenForClock"], json!(false));

        let restored: MidiDeviceState = serde_json::from_value(device).expect("deserialise");
        assert_eq!(
            restored.channel_data[&0][&i32::from(TYPE_CONTROL_CHANGE)].value,
            1.0
        );
    }

    #[test]
    fn events_serialise_for_the_delta_stream() {
        let mut state = MidiState::new();
        let events = apply_raw_message(&mut state, KEY, &cc(TYPE_CONTROL_CHANGE, 0, 127));

        assert_eq!(
            serde_json::to_value(&events[0]).expect("serialise"),
            json!({
                "kind": "value",
                "device": KEY,
                "channel": 0,
                "type": 176,
                "value": 1.0,
                "address": format!("devices.{KEY}.channelData.0.176"),
                "learning": false
            })
        );
    }
}
