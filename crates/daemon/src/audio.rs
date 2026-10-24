//! PipeWire through its PulseAudio layer, in-process: stream events in, volume changes out. No helper processes.

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
use pulse::mainloop::threaded::Mainloop;
use pulse::def::BufferAttr;
use pulse::proplist::{properties, Proplist};
use pulse::sample::{Format, Spec};
use pulse::stream::{FlagSet as StreamFlags, PeekResult, Stream as PaStream};
use pulse::volume::{ChannelVolumes, Volume};

use crate::matching::Stream;
use crate::Event;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// Set a stream's volume in percent (all of its channels).
    SetStream { index: u32, channels: u8, percent: u8 },
    /// Set the default output's volume in percent.
    SetMaster { percent: u8 },
    MuteStream { index: u32, mute: bool },
    MuteMaster { mute: bool },
    /// Make the next real output device the default (apps follow); answers with `Event::OutputSwitched`.
    SwitchOutput,
    /// Start or stop watching how loud a stream plays (it reports `Event::Loud` while it is).
    Meter { index: u32, sink: u32, on: bool },
}

/// Level readings per second for each watched stream.
const METER_RATE: u32 = 25;
/// About -44 dBFS: quieter than speech or music, louder than silence and line noise.
const LOUD: f32 = 0.0063;
/// Readings in a row above `LOUD` before it counts (two readings = 80 ms).
const LOUD_READINGS: u8 = 2;
/// While loud, report at most this often.
const REPORT_EVERY: Duration = Duration::from_millis(200);

#[derive(Debug, Clone, PartialEq)]
pub struct Output {
    pub name: String,
    pub description: String,
    /// A real device (sound card, Bluetooth, USB), not a virtual sink.
    pub real: bool,
}

fn output_from(info: &SinkInfo) -> Output {
    let name = info.name.as_deref().unwrap_or_default().to_string();
    let description = info.description.as_deref().unwrap_or(&name).to_string();
    let virtual_marker = name.contains("nvbroadcast") || description.to_lowercase().contains("nvidia broadcast");
    let real = info.proplist.get_str("device.api").is_some() && !virtual_marker;
    Output { name, description, real }
}

/// The real output after `current` (wrapping), or None when there's nothing else to switch to.
pub fn next_output<'a>(outputs: &'a [Output], current: &str) -> Option<&'a Output> {
    let real: Vec<&Output> = outputs.iter().filter(|o| o.real).collect();
    let at = real.iter().position(|o| o.name == current);
    let next = match at {
        Some(i) => real.get((i + 1) % real.len().max(1)).copied(),
        None => real.first().copied(),
    }?;
    (next.name != current).then_some(next)
}

fn percent_to_volume(percent: u8) -> Volume {
    Volume((u64::from(Volume::NORMAL.0) * u64::from(percent) / 100) as u32)
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

/// Runs forever on its own thread; reconnects when the sound server restarts.
pub fn run(commands: Receiver<Command>, events: Sender<Event>) {
    loop {
        match session(&commands, &events) {
            Ok(()) => return,
            Err(e) => eprintln!("audio: {e}; reconnecting"),
        }
        let _ = events.send(Event::AudioLost);
        std::thread::sleep(Duration::from_secs(1));
    }
}

fn send_stream(events: &Sender<Event>, result: ListResult<&SinkInputInfo>) {
    if let ListResult::Item(info) = result {
        let _ = events.send(Event::Stream(stream_from(info)));
    }
}

/// Ok when the command sender is gone (shutdown), Err when the connection failed or dropped.
fn session(commands: &Receiver<Command>, events: &Sender<Event>) -> Result<(), String> {
    let mut mainloop = Mainloop::new().ok_or_else(|| "no mainloop".to_string())?;
    let mut props = Proplist::new().ok_or_else(|| "no proplist".to_string())?;
    let _ = props.set_str(properties::APPLICATION_NAME, "Cranked Audio");
    let _ = props.set_str(properties::APPLICATION_ID, "cranked-audio");
    let mut context = Context::new_with_proplist(&mainloop, "Cranked Audio", &props).ok_or_else(|| "no context".to_string())?;
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
                    lookup.get_sink_input_info(index, move |result| send_stream(&events, result));
                }
                None => {}
            }
        })));
    }
    context.subscribe(InterestMaskSet::SINK_INPUT, |_| {});
    {
        let events = events.clone();
        introspect.get_sink_input_info_list(move |result| send_stream(&events, result));
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
        if let Command::SwitchOutput = command {
            switch_output(&mut mainloop, &mut context, events);
            continue;
        }
        if let Command::Meter { index, sink, on } = command {
            if let Some(meter) = meters.remove(&index) {
                mainloop.lock();
                stop_meter(&meter);
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
            Command::SwitchOutput | Command::Meter { .. } => {}
        }
        mainloop.unlock();
    };

    mainloop.lock();
    for meter in meters.values() {
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
fn start_meter(context: &mut Context, index: u32, source: u32, events: &Sender<Event>) -> Result<Rc<RefCell<PaStream>>, String> {
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
        let (mut loud_run, mut last_report) = (0u8, None::<Instant>);
        meter.set_read_callback(Some(Box::new(move |_| {
            let Some(stream) = weak.upgrade() else { return };
            let Ok(mut stream) = stream.try_borrow_mut() else { return };
            let mut peak = 0f32;
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
        let attr = BufferAttr { maxlength: u32::MAX, tlength: u32::MAX, prebuf: u32::MAX, minreq: u32::MAX, fragsize: 4 };
        let flags = StreamFlags::PEAK_DETECT | StreamFlags::ADJUST_LATENCY | StreamFlags::DONT_MOVE;
        meter.connect_record(Some(&source.to_string()), Some(&attr), flags).map_err(|e| format!("{e:?}"))?;
    }
    Ok(stream)
}

fn stop_meter(meter: &Rc<RefCell<PaStream>>) {
    if let Ok(mut meter) = meter.try_borrow_mut() {
        meter.set_read_callback(None);
        let _ = meter.disconnect();
    }
}

/// Ask for the current default and all outputs, wait for the answers without holding the lock, then switch.
fn switch_output(mainloop: &mut Mainloop, context: &mut Context, events: &Sender<Event>) {
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
    let Some(next) = next_output(&outputs, &current).cloned() else {
        let _ = events.send(Event::OutputSwitched(None));
        return;
    };
    mainloop.lock();
    context.set_default_sink(&next.name, |_| {});
    mainloop.unlock();
    let _ = events.send(Event::OutputSwitched(Some(next.description)));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn output(name: &str, real: bool) -> Output {
        Output { name: name.into(), description: name.to_uppercase(), real }
    }

    #[test]
    fn switching_cycles_real_outputs_only() {
        let outputs = [output("dac", true), output("nvbroadcast_sink", false), output("hdmi", true)];
        assert_eq!(next_output(&outputs, "dac").map(|o| o.name.as_str()), Some("hdmi"));
        assert_eq!(next_output(&outputs, "hdmi").map(|o| o.name.as_str()), Some("dac"));
        assert_eq!(next_output(&outputs, "nvbroadcast_sink").map(|o| o.name.as_str()), Some("dac"));
        assert_eq!(next_output(&outputs[..1], "dac"), None);
    }
}
