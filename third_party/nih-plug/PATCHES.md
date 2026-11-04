# nih-plug, patched

**Linux standalone identity, 2026-10-18 (CA-72 issue #49).** The standalone opens its
window through the repository's patched baseview on every platform (R-WIN-STANDALONE), so on
Linux the top-level window takes the application's `WM_CLASS` from baseview's change 10 (set by
`ca72_plugin::identify_standalone()`). The previous separate git dependency did not set it. See
`docs/issue-49-linux-icon.md` and `scripts/test-linux-window-identity.py`.

The `nih_plug` crate and its derive macros from
[robbert-vdh/nih-plug](https://github.com/robbert-vdh/nih-plug) at commit
`de421011f41a6d10fc8c7a6084e4f4dee0143683` (ISC, `LICENSE`; its VST3 bindings are
GPL-3.0). The manifest's `[workspace]` and `[profile]` tables are removed, and two of
upstream's own compiler warnings are allowed. The examples, the GUI adapters and the other
plug-ins are left out.

The commit is the project's latest. It fails four of clap-validator 0.4.1's tests, and so
do its own example plug-ins (`sine`, `gain`). These two changes, both in
`src/wrapper/clap/wrapper.rs`, fix them:

1. **A corrupt state no longer aborts the host.** `ext_state_load` reserved the length
   the stream's first eight bytes claim before reading any of it. With random bytes that
   is about 10^18 bytes, and the failed allocation aborts the process (clap-validator's
   `state-invalid-random`). The buffer now grows as the data arrives, 64 KiB at a time,
   so a short or corrupt stream fails the load instead.
2. **The host is told the values changed after a state load.** `set_state_inner` now also
   asks the host to rescan the parameters' values (`CLAP_PARAM_RESCAN_VALUES`). Without
   that, a host may keep showing the values from before the load
   (`state-reproducibility-basic`, `-binary` and `-buffered`).

A third change, in `src/wrapper/vst3/wrapper.rs`, comes from Steinberg's VST3 validator
(SDK 3.8.1):

3. **A bus arrangement is matched against the right buses.** `set_bus_arrangements`
   took a layout's first auxiliary bus to be the host's bus 1 when the layout has no main
   bus, and bus 0 when it has one: the wrong way round (the rest of the file has it
   right). For an instrument with a side chain it read past the end of the host's
   one-element array, and so refused every arrangement the host asked for (the
   validator's mono test: "Mono Input-SpeakerArrangement is not supported").

Five more came from a review of the plug-in before its release (2026-10-03; the CA-72's
`docs/decisions.md` R18):

4. **A VST3 host's refusal to resize reaches the editor.** `request_resize` always
   returned `true`, whatever the host answered, so an editor could not tell that its
   window had not grown (the CA-72's presets' drawer, opening below the panel, went
   unseen while it held the keyboard). On the GUI thread, where the host answers at once
   (macOS, Windows), it now returns the host's answer; elsewhere (Linux, where baseview's
   window has a thread of its own) it still schedules the request and returns `true`.
   `WrapperView::request_resize` (`src/wrapper/vst3/view.rs`) no longer asserts that the
   host agreed, which stopped a debug build at a refusal.
5. **Auxiliary buses are bounded by the host's own count.** In `process`, the VST3 wrapper
   bounded the auxiliary input buses by the host's number of output buses, and both
   wrappers compared with `>`, so a bus at the host's count was read one past the end of its
   array (`src/wrapper/vst3/wrapper.rs`: inputs by `num_inputs`, both with `>=`;
   `src/wrapper/clap/wrapper.rs`: both with `>=`).
6. **A side chain's missing channels are as long as the block.** Channels the host did not
   supply (too few of them, or no pointers) were zeroed at whatever length an earlier block
   left them, which may be shorter than this block's, and a plug-in reading them by the
   block's samples panicked. They are now resized to the block first, within the room
   reserved for them (`src/wrapper/util/buffer_management.rs`).
7. **VST3's buffer configuration carries the processing mode just set.**
   `setup_processing` stored it with the mode from before the call, so `initialize` saw the
   previous mode: realtime for an offline render, offline for playback after one
   (`src/wrapper/vst3/wrapper.rs`).
8. **The processing mode in `process`.** `ProcessContext::process_mode()`
   (`src/context/process.rs`; the VST3, CLAP and standalone contexts). CLAP's `render`
   extension may change the mode while the plug-in is active, and the plug-in is not
   initialized again for it, so `BufferConfig::process_mode` alone may be out of date.

One more came from a report: the CA-72 crashed in Sandyne, a DAW built with JUCE (2026-10-06;
the CA-72's `docs/decisions.md` R31):

9. **A channel the host gives as a null pointer is not read.** A VST3 host may give a null
   pointer for a channel, and JUCE's hosts do for every channel of a bus they deactivated,
   with the bus's full channel count (`HostBufferMapper::associateBufferTo`); setting up an
   instrument with no inputs deactivates its side chain. `create_buffers` read every input
   channel through its pointer, so an instrument with a side chain read from address 0 in
   its first block, an access violation. A null input channel is now read as silence, and a
   null output channel is backed by scratch storage, allocated with the rest and discarded,
   so that every channel the plug-in sees is as long as the block
   (`src/wrapper/util/buffer_management.rs`, with a test of each kind of channel).

Two more came with the CA-72's MIDI Learn (2026-10-06; `docs/decisions.md` R34), which sets a
parameter from the audio thread when a learned MIDI controller moves. Upstream left that as a
`TODO` in `ProcessContext` (`set_parameter`), and the plugin must not use the editor's
`ParamSetter` there (its host calls belong on the GUI thread) nor keep a value of its own that the
host, the editor and the saved state would not see:

10. **`ProcessContext::set_parameter_normalized(param, normalized, timing)`**
    (`src/context/process.rs`). The value is set at once, as each wrapper already sets the
    host's automation during a process call (its smoother updated, the editor told), so the
    plugin reads it from there on; and the host is told as its format has it:
    - **CLAP:** a `CLAP_EVENT_PARAM_VALUE` output event at the change's time, flagged
      `CLAP_EVENT_DONT_RECORD`, no gesture: `clap/ext/params.h`'s "Turning a knob via plugin's
      internal MIDI mapping", where recording both the MIDI and the parameter would conflict
      (`src/wrapper/clap/context.rs`; `Wrapper::set_own_parameter` and `handle_out_events` in
      `src/wrapper/clap/wrapper.rs`, after the editor's changes, in time order). Nothing more:
      Bitwig Studio 5.2.7 does not apply such an event to its own display (without the flag it
      did, and made an undo step of each), but a values rescan after each block was not the
      cure, since clap-wrapper's Audio Unit takes any rescan for a new parameter list.
    - **VST3:** a point at the change's sample offset in the process call's
      `outputParameterChanges`, the processor's way to tell the host and its controller of a
      change (`src/wrapper/vst3/context.rs`, `inner.rs`; drained after each run in
      `src/wrapper/vst3/wrapper.rs`). VST3 has no flag against recording: Steinberg notes that
      some hosts write automation for these, and Ableton Live 12.2.7 does while its automation
      is armed.
    - **The standalone:** set at once; no host to tell (`src/wrapper/standalone/`).

    Each wrapper reserves room for 1024 such changes a process call when it is made
    (`OWN_PARAM_CHANGES`, `src/wrapper/util.rs`), so nothing is allocated, locked or waited for
    on the audio thread; past it a change is set but not told, and the call returns `false`.
    And **at one sample, the host's parameter changes come before the events the plugin reads**,
    whatever the host's order, so that a controller setting a parameter follows the automation of
    it at that sample rather than winning or losing by chance: in VST3 the process call's events
    are sorted by (time, parameter change first) (MIDI CCs arrive as parameter changes there too,
    in the host's queue order); in CLAP, `handle_in_events_until` splits the run before an event
    that a parameter change follows at its sample (`split_before`).
11. **`TestProcessContext`** (`src/context/process.rs`): a `ProcessContext` with no host, for a
    plugin's own tests: the events pushed, a change made through change 10 set as a wrapper sets
    it and kept as a host would be told it, and `automate()` setting a parameter as a host's
    automation is set between runs. It touches only the parameters of the `Params` it is made
    with. (A plugin cannot set a parameter outside nih-plug otherwise: its setters are
    crate-private.)
    Its `set_transport()` gives the next runs a host's transport (playing, tempo, position in
    quarter notes, time signature), for a plugin that follows it (the CA-72's arpeggiator,
    `docs/decisions.md` R-ARP; the transport's fields are crate-private too).
    It says it is the standalone (`PluginApi::Standalone`) until `set_plugin_api()` tells it
    otherwise, for what a plugin does in its standalone alone (the CA-72's TEMPO as a
    modulation target, R-LFO).

One more came from a report: the CA-72's switches and buttons did nothing in Cubase 15 on
Windows 11 (2026-10-07; the CA-72's `docs/decisions.md` R36):

12. **An edit made in the editor while the host processes audio is held, not left to the
    host.** While the host was processing, `raw_set_parameter_normalized` left the value
    alone, for the host to send the edit back to the processor, so that a value never changes
    in the middle of a process call; until then `IEditController::getParamNormalized()`
    reported the value from before the edit. Cubase 15 reads that value within `endEdit()` and
    sends it to the processor at the next process call, after the edit. So a click, a whole
    gesture between two process calls, set nothing, and a drag let go between two calls ended
    one move short. A host that does not send the edit back at all (upstream's `FIXME` names
    REAPER, which stops calling a plugin it bypasses) left it unset. The edit is now held, the
    latest for each parameter (`held_edits`, `src/wrapper/vst3/inner.rs`): the controller
    reports it, the next process call sets it at its start, before the host's own changes at
    its first sample, and `setProcessing(false)` sets it if no call will
    (`src/wrapper/vst3/context.rs`, `wrapper.rs`). Values still change only between process
    calls, and nothing is allocated, locked or waited for on the audio thread: a flag, and an
    atomic for each parameter. The CA-72's `crates/ca72-plugin/tests/vst3_host.rs` drives the
    wrapper through its factory as Cubase 15 does; four of its six tests fail without this.

One more from the CA-72's own testing in Cubase 15 on Windows 11 (2026-10-10; its
`docs/decisions.md` R53): a value typed into its editor never arrived there.

13. **The keys a host gives a VST3 plugin's view reach its editor.** Cubase keeps the keyboard
    from a plugin's own window and gives the plugin its keys through `IPlugView::onKeyDown()`
    and `onKeyUp()`, using those the plugin does not take as its own key commands (Escape closes
    the plugin's window). The wrapper answered `kNotImplemented`, so no key ever reached the
    editor. They are now offered to it (`Editor::on_host_key()`, `HostKey`: VST3's character,
    `VirtualKeyCodes` code, `KeyModifier` flags, down or up; `src/editor.rs`), and the host is
    told `kResultTrue` if the editor took the key, else `kResultFalse` (`src/wrapper/vst3/view.rs`).
    An editor that does not override it takes none, as before. The CA-72's
    `crates/ca72-plugin/tests/vst3_host.rs` gives the view keys as Cubase does.

14. **Queued resize refusals reach the editor.** VST3's frame captures an editor's resize
    request id and size before calling the host, then reports its final answer through
    `Editor::resize_result`, including requests queued onto Linux's `IRunLoop`. The context
    identifies wrappers that send these replies; a dropped request reports failure. The
    default editor hooks retain existing behaviour. The CA-72 uses monotonically numbered
    GUI transactions to restore the last confirmed view after refusal and ignore replies
    from superseded requests or closed windows. `tests/vst3_host.rs` exercises synchronous
    answers and a refusal delivered through the actual Linux run-loop socket.

15. **A remote wrapper can opt out of native-window occlusion.** The additive
    `GuiContext::native_window_occlusion_reliable()` defaults to true. CLAP queries the
    private presence-only host capability
    `com.idlefoundry.native-window-occlusion-unreliable/1` once during initialization,
    caches the answer per instance, and exposes it through the GUI context. The AUv2
    wrapper advertises this capability because a remotely presented editor's local
    NSWindow need not describe its visible proxy. `PluginApi` remains `Clap`; ordinary
    CLAP hosts, VST3 and both standalone adapters retain the default. The filtered
    `visibility_tests` unit tests exercise the actual CLAP initialization and context;
    CI runs them explicitly because this vendored crate is outside the workspace.

16. **A VST3 host's brightness arrives as brightness.** `NoteExpressionController::
    translate_event` (`src/wrapper/vst3/note_expressions.rs`) turned `kBrightnessTypeID` into
    `NoteEvent::PolyExpression` and `kExpressionTypeID` into `PolyBrightness`, the opposite of
    its own `translate_event_reverse` and of the CLAP wrapper. The CA-72 takes brightness as a
    note's timbre (its `docs/decisions.md` R-MPE, issue 81), so a VST3 host's MPE timbre went
    nowhere. They now map each to its own (the module's own test, run as
    `cargo test --manifest-path third_party/nih-plug/Cargo.toml --features vst3 --lib
    note_expressions`; not yet in CI).

One more came from a report: in REAPER 7.83 the CA-72's VST3 knobs went back where they had
been as the mouse was let go, the CLAP's and the standalone's did not (CA-72 issue #146,
2026-11-03; its `docs/decisions.md` R-VST3-QUIET-HOST):

17. **An edit made while the host has stopped calling `process()` is set at once.** Change 12
    holds an edit made while the host processes audio for the next process call. REAPER stops
    calling a plugin without `setProcessing(false)` (a silent track while its transport is
    stopped; a plugin it bypasses: upstream's `FIXME`), so the edit waited for a call that did
    not come, the controller reporting it but the parameter not having it, and an editor
    drawing the parameter's value drew it back as the gesture ended. (The CLAP wrapper asks the
    host for a flush with every edit, which REAPER answers on its main thread while not
    processing.) The wrapper now marks each `process()` call's start and return (`in_process`,
    `last_process`, `mark_process()`: two atomic stores and a read of the monotonic clock on
    the audio thread, nothing allocated or waited for), and from `setProcessing(true)`. When no
    call is running and none has returned for 0.25 s or four of the host's largest blocks,
    whichever is longer (`PROCESS_QUIET`, `host_stopped_calling()`, `src/wrapper/vst3/inner.rs`),
    the editor's edit is set on its thread, with every edit held before it, so that none is set
    over it when the host calls again (`raw_set_parameter_normalized()`, and
    `raw_end_set_parameter()` for edits held before the host stopped;
    `src/wrapper/vst3/context.rs`), and the host's `setParamNormalized()`, dropped before for the
    processor to be sent the change, is set likewise, after any held edit
    (`src/wrapper/vst3/wrapper.rs`). An edit still held when the host goes quiet (a gesture let go
    just before the host's last call, in a note's tail) is set by the editor's thread once the
    host has been quiet that long: `GuiContext` gains `idle()` (`src/context/gui.rs`, nothing by
    default, for the CLAP wrapper and the standalone), which an editor calls each frame (the
    CA-72's does), and the VST3 wrapper's sets the held edits there if the host has stopped
    calling (`set_held_edits_if_host_stopped()`); nothing more is done on the audio thread. A host
    calling as it should is never quiet that long, and its edits are held as before. The flag of
    held edits (`any_held_edit`'s store in `hold_edit()`, its swap in `set_held_edits()`) and
    `in_process` (its store in `mark_process()`, its load in `host_stopped_calling()`) are
    `SeqCst`: the editor's thread stores the flag and then loads `in_process`, a call starting
    stores `in_process` and then swaps the flag, and with weaker orderings each could miss the
    other's store (a store buffer pattern, which x86 allows too).

    This leaves a race, of the kind upstream has between the editor's `is_processing` check and
    `setProcessing(true)`. The editor's thread (or the host's, in `setParamNormalized()`) can be
    preempted after `host_stopped_calling()` has returned true and before `set_held_edits()` has
    finished; a process call can start in between, and the value then changes during that call,
    not only at a call's start, and an older held edit that call sets can land after the host's
    value set at once. The audio thread and the editor's can then also set one parameter at the
    same time: `set_plain_value()` (`src/params/float.rs` and the other kinds) swaps `value` and
    then stores `normalized_value` and the unmodulated values separately, so those can briefly
    disagree, until the parameter is next set to another value. Everything there is atomic, so
    there is no undefined behaviour, and the CA-72 is not harmed: it uses none of nih-plug's
    smoothers or value callbacks, and reads its parameters' values as it uses them in each block.
    The CA-72's `crates/ca72-plugin/tests/vst3_host.rs` drives the wrapper as REAPER does (its
    `docs/decisions.md` R-VST3-QUIET-HOST records which of its tests fail without this).

18. **A CLAP block's first event, a parameter change at a later sample, is set at its sample.**
    `handle_in_events_until` (`src/wrapper/clap/wrapper.rs`) splits the block before each
    parameter change after the run's start, but it handled the run's first event before looking
    at it: a parameter change that was the host's first event of a process call was set at the
    block's first sample, up to a block early (upstream has the same code). It now ends the run
    at that event's sample, as it does for every later one (the CA-72's `docs/decisions.md`
    R-SHORT-BLOCKS; its test `a_parameter_change_first_in_a_block_is_set_at_its_sample`). VST3's
    wrapper looks at every change's time, the first's too, and was right.

To move to a newer upstream commit, copy its `Cargo.toml`, `LICENSE`, `README.md`, `src`
and `nih_plug_derive` here and apply the seventeen changes again, unless upstream has fixed them.
Then update the commit above and `nih_plug_xtask`'s `rev` in the workspace's `Cargo.toml`.

The standalone app adds a separate `standalone-production` feature. Its adapter
in `src/wrapper/standalone/production.rs` owns the instrument exclusively, keeps
the ordinary parameter/state/editor APIs, validates sessions before mutation,
and passes auxiliary input through to the same processing path. Audio task queues
are preallocated SPSC queues; task payloads must be `Copy`. The shared host is
`plugin-kit-standalone`, whose baseline avoids the development launcher's JACK
dependency. CA-72 explicitly retains its global CC120/123 reset semantics.
Enum state validation adds `EnumParamInner::has_id` without changing hosted APIs.
These files and the optional feature are also maintained in plugin-kit.

`ProcessContext::take_standalone_panic` defaults to false. The production context
lets opting-in instruments consume its boundary panic before their local MIDI
queues; otherwise it retains the synthetic CC120/121 sequence. CA-72 uses this to
coalesce the UI button and hardware-source reset without cancelling notes pressed
after the button. Ordinary hosted CC120/123 timing and global behavior are unchanged.

The production standalone's window (the CA-72's `docs/decisions.md` R-WIN-STANDALONE, issue
64): `app_window` in `production.rs`. On Windows the process is made aware of each monitor's
scale before the editor measures the screen (`app_window::scale`, the primary monitor's, where
the window opens), the editor is told that scale before it is first asked its size, and the
window opens at the same scale (`app_window::options`); before, baseview set the awareness only
after its first window was open and scaled the window by the monitor's scale while the editor,
told none, drew at 1, black beyond it at 125 % and more. A resize from outside (Windows scales a
window whose monitor's scale changes) is undone at the next frame on Windows (`refits`, `fit`).
The window is baseview's patched copy (`../baseview`, its change 9: no frame to drag and no
maximise box), not upstream's from git; the `opengl` feature, and with it `gl_config`, is gone
from both standalone adapters. macOS and Linux keep the system's scale policy, as before.

The production standalone's quitting (the CA-72's `docs/decisions.md` R-QUIT, 2026-10-19):
every way of quitting the app closes its window, as the close button does, so that
`run_with_midi_reset_scope` returns and shuts the app down as then (its session saved, audio
and MIDI stopped), and its caller's own way out follows. On macOS the menus and the quit
Apple event are baseview's change 11; on Windows Alt+F4 is the system's, which baseview leaves
to `DefWindowProcW`. On Linux `app_window::quits` is Ctrl+Q: the app's window closes at it
(`window.close()`, as `WM_DELETE_WINDOW` closes it), and an editor that has the keys (X11 gives
them to the window under the pointer) asks for it with `app_window::request_quit`, which the
window acts on at its next frame. The manifest takes `keyboard-types` for the key, as baseview
gives it, with the `standalone-production` feature.

The production standalone's window closing (the CA-72's `docs/decisions.md` R-QUIT-NAP,
2026-10-27): `app_window::on_closing` gives the function called once as the app's window
starts to close, however it is closed: at its `WillClose` (the close button; on macOS ⌘Q and
the quit Apple event, baseview's change 11), before `app_window::quits` and `request_quit`
close it, and as the window's handler is dropped otherwise. The CA-72 lifts App Nap there, so
that a napped app's shutdown runs at its ordinary pace. Nothing is called if none is given.

Production standalone PANIC identity: the additive ProcessContext token accessor
and per-block adapter metadata carry the exact token captured by plugin-kit's
panic flag. Read the token before consuming the panic; direct, untagged and hosted
callbacks retain their existing behavior. Adapter metadata is consumed even if a
malformed block is rejected, preventing stale acknowledgements on later calls.


## Detached offline rendering (R-REFERENCE-AUDIO, 2026-10-21)

`util::with_flush_to_zero` exposes the wrappers' existing scoped FTZ guard for detached
offline processing. It restores the caller's previous FP mode and permits output-buffer
allocation, so the MCP renderer can use the same processing environment without a live host.
The live wrappers' processing path is unchanged.
