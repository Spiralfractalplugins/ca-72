//! Standalone Audio & MIDI settings, drawn inside the existing editor. All
//! device work is submitted to the shared host's control thread.

use std::fmt::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::{Duration, Instant};

use ca72_panel::{Pixmap, art, fonts::Fonts};
use keyboard_types::{Key, KeyState, KeyboardEvent, Modifiers};
use plugin_kit_standalone::{
    AppHandle, ApplyReceipt, Config, DeviceId, Phase, Snapshot, negotiate,
};

static REQUESTED: AtomicBool = AtomicBool::new(false);

pub fn available() -> bool {
    #[cfg(test)]
    if AS_STANDALONE.get() {
        return true;
    }
    AppHandle::current().is_some()
}

#[cfg(test)]
thread_local! {
    /// A test's editor on this thread made as the standalone's, its host there (the host's
    /// handle is the process's, and would put STANDALONE in every test's settings).
    pub(crate) static AS_STANDALONE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}
pub fn request_open() {
    if available() {
        REQUESTED.store(true, Ordering::Release);
    }
}
pub fn take_requested() -> Option<Dialog> {
    let requested = REQUESTED.swap(false, Ordering::AcqRel);
    let app = AppHandle::current()?;
    static ALERTS: OnceLock<Mutex<Alerts>> = OnceLock::new();
    let mut alerts = ALERTS
        .get_or_init(|| Mutex::new(Alerts::default()))
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if !alerts
        .host
        .upgrade()
        .is_some_and(|host| Arc::ptr_eq(&host, &app))
    {
        *alerts = Alerts {
            host: Arc::downgrade(&app),
            ..Alerts::default()
        };
    }
    let mut notify = false;
    if Instant::now() >= alerts.next_poll {
        alerts.next_poll = Instant::now() + Duration::from_millis(200);
        let snapshot = app.snapshot();
        let signature = snapshot.error.or(snapshot.warning);
        notify = signature.is_some() && signature != alerts.last_issue;
        alerts.last_issue = signature;
    }
    (requested || notify).then(|| Dialog::new(app))
}

struct Alerts {
    host: Weak<AppHandle>,
    next_poll: Instant,
    last_issue: Option<String>,
}
impl Default for Alerts {
    fn default() -> Self {
        Self {
            host: Weak::new(),
            next_poll: Instant::now(),
            last_issue: None,
        }
    }
}

const X: f64 = 360.0;
const Y: f64 = 155.0;
const WIDTH: f64 = 2380.0;
const HEIGHT: f64 = 1480.0;
const ROW_Y: f64 = 390.0;
const ROW_H: f64 = 90.0;
const VALUE_X: f64 = X + 560.0;
const NEXT_X: f64 = X + WIDTH - 100.0;
const BUTTON_Y: f64 = Y + HEIGHT - 125.0;
/// The rows: the devices' eight, then TEMPO (decisions.md R-ARP).
const ROWS: usize = 9;
/// The row of TEMPO: the arpeggiator's where no host gives one, applied at once (not a device
/// setting: nothing to restart), kept for the computer by the editor.
const TEMPO_ROW: usize = 8;
/// TEMPO's choices in its list: every 5 from 40 to 240 (the arrow keys step it by 1).
const TEMPO_LIST: (u16, u16, u16) = (40, 240, 5);

#[derive(Clone, Debug, PartialEq, Eq)]
enum Choice {
    Output(Option<DeviceId>),
    OutputChannels([u16; 2]),
    Input(Option<DeviceId>),
    InputChannels([u16; 2]),
    Rate(u32),
    Buffer(u32),
    MidiAuto,
    Midi(Option<String>),
    Channel(Option<u8>),
    Tempo(u16),
}
#[derive(Debug)]
struct SelectionMenu {
    row: usize,
    highlighted: usize,
    top: usize,
    options: Vec<(String, Choice)>,
}
impl SelectionMenu {
    fn visible(&self) -> usize {
        self.options.len().saturating_sub(self.top).min(10)
    }
    fn rectangle(&self) -> [f64; 4] {
        let height = self.visible() as f64 * 64.0 + 72.0;
        let top = (ROW_Y + self.row as f64 * ROW_H + ROW_H)
            .min(BUTTON_Y - height - 15.0)
            .max(Y + 190.0);
        [VALUE_X - 25.0, top, X + WIDTH - VALUE_X - 10.0, height]
    }
    fn move_by(&mut self, offset: i32) {
        self.highlighted =
            (self.highlighted as i32 + offset).clamp(0, self.options.len() as i32 - 1) as usize;
        if self.highlighted < self.top {
            self.top = self.highlighted;
        }
        if self.highlighted >= self.top + 10 {
            self.top = self.highlighted - 9;
        }
    }
}
fn device_label(id: &DeviceId) -> String {
    format!(
        "{} ({} out / {} in)",
        id.name, id.output_channels, id.input_channels
    )
}

fn same_settings(left: &Config, right: &Config) -> bool {
    // The selector is a set of checked inputs; toggling one off/on can change
    // vector order without changing the requested routing.
    let mut left = left.clone();
    let mut right = right.clone();
    left.midi_inputs.sort();
    right.midi_inputs.sort();
    left == right
}

/// UI model, including an unapplied draft. Cancel simply discards this model.
#[derive(Debug)]
pub struct Dialog {
    app: Arc<AppHandle>,
    snapshot: Snapshot,
    draft: Config,
    focus: usize,
    menu: Option<SelectionMenu>,
    local_error: Option<String>,
    // Edits since the last draft sync or submitted application. Keep these even
    // when an older application completes with a different, normalised config.
    edited: bool,
    // Keep the queued candidate until acknowledged, including across later edits.
    submitted: Option<Config>,
    completion: Option<ApplyReceipt>,
    next_poll: Instant,
    stamp: String,
    changed: bool,
    /// TEMPO as shown, and whether it changed since the editor last took it.
    tempo: u16,
    tempo_changed: bool,
    /// The audio threads' priority as last read, and where it is read (decisions.md R-RTKIT:
    /// the audio side's atomics; the tests give their own).
    realtime: (ca72_rt::Standing, ca72_rt::Standing),
    read_realtime: fn() -> (ca72_rt::Standing, ca72_rt::Standing),
}

impl Dialog {
    fn new(app: Arc<AppHandle>) -> Self {
        let snapshot = app.snapshot();
        Self {
            draft: snapshot.config.clone(),
            snapshot,
            app,
            focus: 0,
            menu: None,
            local_error: None,
            edited: false,
            submitted: None,
            completion: None,
            next_poll: Instant::now(),
            stamp: String::new(),
            changed: true,
            tempo: crate::params::DEFAULT_TEMPO as u16,
            tempo_changed: false,
            realtime: crate::audio_thread::standing(),
            read_realtime: crate::audio_thread::standing,
        }
    }

    /// For a look (the editor's review frames): playing at 48 kHz in 256-frame blocks, its audio
    /// threads standing as `realtime`, polled no more.
    #[cfg(all(test, target_os = "linux"))]
    pub(crate) fn review(&mut self, realtime: (ca72_rt::Standing, ca72_rt::Standing)) {
        self.frame();
        self.next_poll = Instant::now() + Duration::from_secs(3600);
        self.snapshot.phase = Phase::Running;
        self.snapshot.actual_sample_rate = 48_000;
        self.snapshot.actual_buffer_frames = 256;
        self.snapshot.error = None;
        self.snapshot.warning = None;
        // (A device to play on, as a computer has.)
        let id = DeviceId {
            name: "Speakers".into(),
            input_channels: 0,
            output_channels: 2,
        };
        let capability = plugin_kit_standalone::StreamCapability {
            channels: 2,
            min_sample_rate: 44_100,
            max_sample_rate: 48_000,
            buffer_range: Some([64, 1024]),
        };
        self.snapshot.devices.backend = "ALSA".into();
        self.snapshot.devices.default_output = Some(id.clone());
        self.snapshot.devices.default_sample_rate = 48_000;
        self.snapshot.devices.audio = vec![plugin_kit_standalone::DeviceInfo {
            id: id.clone(),
            input_configs: Vec::new(),
            output_configs: vec![capability],
            sample_rates: vec![44_100, 48_000],
            default_sample_rate: 48_000,
        }];
        self.snapshot.config.output = Some(id.clone());
        self.draft.output = Some(id);
        self.realtime = realtime;
        self.changed = true;
    }

    /// TEMPO as this computer has it (the editor's, as it opens the dialog).
    pub fn set_tempo(&mut self, bpm: u16) {
        self.tempo = crate::settings::tempo_within(bpm);
        self.changed = true;
    }

    /// TEMPO, if it was changed here since the last time it was asked.
    pub fn take_tempo(&mut self) -> Option<u16> {
        std::mem::take(&mut self.tempo_changed).then_some(self.tempo)
    }

    pub fn frame(&mut self) -> bool {
        if Instant::now() >= self.next_poll {
            self.next_poll = Instant::now() + Duration::from_millis(200);
            if let Some(receipt) = &self.completion {
                match receipt.try_result() {
                    Ok(Some(config)) => self.complete_application(Ok(config)),
                    Err(error) => self.complete_application(Err(error)),
                    Ok(None) => {}
                }
            }
            let realtime = (self.read_realtime)();
            if realtime != self.realtime {
                self.realtime = realtime;
                self.changed = true;
            }
            let snapshot = self.app.snapshot();
            let stamp = format!("{snapshot:?}");
            if stamp != self.stamp {
                self.stamp = stamp;
                self.update_snapshot(snapshot);
            }
        }
        std::mem::take(&mut self.changed)
    }

    fn update_snapshot(&mut self, snapshot: Snapshot) {
        if !self.edited && self.submitted.is_none() {
            self.draft.clone_from(&snapshot.config);
            self.edited = false;
        }
        let menu = (self.snapshot.devices != snapshot.devices)
            .then(|| self.menu.take())
            .flatten();
        self.snapshot = snapshot;
        if let Some(previous) = menu {
            // Inventory arrives asynchronously and can change while a selector
            // is open. Rebuild its choices without changing the user's draft.
            self.open_menu(previous.row);
            if let Some(menu) = &mut self.menu
                && let Some((_, choice)) = previous.options.get(previous.highlighted)
                && let Some(index) = menu.options.iter().position(|(_, next)| next == choice)
            {
                menu.highlighted = index;
                menu.top = previous.top.min(menu.options.len().saturating_sub(1));
                menu.move_by(0);
            }
        }
        self.changed = true;
    }

    fn output(&self) -> Option<&DeviceId> {
        self.draft
            .output
            .as_ref()
            .or(self.snapshot.devices.default_output.as_ref())
    }
    fn channel_choices(&self, input: bool) -> Vec<[u16; 2]> {
        let channels = if input {
            self.draft.input.as_ref().map_or(0, |id| id.input_channels)
        } else {
            self.output().map_or(0, |id| id.output_channels)
        };
        // Every mono channel and every adjacent stereo pair is selectable.
        let mut choices: Vec<_> = (0..channels).map(|channel| [channel, channel]).collect();
        choices.extend((0..channels.saturating_sub(1)).map(|channel| [channel, channel + 1]));
        choices
    }
    fn rates(&self) -> Vec<u32> {
        let mut rates = vec![0];
        if let Some(id) = self.output()
            && let Some(device) = self
                .snapshot
                .devices
                .audio
                .iter()
                .find(|device| device.id == *id)
        {
            rates.extend(device.sample_rates.iter().copied());
        }
        if !rates.contains(&self.draft.sample_rate) {
            rates.push(self.draft.sample_rate);
        }
        rates.sort_unstable();
        rates.dedup();
        rates
    }
    fn buffers(&self) -> Vec<u32> {
        let mut buffers = vec![32, 64, 128, 256, 512, 1024, 2048, 4096, 8192];
        if let Some(id) = self.output()
            && let Some(device) = self
                .snapshot
                .devices
                .audio
                .iter()
                .find(|device| device.id == *id)
        {
            for capability in &device.output_configs {
                if let Some([minimum, maximum]) = capability.buffer_range {
                    buffers.extend([minimum.max(16), maximum.min(8192)]);
                }
            }
        }
        buffers.push(self.draft.buffer_frames);
        buffers.retain(|frames| (16..=8192).contains(frames));
        buffers.sort_unstable();
        buffers.dedup();
        buffers
    }
    fn choices(&self) -> Vec<Option<DeviceId>> {
        let mut choices = vec![None];
        choices.extend(
            self.snapshot
                .devices
                .audio
                .iter()
                .filter(|device| device.id.output_channels > 0)
                .map(|device| Some(device.id.clone())),
        );
        if !choices.contains(&self.draft.output) {
            choices.push(self.draft.output.clone());
        }
        choices
    }
    fn input_choices(&self) -> Vec<Option<DeviceId>> {
        let mut choices = vec![None];
        choices.extend(
            self.snapshot
                .devices
                .audio
                .iter()
                .filter(|device| device.id.input_channels > 0)
                .map(|device| Some(device.id.clone())),
        );
        if !choices.contains(&self.draft.input) {
            choices.push(self.draft.input.clone());
        }
        choices
    }
    fn options(&self, row: usize) -> Vec<(String, Choice)> {
        match row {
            0 => self
                .choices()
                .into_iter()
                .map(|id| {
                    (
                        id.as_ref()
                            .map_or_else(|| "System default".into(), device_label),
                        Choice::Output(id),
                    )
                })
                .collect(),
            1 => self
                .channel_choices(false)
                .into_iter()
                .map(|value| (channels(value), Choice::OutputChannels(value)))
                .collect(),
            2 => self
                .input_choices()
                .into_iter()
                .map(|id| {
                    (
                        id.as_ref().map_or_else(|| "DISABLED".into(), device_label),
                        Choice::Input(id),
                    )
                })
                .collect(),
            3 => self
                .channel_choices(true)
                .into_iter()
                .map(|value| (channels(value), Choice::InputChannels(value)))
                .collect(),
            4 => self
                .rates()
                .into_iter()
                .map(|rate| {
                    (
                        if rate == 0 {
                            "Device default".into()
                        } else {
                            format!("{rate} Hz")
                        },
                        Choice::Rate(rate),
                    )
                })
                .collect(),
            5 => self
                .buffers()
                .into_iter()
                .map(|frames| (format!("{frames} frames"), Choice::Buffer(frames)))
                .collect(),
            6 => [
                ("Automatic (first available input)".into(), Choice::MidiAuto),
                ("Disable hardware MIDI inputs".into(), Choice::Midi(None)),
            ]
            .into_iter()
            .chain(
                self.snapshot
                    .devices
                    .midi_inputs
                    .iter()
                    .map(|name| (name.clone(), Choice::Midi(Some(name.clone())))),
            )
            .collect(),
            7 => std::iter::once(("ALL".into(), Choice::Channel(None)))
                .chain(
                    (1..=16).map(|channel| (channel.to_string(), Choice::Channel(Some(channel)))),
                )
                .collect(),
            TEMPO_ROW => {
                let (lo, hi, step) = TEMPO_LIST;
                let mut tempos: Vec<u16> = (lo..=hi).step_by(usize::from(step)).collect();
                if !tempos.contains(&self.tempo) {
                    tempos.push(self.tempo);
                    tempos.sort_unstable();
                }
                tempos
                    .into_iter()
                    .map(|bpm| (format!("{bpm} BPM"), Choice::Tempo(bpm)))
                    .collect()
            }
            _ => Vec::new(),
        }
    }
    fn selected(&self, choice: &Choice) -> bool {
        match choice {
            Choice::Output(id) => self.draft.output == *id,
            Choice::OutputChannels(channels) => self.draft.output_channels == *channels,
            Choice::Input(id) => self.draft.input == *id,
            Choice::InputChannels(channels) => self.draft.input_channels == *channels,
            Choice::Rate(rate) => self.draft.sample_rate == *rate,
            Choice::Buffer(frames) => self.draft.buffer_frames == *frames,
            Choice::MidiAuto => self.draft.auto_midi,
            Choice::Midi(None) => !self.draft.auto_midi && self.draft.midi_inputs.is_empty(),
            Choice::Midi(Some(name)) => self.draft.midi_inputs.contains(name),
            Choice::Channel(channel) => self.draft.midi_channel == *channel,
            Choice::Tempo(bpm) => self.tempo == *bpm,
        }
    }
    fn choose(&mut self, choice: Choice) {
        let previous = self.draft.clone();
        match choice {
            Choice::Output(id) => {
                if self.draft.output != id {
                    self.draft.output = id;
                    self.draft.input = None;
                    self.draft.output_channels =
                        if self.output().is_some_and(|id| id.output_channels > 1) {
                            [0, 1]
                        } else {
                            [0, 0]
                        };
                }
            }
            Choice::OutputChannels(channels) => self.draft.output_channels = channels,
            Choice::Input(id) => {
                if self.draft.input != id {
                    self.draft.input = id;
                    self.draft.input_channels = [0, 0];
                }
            }
            Choice::InputChannels(channels) => self.draft.input_channels = channels,
            Choice::Rate(rate) => self.draft.sample_rate = rate,
            Choice::Buffer(frames) => self.draft.buffer_frames = frames,
            Choice::MidiAuto => {
                if !self.draft.auto_midi {
                    self.draft.auto_midi = true;
                    self.draft.midi_inputs = if self.snapshot.config.auto_midi {
                        self.snapshot.config.midi_inputs.clone()
                    } else {
                        Vec::new()
                    };
                }
            }
            Choice::Midi(None) => {
                self.draft.auto_midi = false;
                self.draft.midi_inputs.clear();
            }
            Choice::Midi(Some(name)) => {
                self.draft.auto_midi = false;
                if self.draft.midi_inputs.contains(&name) {
                    self.draft.midi_inputs.retain(|selected| selected != &name);
                } else {
                    self.draft.midi_inputs.push(name);
                }
            }
            Choice::Channel(channel) => self.draft.midi_channel = channel,
            Choice::Tempo(bpm) => {
                self.tempo = crate::settings::tempo_within(bpm);
                self.tempo_changed = true;
            }
        }
        self.record_edit(&previous);
        self.local_error = None;
        self.changed = true;
    }
    fn open_menu(&mut self, row: usize) {
        let options = self.options(row);
        if options.is_empty() {
            return;
        }
        let highlighted = options
            .iter()
            .position(|(_, choice)| self.selected(choice))
            .unwrap_or(0);
        self.menu = Some(SelectionMenu {
            row,
            top: highlighted.saturating_sub(9),
            highlighted,
            options,
        });
        self.focus = row;
        self.changed = true;
    }
    fn cycle(&mut self, row: usize, direction: i32) {
        self.local_error = None;
        let previous = self.draft.clone();
        match row {
            0 => {
                let output = stepped(&self.choices(), &self.draft.output, direction)
                    .cloned()
                    .flatten();
                self.choose(Choice::Output(output));
            }
            1 => {
                if let Some(channels) = stepped(
                    &self.channel_choices(false),
                    &self.draft.output_channels,
                    direction,
                ) {
                    self.draft.output_channels = *channels;
                }
            }
            2 => {
                let choices = self.input_choices();
                let input = stepped(&choices, &self.draft.input, direction)
                    .cloned()
                    .flatten();
                self.choose(Choice::Input(input));
            }
            3 => {
                if let Some(channels) = stepped(
                    &self.channel_choices(true),
                    &self.draft.input_channels,
                    direction,
                ) {
                    self.draft.input_channels = *channels;
                }
            }
            4 => {
                if let Some(rate) = stepped(&self.rates(), &self.draft.sample_rate, direction) {
                    self.draft.sample_rate = *rate;
                }
            }
            5 => {
                if let Some(buffer) = stepped(&self.buffers(), &self.draft.buffer_frames, direction)
                {
                    self.draft.buffer_frames = *buffer;
                }
            }
            6 => self.open_menu(6),
            7 => {
                let channels: Vec<_> = std::iter::once(None).chain((1..=16).map(Some)).collect();
                self.draft.midi_channel = stepped(&channels, &self.draft.midi_channel, direction)
                    .copied()
                    .flatten();
            }
            TEMPO_ROW => {
                let bpm = (i32::from(self.tempo) + direction).clamp(0, i32::from(u16::MAX));
                self.choose(Choice::Tempo(bpm as u16));
            }
            _ => {}
        }
        self.record_edit(&previous);
        self.focus = row;
        self.changed = true;
    }
    fn validation(&self) -> Result<(), String> {
        negotiate(&self.draft, &self.snapshot.devices).map(|_| ())
    }
    fn record_edit(&mut self, previous: &Config) {
        if self.draft != *previous {
            self.edited =
                self.submitted.is_some() || !same_settings(&self.draft, &self.snapshot.config);
        }
    }
    fn has_changes(&self) -> bool {
        self.submitted.is_some() || !same_settings(&self.draft, &self.snapshot.config)
    }
    fn complete_application(&mut self, result: Result<Config, String>) {
        let request = self.submitted.take();
        self.completion = None;
        match result {
            Ok(config) => {
                if !self.edited
                    || request
                        .as_ref()
                        .is_some_and(|request| same_settings(request, &self.draft))
                {
                    self.draft.clone_from(&config);
                }
                self.snapshot.config = config;
                self.local_error = None;
            }
            Err(error) => self.local_error = Some(error),
        }
        self.edited = !same_settings(&self.draft, &self.snapshot.config);
        // A newer host snapshot may already have been observed while this
        // receipt was pending. Always reconcile it after consuming the result.
        self.stamp.clear();
        self.changed = true;
    }
    fn finish_or_apply(&mut self) -> bool {
        if !self.has_changes() {
            return true;
        }
        self.apply();
        false
    }
    fn apply(&mut self) {
        if self.submitted.is_some() {
            return;
        }
        match self
            .validation()
            .and_then(|()| self.app.apply_tracked(self.draft.clone()))
        {
            Ok(receipt) => {
                self.local_error = None;
                self.submitted = Some(self.draft.clone());
                self.completion = Some(receipt);
                self.edited = false;
            }
            Err(error) => self.local_error = Some(error),
        }
        self.changed = true;
    }

    /// Pointer coordinates are the panel's drawing units. All presses in this
    /// overlay are consumed by the editor, including outside the dialog's box.
    pub fn press(&mut self, x: f64, y: f64) -> bool {
        if let Some(menu) = &self.menu {
            let [left, top, width, height] = menu.rectangle();
            if (left..left + width).contains(&x) && (top..top + height).contains(&y) {
                let index = ((y - top) / 64.0) as usize;
                if index < menu.visible() {
                    let chosen = menu.options[menu.top + index].1.clone();
                    let multiple = matches!(chosen, Choice::Midi(_));
                    self.choose(chosen);
                    if !multiple {
                        self.menu = None;
                    }
                } else if let Some(menu) = &mut self.menu {
                    let direction = if x < left + width / 2.0 { -10 } else { 10 };
                    menu.move_by(direction);
                    self.changed = true;
                }
            } else {
                self.menu = None;
                self.changed = true;
            }
            return false;
        }
        if (BUTTON_Y..BUTTON_Y + 75.0).contains(&y) {
            if (X + 45.0..X + 330.0).contains(&x) {
                self.local_error = self.app.refresh().err();
                self.changed = true;
            }
            if (X + WIDTH - 710.0..X + WIDTH - 405.0).contains(&x) {
                return true;
            }
            if (X + WIDTH - 365.0..X + WIDTH - 50.0).contains(&x) {
                return self.finish_or_apply();
            }
            return false;
        }
        if (ROW_Y..ROW_Y + ROW_H * ROWS as f64).contains(&y) && (X..X + WIDTH).contains(&x) {
            let row = ((y - ROW_Y) / ROW_H) as usize;
            self.focus = row;
            if x >= VALUE_X - 30.0 {
                self.open_menu(row);
            } else {
                self.changed = true;
            }
        }
        false
    }

    pub fn key(&mut self, event: &KeyboardEvent) -> bool {
        if event.state != KeyState::Down {
            return false;
        }
        if let Some(menu) = &mut self.menu {
            match event.key {
                Key::Escape => self.menu = None,
                Key::ArrowUp => menu.move_by(-1),
                Key::ArrowDown => menu.move_by(1),
                Key::PageUp => menu.move_by(-10),
                Key::PageDown => menu.move_by(10),
                Key::Home => {
                    menu.highlighted = 0;
                    menu.top = 0;
                }
                Key::End => {
                    menu.highlighted = menu.options.len() - 1;
                    menu.top = menu.highlighted.saturating_sub(9);
                }
                Key::Enter => {
                    let chosen = menu.options[menu.highlighted].1.clone();
                    let multiple = matches!(chosen, Choice::Midi(_));
                    self.choose(chosen);
                    if !multiple {
                        self.menu = None;
                    }
                }
                Key::Character(ref text) if text == " " => {
                    let chosen = menu.options[menu.highlighted].1.clone();
                    let multiple = matches!(chosen, Choice::Midi(_));
                    self.choose(chosen);
                    if !multiple {
                        self.menu = None;
                    }
                }
                Key::Tab => self.menu = None,
                _ => {}
            }
            self.changed = true;
            return false;
        }
        match event.key {
            Key::Escape => return true,
            Key::Tab => {
                self.focus = (self.focus
                    + if event.modifiers.contains(Modifiers::SHIFT) {
                        ROWS + 2
                    } else {
                        1
                    })
                    % (ROWS + 3);
                self.changed = true;
            }
            Key::ArrowDown => {
                self.focus = (self.focus + 1) % (ROWS + 3);
                self.changed = true;
            }
            Key::ArrowUp => {
                self.focus = (self.focus + ROWS + 2) % (ROWS + 3);
                self.changed = true;
            }
            Key::ArrowLeft => self.cycle(self.focus, -1),
            Key::ArrowRight => self.cycle(self.focus, 1),
            Key::Enter => match self.focus {
                f if f == ROWS => {
                    self.local_error = self.app.refresh().err();
                    self.changed = true;
                }
                f if f == ROWS + 1 => return true,
                f if f == ROWS + 2 => return self.finish_or_apply(),
                _ => self.open_menu(self.focus),
            },
            Key::Character(ref text) if text == " " => self.open_menu(self.focus),
            _ => {}
        }
        false
    }

    pub fn draw(&self, onto: &mut Pixmap, scale: f64) {
        let fonts = Fonts::new();
        let svg = self.svg();
        let options = resvg::usvg::Options {
            fontdb: fonts.database(),
            ..Default::default()
        };
        let Ok(tree) = resvg::usvg::Tree::from_str(&svg, &options) else {
            return;
        };
        resvg::render(
            &tree,
            resvg::tiny_skia::Transform::from_scale(scale as f32, scale as f32),
            &mut onto.as_mut(),
        );
    }

    /// Kept public for static visual proof without opening any device/window.
    pub fn svg(&self) -> String {
        let mut svg = format!(
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="{}" height="{}" viewBox="0 0 {} {}"><rect width="100%" height="100%" fill="#08090b" opacity=".72"/><rect x="{X}" y="{Y}" width="{WIDTH}" height="{HEIGHT}" rx="20" fill="#1f2228" stroke="#4a4640" stroke-width="4"/><g font-family="URW Gothic" fill="#ebe6d8">"##,
            art::W,
            art::H,
            art::W,
            art::H
        );
        text(
            &mut svg,
            X + 55.0,
            Y + 85.0,
            52.0,
            "AUDIO & MIDI",
            "#f0a030",
        );
        text(
            &mut svg,
            X + 55.0,
            Y + 153.0,
            31.0,
            &format!(
                "{} · Click a value to choose; Tab selects a setting; Enter opens its list",
                self.snapshot.devices.backend
            ),
            "#a6a5a1",
        );
        let output = self.draft.output.as_ref().map_or_else(
            || {
                format!(
                    "System default ({})",
                    self.snapshot
                        .devices
                        .default_output
                        .as_ref()
                        .map_or("unavailable", |id| id.name.as_str())
                )
            },
            |id| id.name.clone(),
        );
        let input = self
            .draft
            .input
            .as_ref()
            .map_or_else(|| "DISABLED".to_owned(), |id| id.name.clone());
        let rate = if self.draft.sample_rate == 0 {
            "Device default".to_owned()
        } else {
            format!("{} Hz", self.draft.sample_rate)
        };
        let midi = if self.draft.auto_midi {
            if self.draft.midi_inputs.is_empty() {
                format!(
                    "Automatic (first available; {} detected)",
                    self.snapshot.devices.midi_inputs.len()
                )
            } else {
                format!("Automatic · {}", self.draft.midi_inputs.join(", "))
            }
        } else if self.draft.midi_inputs.is_empty() {
            format!(
                "{} detected; none selected (click to choose)",
                self.snapshot.devices.midi_inputs.len()
            )
        } else {
            format!(
                "{} selected · {}",
                self.draft.midi_inputs.len(),
                self.draft.midi_inputs.join(", ")
            )
        };
        let rows = [
            ("OUTPUT DEVICE", output),
            ("OUTPUT CHANNELS", channels(self.draft.output_channels)),
            ("EXTERNAL INPUT", input),
            (
                "INPUT CHANNELS",
                if self.draft.input.is_some() {
                    channels(self.draft.input_channels)
                } else {
                    "Input is disabled".to_owned()
                },
            ),
            ("SAMPLE RATE", rate),
            (
                "BUFFER REQUEST",
                format!("{} frames", self.draft.buffer_frames),
            ),
            ("MIDI INPUTS", midi),
            (
                "MIDI CHANNEL",
                self.draft
                    .midi_channel
                    .map_or_else(|| "ALL".to_owned(), |channel| channel.to_string()),
            ),
            (
                "TEMPO",
                format!("{} BPM (the arpeggiator's; applied at once)", self.tempo),
            ),
        ];
        for (row, (label, value)) in rows.iter().enumerate() {
            let y = ROW_Y + row as f64 * ROW_H;
            let fill = if self.focus == row {
                "#3a3022"
            } else {
                "#15171b"
            };
            let _ = write!(
                svg,
                r##"<rect x="{}" y="{y}" width="{}" height="{}" rx="8" fill="{fill}"/>"##,
                X + 35.0,
                WIDTH - 70.0,
                ROW_H - 12.0
            );
            text(&mut svg, X + 65.0, y + 51.0, 30.0, label, "#a6a5a1");
            text(
                &mut svg,
                VALUE_X,
                y + 51.0,
                33.0,
                &ellipsize(value, 75),
                "#ebe6d8",
            );
            let _ = write!(
                svg,
                r##"<path d="M {} {} l 24 0 l -12 16 z" fill="#f0a030"/>"##,
                NEXT_X - 12.0,
                y + 31.0
            );
        }
        let status_y = ROW_Y + ROW_H * ROWS as f64 + 50.0;
        let phase = match self.snapshot.phase {
            Phase::Stopped => "STOPPED",
            Phase::Starting => "STARTING",
            Phase::Running => "RUNNING",
            Phase::Recovering => "RECOVERING",
            Phase::Error => "ERROR",
        };
        // (The audio threads' priority after, while it plays: decisions.md R-RTKIT.)
        let (audio, workers) = self.realtime;
        let priority = match crate::audio_thread::summary(audio, workers) {
            crate::audio_thread::Standing::Unknown => "",
            crate::audio_thread::Standing::Asking => " · asking RTKit for real-time priority",
            crate::audio_thread::Standing::RealTime { rtkit: false } => " · real-time",
            crate::audio_thread::Standing::RealTime { rtkit: true } => " · real-time (RTKit)",
            crate::audio_thread::Standing::Ordinary => " · ordinary priority",
        };
        let actual = self.snapshot.buffer_duration_ms().map_or_else(
            || "Audio has not started".to_owned(),
            |ms| {
                format!(
                    "{} Hz · actual callback {} frames · buffer duration {ms:.2} ms{priority}",
                    self.snapshot.actual_sample_rate, self.snapshot.actual_buffer_frames
                )
            },
        );
        text(
            &mut svg,
            X + 55.0,
            status_y,
            34.0,
            &format!("{phase}   {actual}"),
            "#f0a030",
        );
        let issue = self
            .local_error
            .clone()
            .or_else(|| self.snapshot.error.clone())
            .or_else(|| self.validation().err())
            .or_else(|| self.snapshot.warning.clone());
        let busy = issue.is_some();
        if let Some(issue) = issue {
            text(
                &mut svg,
                X + 55.0,
                status_y + 67.0,
                30.0,
                &ellipsize(&issue, 124),
                "#e5b94d",
            );
        }
        // The audio not real-time (decisions.md R-RTKIT): which threads and why, then the fix,
        // under any other issue, the note below moved down for them.
        let mut note = status_y + 135.0;
        if let Some(w) = crate::audio_thread::warning(audio, workers, cfg!(target_os = "linux")) {
            let (y, size) = if busy {
                note += 34.0;
                (status_y + 101.0, 27.0)
            } else {
                (status_y + 67.0, 30.0)
            };
            text(
                &mut svg,
                X + 55.0,
                y,
                size,
                &ellipsize(&w.detail, 124),
                "#e5b94d",
            );
            text(
                &mut svg,
                X + 55.0,
                y + 34.0,
                27.0,
                &ellipsize(w.remedy, 124),
                "#e5b94d",
            );
        }
        text(
            &mut svg,
            X + 55.0,
            note,
            27.0,
            "External input is adapted to the output clock. Changing settings briefly interrupts sound.",
            "#a6a5a1",
        );
        for (button, (x, width, label, enabled)) in [
            (X + 45.0, 285.0, "REFRESH", true),
            (X + WIDTH - 710.0, 305.0, "CANCEL", true),
            (
                X + WIDTH - 365.0,
                315.0,
                if self.has_changes() { "APPLY" } else { "DONE" },
                !self.has_changes() || (self.submitted.is_none() && self.validation().is_ok()),
            ),
        ]
        .into_iter()
        .enumerate()
        {
            let _ = write!(
                svg,
                r##"<rect x="{x}" y="{BUTTON_Y}" width="{width}" height="75" rx="8" fill="{}" stroke="{}" stroke-width="3"/>"##,
                if enabled { "#3a3022" } else { "#15171b" },
                if self.focus == button + ROWS {
                    "#f0a030"
                } else {
                    "#4a4640"
                }
            );
            text(
                &mut svg,
                x + 44.0,
                BUTTON_Y + 51.0,
                35.0,
                label,
                if enabled { "#f0a030" } else { "#707174" },
            );
        }
        if let Some(menu) = &self.menu {
            let [left, top, width, height] = menu.rectangle();
            let _ = write!(
                svg,
                r##"<rect x="{left}" y="{top}" width="{width}" height="{height}" rx="8" fill="#1f2228" stroke="#82715c" stroke-width="4"/>"##
            );
            for (visible, (label, choice)) in menu
                .options
                .iter()
                .skip(menu.top)
                .take(menu.visible())
                .enumerate()
            {
                let y = top + visible as f64 * 64.0;
                if menu.top + visible == menu.highlighted {
                    let _ = write!(
                        svg,
                        r##"<rect x="{}" y="{y}" width="{}" height="64" fill="#3a3022"/>"##,
                        left + 4.0,
                        width - 8.0
                    );
                }
                if self.selected(choice) {
                    let _ = write!(
                        svg,
                        r##"<path d="M {} {} l 11 11 l 22 -25" fill="none" stroke="#f0a030" stroke-width="5" stroke-linecap="round" stroke-linejoin="round"/>"##,
                        left + 28.0,
                        y + 31.0
                    );
                }
                text(
                    &mut svg,
                    left + 85.0,
                    y + 45.0,
                    32.0,
                    &ellipsize(label, 95),
                    "#ebe6d8",
                );
            }
            let footer_y = top + menu.visible() as f64 * 64.0 + 44.0;
            let footer = if menu.options.len() > 10 {
                "↑ Previous page                                Next page ↓"
            } else if menu.row == 6 {
                "Click to select inputs; click outside to finish"
            } else {
                "↑ ↓ move   ·   Enter selects   ·   Escape closes"
            };
            text(&mut svg, left + 30.0, footer_y, 27.0, footer, "#a6a5a1");
        }
        svg.push_str("</g></svg>");
        svg
    }
}

fn channels(channels: [u16; 2]) -> String {
    if channels[0] == channels[1] {
        format!("Mono {}", u32::from(channels[0]) + 1)
    } else {
        format!(
            "Stereo {} / {}",
            u32::from(channels[0]) + 1,
            u32::from(channels[1]) + 1
        )
    }
}
fn stepped<'a, T: PartialEq>(choices: &'a [T], current: &T, direction: i32) -> Option<&'a T> {
    if choices.is_empty() {
        return None;
    }
    let index = choices
        .iter()
        .position(|choice| choice == current)
        .unwrap_or(0) as i32;
    choices.get((index + direction).rem_euclid(choices.len() as i32) as usize)
}
fn ellipsize(text: &str, maximum: usize) -> String {
    if text.chars().count() <= maximum {
        text.to_owned()
    } else {
        text.chars()
            .take(maximum.saturating_sub(1))
            .chain(std::iter::once('…'))
            .collect()
    }
}
fn text(svg: &mut String, x: f64, y: f64, size: f64, value: &str, color: &str) {
    let value: String = value
        .chars()
        .filter_map(|c| match c {
            '&' => Some("&amp;".to_owned()),
            '<' => Some("&lt;".to_owned()),
            '>' => Some("&gt;".to_owned()),
            '\u{0}'..='\u{1f}' | '\u{fffe}' | '\u{ffff}' => None,
            c => Some(c.to_string()),
        })
        .collect();
    let _ = write!(
        svg,
        r#"<text x="{x}" y="{y}" font-size="{size}" fill="{color}">{value}</text>"#
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use plugin_kit_standalone::{
        AppOptions, AudioBlock, AudioProcessor, DeviceInfo, ProcessConfig, StreamCapability,
    };
    struct Noop;
    impl AudioProcessor for Noop {
        fn prepare(&mut self, _config: ProcessConfig) -> Result<(), String> {
            Ok(())
        }
        fn process(&mut self, _block: AudioBlock<'_>) {}
        fn deactivate(&mut self) {}
    }
    #[test]
    fn settings_choices_wrap_and_channel_names_are_explicit() {
        assert_eq!(
            stepped(&[None, Some(1), Some(16)], &None, -1),
            Some(&Some(16))
        );
        assert_eq!(stepped(&[32, 64, 128], &128, 1), Some(&32));
        assert_eq!(stepped::<u32>(&[], &0, 1), None);
        assert_eq!(channels([0, 0]), "Mono 1");
        assert_eq!(channels([2, 3]), "Stereo 3 / 4");
        assert_eq!(
            channels([u16::MAX; 2]),
            "Mono 65536",
            "invalid persisted channels must remain editable"
        );
        assert_eq!(ellipsize("12345", 3), "12…");
    }

    #[test]
    fn done_closes_without_applying_and_only_changed_settings_show_apply() {
        let directory = tempfile::tempdir().expect("test state folder");
        let mut runtime = plugin_kit_standalone::spawn(
            Box::new(Noop),
            None,
            AppOptions {
                app_id: "ca72-settings-done-test".into(),
                state_dir: directory.path().to_owned(),
                device_access: false,
            },
        )
        .expect("test host without devices");
        runtime.wait_ready(Duration::from_secs(2)).expect("ready");
        let mut dialog = Dialog::new(runtime.handle());
        let mut snapshot = dialog.snapshot.clone();
        snapshot.config.midi_inputs = vec!["Remembered keyboard".into()];
        snapshot.config.input = Some(DeviceId {
            name: "Remembered interface".into(),
            input_channels: 2,
            output_channels: 2,
        });
        snapshot.config.input_channels = [1, 1];
        snapshot.config.output_channels = [1, 1];
        dialog.update_snapshot(snapshot);
        assert!(dialog.svg().contains(">DONE</text>"));
        assert!(!dialog.svg().contains(">APPLY</text>"));
        assert!(dialog.press(X + WIDTH - 200.0, BUTTON_Y + 30.0));
        dialog.focus = ROWS + 2;
        assert!(dialog.key(&KeyboardEvent {
            key: Key::Enter,
            ..Default::default()
        }));
        assert!(dialog.submitted.is_none(), "Done never submits settings");
        let unchanged = dialog.draft.clone();
        dialog.choose(Choice::MidiAuto);
        dialog.choose(Choice::Output(dialog.draft.output.clone()));
        dialog.choose(Choice::Input(dialog.draft.input.clone()));
        dialog.cycle(0, 1);
        assert_eq!(
            dialog.draft, unchanged,
            "no-op choices preserve dependent settings"
        );
        assert!(!dialog.has_changes());
        dialog.choose(Choice::Midi(None));
        dialog.choose(Choice::MidiAuto);
        assert!(
            !dialog.has_changes(),
            "returning to Automatic restores its remembered input"
        );
        let mut manual = dialog.snapshot.clone();
        manual.config.auto_midi = false;
        manual.config.midi_inputs = vec!["Port A".into(), "Port B".into()];
        dialog.update_snapshot(manual);
        dialog.choose(Choice::Midi(Some("Port A".into())));
        dialog.choose(Choice::Midi(Some("Port A".into())));
        assert!(
            !dialog.has_changes(),
            "input selection order does not count as an edit"
        );
        let mut automatic = dialog.snapshot.clone();
        automatic.config = unchanged;
        dialog.update_snapshot(automatic);
        if let Some(path) = std::env::var_os("CA72_STANDALONE_DONE_PROOF") {
            let scale = 1380.0 / art::W;
            let mut pixmap = Pixmap::new(1380, (art::H * scale).round() as u32).expect("pixels");
            dialog.draw(&mut pixmap, scale);
            pixmap.save_png(path).expect("saved Done settings proof");
        }

        let latest = dialog.app.snapshot();
        dialog.stamp = format!("{latest:?}");
        let mut older_accepted = latest.config.clone();
        older_accepted.midi_inputs = vec!["Older accepted input".into()];
        dialog.submitted = Some(older_accepted.clone());
        dialog.complete_application(Ok(older_accepted));
        dialog.next_poll = Instant::now();
        dialog.frame();
        assert_eq!(
            dialog.draft, latest.config,
            "completion must reconcile a newer snapshot already observed while pending"
        );

        dialog.choose(Choice::Channel(None));
        assert!(
            !dialog.has_changes(),
            "choosing the current value is a no-op"
        );
        dialog.choose(Choice::Channel(Some(2)));
        assert!(dialog.svg().contains(">APPLY</text>"));
        assert!(
            !dialog.finish_or_apply(),
            "failed Apply keeps the dialog open"
        );
        assert!(
            dialog.local_error.is_some(),
            "no output device is available"
        );
        assert!(dialog.has_changes());
        dialog.choose(Choice::Channel(None));
        assert!(!dialog.has_changes(), "reverting the edit restores Done");
        dialog.cycle(7, 1);
        assert!(dialog.has_changes());
        dialog.cycle(7, -1);
        assert!(
            !dialog.has_changes(),
            "arrow-key edits use the same comparison"
        );
        assert!(
            dialog.finish_or_apply(),
            "Done works even without audio devices"
        );
        runtime.shutdown().expect("test host closes");
    }

    #[test]
    fn open_device_menu_follows_inventory_without_changing_the_draft() {
        let directory = tempfile::tempdir().expect("test state folder");
        let mut runtime = plugin_kit_standalone::spawn(
            Box::new(Noop),
            None,
            AppOptions {
                app_id: "ca72-live-midi-menu-test".into(),
                state_dir: directory.path().to_owned(),
                device_access: false,
            },
        )
        .expect("test host without devices");
        runtime.wait_ready(Duration::from_secs(2)).expect("ready");
        let mut dialog = Dialog::new(runtime.handle());
        dialog.choose(Choice::Midi(None));
        dialog.open_menu(6);
        assert_eq!(
            dialog.menu.as_ref().expect("MIDI selector").options.len(),
            2
        );
        let original = dialog.draft.clone();
        let mut snapshot = dialog.snapshot.clone();
        snapshot.devices.midi_inputs = vec!["Keyboard Port 1".into(), "Keyboard Port 2".into()];
        dialog.update_snapshot(snapshot.clone());
        assert_eq!(
            dialog
                .menu
                .as_ref()
                .expect("selector stays open")
                .options
                .len(),
            4
        );
        assert_eq!(
            dialog.draft, original,
            "discovery does not activate an input"
        );
        assert!(dialog.svg().contains("2 detected; none selected"));
        let event = |key| KeyboardEvent {
            key,
            ..Default::default()
        };
        dialog.key(&event(Key::ArrowDown));
        dialog.key(&event(Key::Character(" ".into())));
        assert_eq!(dialog.draft.midi_inputs, ["Keyboard Port 1"]);
        snapshot.devices.midi_inputs.reverse();
        dialog.update_snapshot(snapshot.clone());
        let menu = dialog
            .menu
            .as_ref()
            .expect("selector stays open after reorder");
        assert_eq!(menu.options[menu.highlighted].0, "Keyboard Port 1");
        assert_eq!(dialog.draft.midi_inputs, ["Keyboard Port 1"]);
        snapshot.devices.midi_inputs = vec!["Keyboard Port 2".into()];
        dialog.update_snapshot(snapshot);
        let menu = dialog
            .menu
            .as_ref()
            .expect("selector stays open after unplug");
        assert_eq!(menu.options.len(), 3);
        assert!(menu.highlighted < menu.options.len());
        assert!(
            !menu
                .options
                .iter()
                .any(|(label, _)| label == "Keyboard Port 1")
        );
        assert_eq!(
            dialog.draft.midi_inputs,
            ["Keyboard Port 1"],
            "draft is preserved for reconnection"
        );
        assert_eq!(
            runtime.handle().snapshot().config,
            Config::initial(),
            "nothing applied implicitly"
        );
        runtime.shutdown().expect("test host closes");
    }

    #[test]
    fn automatic_midi_resolution_updates_only_untouched_or_applied_drafts() {
        let directory = tempfile::tempdir().expect("test state folder");
        let mut runtime = plugin_kit_standalone::spawn(
            Box::new(Noop),
            None,
            AppOptions {
                app_id: "ca72-automatic-midi-test".into(),
                state_dir: directory.path().to_owned(),
                device_access: false,
            },
        )
        .expect("test host without devices");
        runtime.wait_ready(Duration::from_secs(2)).expect("ready");
        let mut dialog = Dialog::new(runtime.handle());
        assert!(dialog.selected(&Choice::MidiAuto));
        assert!(!dialog.selected(&Choice::Midi(None)));
        let mut snapshot = dialog.snapshot.clone();
        snapshot.devices.midi_inputs = vec!["Keyboard Port 1".into(), "Keyboard Port 2".into()];
        snapshot.config.midi_inputs = vec!["Keyboard Port 1".into()];
        dialog.update_snapshot(snapshot.clone());
        assert_eq!(dialog.draft.midi_inputs, ["Keyboard Port 1"]);
        assert!(dialog.svg().contains("Automatic · Keyboard Port 1"));

        dialog.choose(Choice::Midi(None));
        assert!(!dialog.draft.auto_midi);
        assert!(dialog.draft.midi_inputs.is_empty());
        snapshot.devices.midi_inputs.reverse();
        dialog.update_snapshot(snapshot.clone());
        assert!(dialog.selected(&Choice::Midi(None)));
        assert!(dialog.draft.midi_inputs.is_empty(), "keep unapplied edits");
        dialog.choose(Choice::Midi(Some("Keyboard Port 2".into())));
        assert!(!dialog.draft.auto_midi);
        assert_eq!(dialog.draft.midi_inputs, ["Keyboard Port 2"]);

        // Apply can normalise both the device-default rate and Automatic's
        // selected port. Once the host publishes that result, the draft follows.
        let id = DeviceId {
            name: "Test output".into(),
            input_channels: 0,
            output_channels: 2,
        };
        snapshot.devices.default_output = Some(id.clone());
        snapshot.devices.audio = vec![DeviceInfo {
            id,
            input_configs: Vec::new(),
            output_configs: vec![StreamCapability {
                channels: 2,
                min_sample_rate: 48_000,
                max_sample_rate: 48_000,
                buffer_range: Some([16, 8192]),
            }],
            sample_rates: vec![48_000],
            default_sample_rate: 48_000,
        }];
        snapshot.config.auto_midi = false;
        snapshot.config.midi_inputs = vec!["Keyboard Port 2".into()];
        dialog.update_snapshot(snapshot.clone());
        dialog.choose(Choice::MidiAuto);
        assert!(dialog.draft.auto_midi);
        assert!(dialog.draft.midi_inputs.is_empty());
        dialog.apply();
        assert!(dialog.local_error.is_none());
        assert!(dialog.submitted.is_some());
        assert!(
            dialog.has_changes(),
            "Apply stays until host acknowledgement"
        );
        dialog.update_snapshot(snapshot.clone());
        assert!(
            dialog.has_changes(),
            "an old snapshot cannot reset the draft"
        );
        assert!(dialog.draft.midi_inputs.is_empty());
        snapshot.phase = Phase::Running;
        snapshot.config = negotiate(&dialog.draft, &snapshot.devices)
            .expect("supported output")
            .config;
        snapshot.config.midi_inputs = vec!["Keyboard Port 2".into()];
        dialog.complete_application(Ok(snapshot.config.clone()));
        dialog.update_snapshot(snapshot.clone());
        assert_eq!(dialog.draft, snapshot.config);
        assert!(dialog.submitted.is_none());
        assert!(!dialog.has_changes(), "normalised success restores Done");
        assert!(dialog.svg().contains("Automatic · Keyboard Port 2"));

        // Equality with the current snapshot is not proof of completion, even
        // when choosing the device-default rate normalises to the running rate.
        dialog.choose(Choice::Rate(0));
        dialog.apply();
        dialog.update_snapshot(snapshot.clone());
        assert!(
            dialog.has_changes(),
            "matching old config cannot acknowledge a request"
        );
        dialog.complete_application(Err("Same repeated transition failure".into()));
        assert!(dialog.submitted.is_none(), "failed receipt permits retry");
        assert!(dialog.has_changes());
        dialog.apply();
        dialog.complete_application(Ok(snapshot.config.clone()));
        assert!(!dialog.has_changes());

        // Returning to the submitted value resumes following automatic updates
        // once that request completes; it must not leave a stale edit latch.
        dialog.choose(Choice::Channel(Some(2)));
        dialog.apply();
        let queued = dialog.submitted.clone();
        dialog.choose(Choice::Channel(Some(3)));
        dialog.apply();
        assert_eq!(
            dialog.submitted, queued,
            "a pending Apply cannot be replaced"
        );
        dialog.choose(Choice::Channel(Some(2)));
        let mut applied = snapshot.config.clone();
        applied.midi_channel = Some(2);
        dialog.complete_application(Ok(applied.clone()));
        assert!(!dialog.has_changes());
        let mut automatic = snapshot.clone();
        automatic.config = applied;
        automatic.config.midi_inputs = vec!["Keyboard Port 1".into()];
        dialog.update_snapshot(automatic);
        assert_eq!(dialog.draft.midi_inputs, ["Keyboard Port 1"]);
        assert!(
            !dialog.has_changes(),
            "completed edits follow automatic host updates"
        );
        dialog.update_snapshot(snapshot.clone());

        dialog.choose(Choice::Channel(Some(2)));
        dialog.apply();
        snapshot.config.midi_channel = Some(2);
        dialog.choose(Choice::Channel(None));
        assert_eq!(dialog.draft, dialog.snapshot.config);
        assert!(dialog.has_changes(), "an earlier Apply is still pending");
        dialog.complete_application(Ok(snapshot.config.clone()));
        dialog.update_snapshot(snapshot.clone());
        assert_eq!(
            dialog.draft.midi_channel, None,
            "editing back to the old value still counts as a post-Apply edit"
        );
        assert!(
            dialog.has_changes(),
            "a late acknowledgement keeps the newer edit"
        );

        snapshot.config.auto_midi = false;
        dialog.update_snapshot(snapshot.clone());
        dialog.choose(Choice::Midi(None));
        dialog.choose(Choice::MidiAuto);
        dialog.apply();
        dialog.choose(Choice::Channel(Some(2)));
        let mut accepted = snapshot.config.clone();
        accepted.midi_channel = None;
        dialog.complete_application(Ok(accepted.clone()));
        snapshot.config = accepted;
        dialog.update_snapshot(snapshot);
        assert_eq!(dialog.draft.midi_channel, Some(2));
        assert!(
            dialog.draft.midi_inputs.is_empty(),
            "keep edits made after Apply"
        );
        runtime.shutdown().expect("test host closes");
    }

    #[test]
    fn dialog_uses_capabilities_and_cancel_preserves_running_config() {
        let directory = tempfile::tempdir().expect("test state folder");
        let mut runtime = plugin_kit_standalone::spawn(
            Box::new(Noop),
            None,
            AppOptions {
                app_id: "ca72-settings-test".into(),
                state_dir: directory.path().to_owned(),
                device_access: false,
            },
        )
        .expect("test host without devices");
        runtime.wait_ready(Duration::from_secs(2)).expect("ready");
        let mut dialog = Dialog::new(runtime.handle());
        let id = DeviceId {
            name: "Virtual & interface".into(),
            input_channels: 2,
            output_channels: 4,
        };
        let capability = StreamCapability {
            channels: 4,
            min_sample_rate: 44_100,
            max_sample_rate: 48_000,
            buffer_range: Some([64, 1024]),
        };
        dialog.snapshot.devices.backend = "Test backend".into();
        dialog.snapshot.devices.default_output = Some(id.clone());
        dialog.snapshot.devices.default_sample_rate = 48_000;
        dialog.snapshot.devices.audio = vec![DeviceInfo {
            id: id.clone(),
            input_configs: vec![StreamCapability {
                channels: 2,
                ..capability.clone()
            }],
            output_configs: vec![capability],
            sample_rates: vec![44_100, 48_000],
            default_sample_rate: 48_000,
        }];
        dialog.snapshot.devices.midi_inputs =
            vec!["Keyboard < one >".into(), "Keyboard two".into()];
        assert!(
            dialog.draft.input.is_none(),
            "external input starts disabled"
        );
        assert!(dialog.validation().is_ok());
        assert_eq!(dialog.channel_choices(false).len(), 7);
        assert!(!dialog.press(VALUE_X + 20.0, ROW_Y + 20.0));
        let menu = dialog.menu.as_ref().expect("explicit output dropdown");
        assert!(menu.options[1].0.contains("Virtual & interface"));
        let [left, top, _, _] = menu.rectangle();
        dialog.press(left + 100.0, top + 64.0 + 20.0);
        assert!(dialog.menu.is_none());
        assert_eq!(dialog.draft.output, Some(id.clone()));
        dialog.cycle(2, 1);
        assert_eq!(dialog.draft.input, Some(id));
        dialog.choose(Choice::Midi(Some("Keyboard two".into())));
        assert_eq!(dialog.draft.midi_inputs, ["Keyboard two"]);
        dialog.open_menu(6);
        let [left, top, _, _] = dialog
            .menu
            .as_ref()
            .expect("multi MIDI dropdown")
            .rectangle();
        let index = dialog
            .menu
            .as_ref()
            .expect("MIDI list")
            .options
            .iter()
            .position(|(label, _)| label == "Keyboard < one >")
            .expect("first port");
        dialog.press(left + 100.0, top + index as f64 * 64.0 + 20.0);
        assert!(
            dialog.menu.is_some(),
            "multiple inputs can be selected in one list"
        );
        assert_eq!(dialog.draft.midi_inputs.len(), 2);
        dialog.press(X, Y);
        assert!(dialog.menu.is_none());
        dialog.cycle(7, -1);
        assert_eq!(dialog.draft.midi_channel, Some(16));
        assert!(dialog.press(X + WIDTH - 600.0, BUTTON_Y + 20.0));
        assert_eq!(
            runtime.handle().snapshot().config,
            Config::initial(),
            "cancel never submitted the draft"
        );
        assert!(dialog.svg().contains("Virtual &amp; interface"));
        let scale = 1380.0 / art::W;
        let mut pixmap = Pixmap::new(1380, (art::H * scale).round() as u32).expect("pixels");
        dialog.draw(&mut pixmap, scale);
        assert!(
            pixmap
                .data()
                .chunks_exact(4)
                .any(|pixel| pixel[0] > 120 && pixel[1] > 50),
            "dialog has rendered lettering"
        );
        if let Some(path) = std::env::var_os("CA72_STANDALONE_SETTINGS_PROOF") {
            pixmap.save_png(&path).expect("saved settings proof");
            dialog.open_menu(0);
            let mut menu_pixmap =
                Pixmap::new(1380, (art::H * scale).round() as u32).expect("menu pixels");
            dialog.draw(&mut menu_pixmap, scale);
            let menu_path =
                std::path::PathBuf::from(path).with_file_name("settings-dropdown-proof.png");
            menu_pixmap
                .save_png(menu_path)
                .expect("saved dropdown proof");
        }
        dialog.menu = None;
        dialog.open_menu(7);
        let key = |key| KeyboardEvent {
            state: KeyState::Down,
            key,
            ..KeyboardEvent::default()
        };
        dialog.key(&key(Key::Home));
        dialog.key(&key(Key::ArrowDown));
        dialog.key(&key(Key::Enter));
        assert_eq!(dialog.draft.midi_channel, Some(1));
        assert!(dialog.menu.is_none());
        runtime
            .shutdown()
            .expect("bounded shutdown without callbacks");
    }

    #[test]
    fn session_warnings_open_the_dialog_once_until_the_condition_changes() {
        let directory = tempfile::tempdir().expect("test state folder");
        std::fs::write(directory.path().join("session.json"), "{broken")
            .expect("invalid session fixture");
        let mut runtime = plugin_kit_standalone::spawn(
            Box::new(Noop),
            None,
            AppOptions {
                app_id: "ca72-settings-alert-test".into(),
                state_dir: directory.path().to_owned(),
                device_access: false,
            },
        )
        .expect("test host without devices");
        runtime.wait_ready(Duration::from_secs(2)).expect("ready");
        AppHandle::install_current(&runtime.handle());
        assert!(
            take_requested().is_some(),
            "a malformed session must be visible without opening settings manually"
        );
        assert!(
            take_requested().is_none(),
            "same warning must not reopen a dismissed dialog"
        );
        std::thread::sleep(Duration::from_millis(210));
        assert!(take_requested().is_none());
        request_open();
        assert!(
            take_requested().is_some(),
            "manual settings remain available after dismissing warning"
        );
        let error = runtime
            .shutdown()
            .expect_err("preserved malformed session prevents final save");
        assert!(error.contains("preserved"));
        assert!(
            runtime
                .handle()
                .snapshot()
                .warning
                .as_deref()
                .is_some_and(|warning| warning.contains("preserved"))
        );
        assert_eq!(
            std::fs::read_to_string(directory.path().join("session.json")).expect("preserved"),
            "{broken"
        );
    }
}
