//! Linux: `PipeWire` through its `PulseAudio` layer, in-process: stream events in, volume changes out. No helper
//! processes.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

use libpulse_binding as pulse;
use pulse::callbacks::ListResult;
use pulse::context::introspect::{SinkInfo, SinkInputInfo};
use pulse::context::subscribe::{Facility, InterestMaskSet, Operation};
use pulse::context::{Context, FlagSet, State};
use pulse::def::BufferAttr;
use pulse::mainloop::threaded::Mainloop;
use pulse::proplist::{properties, Proplist};
use pulse::sample::{Format, Spec};
use pulse::stream::{FlagSet as StreamFlags, PeekResult, Stream as PaStream};
use pulse::volume::{ChannelVolumes, Volume};

use crate::audio::{next_output, Command, Output, LOUD, LOUD_READINGS, METER_RATE, REPORT_EVERY};
use crate::matching::Stream;
use crate::Event;
use cranked_protocol::convert::round_u8;

fn output_from(info: &SinkInfo) -> Output {
    let name = info.name.as_deref().unwrap_or_default().to_string();
    let description = info.description.as_deref().unwrap_or(&name).to_string();
    let virtual_marker = name.contains("nvbroadcast") || description.to_lowercase().contains("nvidia broadcast");
    let real = info.proplist.get_str("device.api").is_some() && !virtual_marker;
    Output { name, description, real }
}

fn percent_to_volume(percent: u8) -> Volume {
    Volume(u32::try_from(u64::from(Volume::NORMAL.0) * u64::from(percent) / 100).unwrap_or(u32::MAX))
}

fn volume_to_percent(volume: Volume) -> f64 {
    f64::from(volume.0) * 100.0 / f64::from(Volume::NORMAL.0)
}

fn stream_from(info: &SinkInputInfo) -> Stream {
    let get = |key: &str| info.proplist.get_str(key).unwrap_or_default();
    Stream {
        index: info.index,
        app_name: get(properties::APPLICATION_NAME),
        binary: get(properties::APPLICATION_PROCESS_BINARY),
        app_id: get(properties::APPLICATION_ID),
        media_name: get(properties::MEDIA_NAME),
        media_role: get(properties::MEDIA_ROLE),
        node_name: get("node.name"),
        volume: volume_to_percent(info.volume.avg()),
        channels: info.volume.len(),
        writable: info.has_volume && info.volume_writable,
        muted: info.mute,
        corked: info.corked,
        sink: info.sink,
    }
}

/// The Linux audio adapter: `PipeWire` through its `PulseAudio` layer.
pub struct PipeWire;

impl crate::audio::AudioSystem for PipeWire {
    fn start(&self, commands: Receiver<Command>, events: Sender<Event>) {
        crate::platform::spawn_thread("audio", move || run(&commands, &events));
    }

    fn outputs(&self) -> Vec<cranked_ipc::AudioOutput> {
        list_outputs()
    }
}

/// Runs forever on its own thread; reconnects when the sound server restarts.
fn run(commands: &Receiver<Command>, events: &Sender<Event>) {
    loop {
        match session(commands, events) {
            Ok(()) => return,
            Err(e) => eprintln!("audio: {e}; reconnecting"),
        }
        let _ = events.send(Event::AudioLost);
        std::thread::sleep(Duration::from_secs(1));
    }
}

fn send_stream(events: &Sender<Event>, result: &ListResult<&SinkInputInfo>) {
    if let ListResult::Item(info) = *result {
        let _ = events.send(Event::Stream(stream_from(info)));
    }
}

/// Ok when the command sender is gone (shutdown), Err when the connection failed or dropped.
fn session(commands: &Receiver<Command>, events: &Sender<Event>) -> Result<(), String> {
    let mut mainloop = Mainloop::new().ok_or_else(|| "no mainloop".to_string())?;
    let mut props = Proplist::new().ok_or_else(|| "no proplist".to_string())?;
    let _ = props.set_str(properties::APPLICATION_NAME, "Cranked Audio");
    let _ = props.set_str(properties::APPLICATION_ID, "cranked-audio");
    let mut context =
        Context::new_with_proplist(&mainloop, "Cranked Audio", &props).ok_or_else(|| "no context".to_string())?;
    context.connect(None, FlagSet::NOFAIL, None).map_err(|e| format!("{e:?}"))?;
    mainloop.start().map_err(|e| format!("{e:?}"))?;

    let ready_by = Instant::now() + Duration::from_secs(10);
    loop {
        mainloop.lock();
        let state = context.get_state();
        mainloop.unlock();
        match state {
            State::Ready => break,
            State::Failed | State::Terminated => return Err(format!("connection {state:?}")),
            _ if Instant::now() > ready_by => return Err("timed out connecting".into()),
            _ => std::thread::sleep(Duration::from_millis(20)),
        }
    }

    let lost = Arc::new(AtomicBool::new(false));
    mainloop.lock();
    {
        // after Ready the only state changes are failures
        let flag = Arc::clone(&lost);
        context.set_state_callback(Some(Box::new(move || flag.store(true, Ordering::Relaxed))));
    }
    let introspect = context.introspect();
    {
        let events = events.clone();
        let lookup = context.introspect();
        context.set_subscribe_callback(Some(Box::new(move |facility, operation, index| {
            if facility != Some(Facility::SinkInput) {
                return;
            }
            match operation {
                Some(Operation::Removed) => {
                    let _ = events.send(Event::StreamRemoved(index));
                }
                Some(_) => {
                    let events = events.clone();
                    lookup.get_sink_input_info(index, move |result| send_stream(&events, &result));
                }
                None => {}
            }
        })));
    }
    context.subscribe(InterestMaskSet::SINK_INPUT, |_| {});
    {
        let events = events.clone();
        introspect.get_sink_input_info_list(move |result| send_stream(&events, &result));
    }
    // the output's volume, so a System knob without a mixer starts where it is
    {
        let events = events.clone();
        introspect.get_sink_info_by_name("@DEFAULT_SINK@", move |result| {
            if let ListResult::Item(sink) = result {
                let percent = round_u8(volume_to_percent(sink.volume.avg()).clamp(0.0, 100.0));
                let _ = events.send(Event::OutputVolume(percent));
            }
        });
    }
    mainloop.unlock();
    let _ = events.send(Event::AudioReady);

    let mut meters: HashMap<u32, Rc<RefCell<PaStream>>> = HashMap::new();
    let mut monitors: HashMap<u32, u32> = HashMap::new();
    let result = loop {
        if lost.load(Ordering::Relaxed) {
            break Err("sound server connection lost".to_string());
        }
        let command = match commands.recv_timeout(Duration::from_secs(1)) {
            Ok(command) => command,
            Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => break Ok(()),
        };
        if let Command::SwitchOutput { enabled, disabled } = command {
            switch_output(&mut mainloop, &mut context, &enabled, &disabled, events);
            continue;
        }
        if let Command::Meter { index, sink, on } = command {
            if let Some(meter) = meters.remove(&index) {
                mainloop.lock();
                stop_meter(meter);
                mainloop.unlock();
            }
            if on {
                let monitor = match monitors.get(&sink) {
                    Some(source) => Some(*source),
                    None => monitor_source(&mut mainloop, &context, sink),
                };
                if let Some(source) = monitor {
                    monitors.insert(sink, source);
                    mainloop.lock();
                    match start_meter(&mut context, index, source, events) {
                        Ok(meter) => {
                            meters.insert(index, meter);
                        }
                        Err(e) => eprintln!("audio: can't watch stream {index}'s level ({e})"),
                    }
                    mainloop.unlock();
                }
            }
            continue;
        }
        mainloop.lock();
        let mut introspect = context.introspect();
        match command {
            Command::SetStream { index, channels, percent } => {
                let mut volumes = ChannelVolumes::default();
                volumes.set(channels.max(1), percent_to_volume(percent));
                introspect.set_sink_input_volume(index, &volumes, None);
            }
            Command::SetMaster { percent } => {
                let mut setter = context.introspect();
                introspect.get_sink_info_by_name("@DEFAULT_SINK@", move |result| {
                    if let ListResult::Item(sink) = result {
                        let mut volumes = ChannelVolumes::default();
                        volumes.set(sink.volume.len().max(1), percent_to_volume(percent));
                        setter.set_sink_volume_by_index(sink.index, &volumes, None);
                    }
                });
            }
            Command::MuteStream { index, mute } => {
                introspect.set_sink_input_mute(index, mute, None);
            }
            Command::MuteMaster { mute } => {
                introspect.set_sink_mute_by_name("@DEFAULT_SINK@", mute, None);
            }
            Command::SwitchOutput { enabled: _, disabled: _ } | Command::Meter { index: _, sink: _, on: _ } => {}
        }
        mainloop.unlock();
    };

    mainloop.lock();
    #[expect(clippy::iter_over_hash_type, reason = "stopping every meter; order doesn't matter")]
    for (_, meter) in meters.drain() {
        stop_meter(meter);
    }
    context.set_state_callback(None);
    context.set_subscribe_callback(None);
    context.disconnect();
    mainloop.unlock();
    mainloop.stop();
    result
}

/// The monitor source of an output device (what it plays, as a recordable source).
fn monitor_source(mainloop: &mut Mainloop, context: &Context, sink: u32) -> Option<u32> {
    let (answer, inbox) = std::sync::mpsc::channel();
    mainloop.lock();
    context.introspect().get_sink_info_by_index(sink, move |result| {
        if let ListResult::Item(info) = result {
            let _ = answer.send(info.monitor_source);
        }
    });
    mainloop.unlock();
    inbox.recv_timeout(Duration::from_secs(2)).ok()
}

/// Peak-level readings of one stream's own output (never a microphone). Call with the mainloop locked.
fn start_meter(
    context: &mut Context,
    index: u32,
    source: u32,
    events: &Sender<Event>,
) -> Result<Rc<RefCell<PaStream>>, String> {
    let spec = Spec { format: Format::FLOAT32NE, channels: 1, rate: METER_RATE };
    let mut props = Proplist::new().ok_or_else(|| "no proplist".to_string())?;
    let _ = props.set_str(properties::MEDIA_NAME, "Cranked Audio level meter");
    let _ = props.set_str("node.name", "cranked-audio-meter");
    let stream = PaStream::new_with_proplist(context, "Cranked Audio level meter", &spec, None, &mut props)
        .ok_or_else(|| "no stream".to_string())?;
    let stream = Rc::new(RefCell::new(stream));
    {
        let mut meter = stream.borrow_mut();
        meter.set_monitor_stream(index).map_err(|e| format!("{e:?}"))?;
        let weak = Rc::downgrade(&stream);
        let events = events.clone();
        let (mut loud_run, mut last_report) = (0_u8, None::<Instant>);
        meter.set_read_callback(Some(Box::new(move |_| {
            let Some(stream) = weak.upgrade() else { return };
            let Ok(mut stream) = stream.try_borrow_mut() else { return };
            let mut peak = 0_f32;
            loop {
                match stream.peek() {
                    Ok(PeekResult::Data(data)) => {
                        for &chunk in data.as_chunks::<4>().0 {
                            peak = peak.max(f32::from_ne_bytes(chunk).abs());
                        }
                        let _ = stream.discard();
                    }
                    Ok(PeekResult::Hole(_)) => {
                        let _ = stream.discard();
                    }
                    _ => break,
                }
            }
            loud_run = if peak >= LOUD { loud_run.saturating_add(1) } else { 0 };
            if loud_run >= LOUD_READINGS && last_report.is_none_or(|t| t.elapsed() >= REPORT_EVERY) {
                last_report = Some(Instant::now());
                let _ = events.send(Event::Loud(index));
            }
        })));
        let attr =
            BufferAttr { maxlength: u32::MAX, tlength: u32::MAX, prebuf: u32::MAX, minreq: u32::MAX, fragsize: 4 };
        let flags = StreamFlags::PEAK_DETECT | StreamFlags::ADJUST_LATENCY | StreamFlags::DONT_MOVE;
        meter.connect_record(Some(&source.to_string()), Some(&attr), flags).map_err(|e| format!("{e:?}"))?;
    }
    Ok(stream)
}

/// Stops and frees a meter stream. Call with the mainloop locked: freeing it unlocked races the mainloop thread
/// (libpulse aborts with a `pa_queue` assertion).
fn stop_meter(meter: Rc<RefCell<PaStream>>) {
    if let Ok(mut stream) = meter.try_borrow_mut() {
        stream.set_read_callback(None);
        let _ = stream.disconnect();
    }
    drop(meter);
}

/// Ask for the current default and all outputs, wait for the answers without holding the lock, then switch.
fn switch_output(
    mainloop: &mut Mainloop,
    context: &mut Context,
    enabled: &[String],
    disabled: &[String],
    events: &Sender<Event>,
) {
    let (answers, inbox) = std::sync::mpsc::channel::<Option<Result<String, Output>>>();
    mainloop.lock();
    let introspect = context.introspect();
    {
        let answers = answers.clone();
        introspect.get_server_info(move |info| {
            let _ = answers.send(Some(Ok(info.default_sink_name.as_deref().unwrap_or_default().to_string())));
        });
    }
    introspect.get_sink_info_list(move |result| {
        let _ = answers.send(match result {
            ListResult::Item(info) => Some(Err(output_from(info))),
            _ => None,
        });
    });
    mainloop.unlock();

    let (mut current, mut outputs, mut list_done) = (None, Vec::new(), false);
    while current.is_none() || !list_done {
        match inbox.recv_timeout(Duration::from_secs(2)) {
            Ok(Some(Ok(name))) => current = Some(name),
            Ok(Some(Err(output))) => outputs.push(output),
            Ok(None) => list_done = true,
            Err(_) => return eprintln!("audio: no answer about outputs"),
        }
    }
    let current = current.unwrap_or_default();
    let Some(next) = next_output(&outputs, &current, enabled, disabled).cloned() else {
        let _ = events.send(Event::OutputSwitched(None));
        return;
    };
    mainloop.lock();
    context.set_default_sink(&next.name, |_| {});
    mainloop.unlock();
    let _ = events.send(Event::OutputSwitched(Some(next.description)));
}

fn list_outputs() -> Vec<cranked_ipc::AudioOutput> {
    let Some(mut mainloop) = Mainloop::new() else { return Vec::new() };
    let Some(mut context) = Context::new(&mainloop, "Cranked Audio outputs") else { return Vec::new() };
    if context.connect(None, FlagSet::NOFAIL, None).is_err() || mainloop.start().is_err() {
        return Vec::new();
    }
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        mainloop.lock();
        let state = context.get_state();
        mainloop.unlock();
        match state {
            State::Ready => break,
            State::Failed | State::Terminated => {
                mainloop.stop();
                return Vec::new();
            }
            _ if Instant::now() > deadline => {
                mainloop.stop();
                return Vec::new();
            }
            _ => std::thread::sleep(Duration::from_millis(10)),
        }
    }
    let (tx, rx) = std::sync::mpsc::channel();
    mainloop.lock();
    context.introspect().get_sink_info_list(move |result| {
        let _ = tx.send(match result {
            ListResult::Item(info) => {
                let out = output_from(info);
                Some(cranked_ipc::AudioOutput { name: out.name, description: out.description, real: out.real })
            }
            _ => None,
        });
    });
    mainloop.unlock();
    let mut outputs = Vec::new();
    while let Ok(Some(out)) = rx.recv_timeout(Duration::from_secs(2)) {
        outputs.push(out);
    }
    mainloop.lock();
    context.disconnect();
    mainloop.unlock();
    mainloop.stop();
    outputs
}
