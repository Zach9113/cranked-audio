//! Windows: WASAPI audio sessions, the per-app entries of Windows' own volume mixer. Every active output's
//! sessions are looked at a few times a second and changes are reported as the same stream events Linux sends.
//! A session's volume is the percentage the Windows mixer shows; the System knob moves the default output.

use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

use windows::core::{Interface, GUID, PCWSTR, PWSTR};
use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
use windows::Win32::Foundation::{CloseHandle, PROPERTYKEY, S_OK};
use windows::Win32::Media::Audio::Endpoints::{IAudioEndpointVolume, IAudioMeterInformation};
use windows::Win32::Media::Audio::{
    eCommunications, eConsole, eMultimedia, eRender, AudioSessionStateActive, AudioSessionStateExpired,
    IAudioSessionControl2, IAudioSessionManager2, IMMDevice, IMMDeviceEnumerator, ISimpleAudioVolume,
    MMDeviceEnumerator, DEVICE_STATE_ACTIVE,
};
use windows::Win32::Storage::FileSystem::{GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_ALL, COINIT_MULTITHREADED, STGM_READ,
};
use windows::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};

use super::{next_output, Command, Output, LOUD, LOUD_READINGS, METER_RATE, REPORT_EVERY};
use crate::matching::Stream;
use crate::Event;

/// How often the sessions are looked at (new apps, apps gone, volume changed elsewhere).
const SCAN_EVERY: Duration = Duration::from_millis(300);

/// Runs forever on its own thread; starts over when Windows audio goes away (the audio service restarted).
pub fn run(commands: Receiver<Command>, events: Sender<Event>) {
    // SAFETY: this thread's COM apartment, undone below when the session ends
    if let Err(e) = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.ok() {
        return eprintln!("audio: COM didn't start ({e})");
    }
    loop {
        match session(&commands, &events) {
            Ok(()) => break,
            Err(e) => eprintln!("audio: {e}; reconnecting"),
        }
        let _ = events.send(Event::AudioLost);
        std::thread::sleep(Duration::from_secs(1));
    }
    // SAFETY: matches the CoInitializeEx above; every COM object of this thread is gone by now
    unsafe { CoUninitialize() };
}

/// One app's session as last seen.
struct Session {
    pid: u32,
    volume: ISimpleAudioVolume,
    meter: IAudioMeterInformation,
    stream: Stream,
}

/// Auto-lower's level watch on one session.
#[derive(Default)]
struct Meter {
    loud_run: u8,
    last_report: Option<Instant>,
}

#[derive(Default)]
struct Sessions {
    /// Our stream numbers by session instance id (stable while the session lives).
    numbers: HashMap<String, u32>,
    next_number: u32,
    /// Output numbers by device id (the stream's `sink`), stable for the run.
    outputs: HashMap<String, u32>,
    live: HashMap<u32, Session>,
    /// Program names and descriptions by process id.
    programs: HashMap<u32, (String, String)>,
}

fn com_string(text: PWSTR) -> String {
    if text.is_null() {
        return String::new();
    }
    // SAFETY: COM handed us a NUL-terminated string we own; read it, then free it once
    let string = unsafe { text.to_string() }.unwrap_or_default();
    unsafe { CoTaskMemFree(Some(text.0.cast())) };
    string
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

/// A program's file name ("Spotify.exe") and its description ("Spotify"), by process id.
fn program(pid: u32) -> Option<(String, String)> {
    // SAFETY: a query-only handle, closed below
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }.ok()?;
    let mut buffer = [0u16; 1024];
    let mut length = buffer.len() as u32;
    // SAFETY: the buffer and its length in characters
    let named =
        unsafe { QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, PWSTR(buffer.as_mut_ptr()), &mut length) };
    // SAFETY: the handle opened above
    let _ = unsafe { CloseHandle(process) };
    named.ok()?;
    let path = String::from_utf16_lossy(&buffer[..length as usize]);
    let file = path.rsplit('\\').next().unwrap_or(&path).to_string();
    let description = file_description(&path).unwrap_or_else(|| file.trim_end_matches(".exe").to_string());
    Some((file, description))
}

/// The "File description" in a program's version info, which is the name Windows shows for it.
fn file_description(path: &str) -> Option<String> {
    let path = wide(path);
    // SAFETY: a NUL-terminated path
    let size = unsafe { GetFileVersionInfoSizeW(PCWSTR(path.as_ptr()), None) };
    if size == 0 {
        return None;
    }
    let mut data = vec![0u8; size as usize];
    // SAFETY: `data` is `size` bytes long
    unsafe { GetFileVersionInfoW(PCWSTR(path.as_ptr()), None, size, data.as_mut_ptr().cast()) }.ok()?;
    let query = |key: &str| -> Option<&[u8]> {
        let key = wide(key);
        let mut value = std::ptr::null_mut();
        let mut length = 0u32;
        // SAFETY: VerQueryValueW points `value` into `data`, which outlives the slice made from it
        let found =
            unsafe { VerQueryValueW(data.as_ptr().cast(), PCWSTR(key.as_ptr()), &mut value, &mut length).as_bool() };
        (found && !value.is_null()).then(|| unsafe { std::slice::from_raw_parts(value.cast::<u8>(), length as usize) })
    };
    // language and code page of the strings, then the description in UTF-16 (length in characters)
    let translation = query("\\VarFileInfo\\Translation")?;
    let (language, code_page) = (
        u16::from_le_bytes([*translation.first()?, *translation.get(1)?]),
        u16::from_le_bytes([*translation.get(2)?, *translation.get(3)?]),
    );
    let key = format!("\\StringFileInfo\\{language:04x}{code_page:04x}\\FileDescription");
    let key = wide(&key);
    let mut value = std::ptr::null_mut();
    let mut length = 0u32;
    // SAFETY: as above; the value is `length` UTF-16 characters including the NUL
    let found = unsafe { VerQueryValueW(data.as_ptr().cast(), PCWSTR(key.as_ptr()), &mut value, &mut length) };
    if !found.as_bool() || value.is_null() || length == 0 {
        return None;
    }
    let text = unsafe { std::slice::from_raw_parts(value.cast::<u16>(), length as usize) };
    let text = text.split(|&c| c == 0).next().unwrap_or(text);
    let text = String::from_utf16_lossy(text).trim().to_string();
    (!text.is_empty()).then_some(text)
}

fn friendly_name(device: &IMMDevice) -> Option<String> {
    // SAFETY: plain COM calls on a live device
    unsafe {
        let store = device.OpenPropertyStore(STGM_READ).ok()?;
        let value = store.GetValue(&PKEY_Device_FriendlyName).ok()?;
        Some(value.to_string())
    }
}

/// The output's parent device, e.g. `USB\VID_...`, `HDAUDIO\FUNC_01...` or `{1}.ROOT\STEAMSTREAMINGSPEAKERS\0000`
/// (not in the SDK headers; the same key the Sound control panel shows as the device instance).
const PKEY_PARENT_DEVICE: PROPERTYKEY =
    PROPERTYKEY { fmtid: GUID::from_u128(0xb3f8fa53_0004_438e_9003_51a46e139bfc), pid: 2 };

fn parent_device(device: &IMMDevice) -> Option<String> {
    // SAFETY: plain COM calls on a live device
    unsafe {
        let store = device.OpenPropertyStore(STGM_READ).ok()?;
        let value = store.GetValue(&PKEY_PARENT_DEVICE).ok()?;
        Some(value.to_string())
    }
}

/// Real outputs sit on a bus (USB, HD Audio, PCI, Bluetooth); software-only ones (Steam Streaming Speakers, virtual
/// cables) are root-enumerated (`ROOT\...` or `{1}.ROOT\...`). Switching skips those by default, like the Linux side
/// skips its virtual sinks.
pub fn is_virtual_output(parent: &str) -> bool {
    let instance = parent.split_once("}.").map_or(parent, |(_, rest)| rest);
    instance.get(..5).is_some_and(|prefix| prefix.eq_ignore_ascii_case("ROOT\\"))
}

pub fn list_outputs() -> Vec<cranked_ipc::AudioOutput> {
    // SAFETY: plain COM calls on the calling thread
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let Ok(enumerator) = CoCreateInstance::<_, IMMDeviceEnumerator>(&MMDeviceEnumerator, None, CLSCTX_ALL) else {
            return Vec::new();
        };
        let Ok(devices) = active_outputs(&enumerator) else { return Vec::new() };
        devices
            .iter()
            .map(|(id, device)| cranked_ipc::AudioOutput {
                name: id.clone(),
                description: friendly_name(device).unwrap_or_else(|| id.clone()),
                real: !parent_device(device).is_some_and(|parent| is_virtual_output(&parent)),
            })
            .collect()
    }
}

fn active_outputs(enumerator: &IMMDeviceEnumerator) -> windows::core::Result<Vec<(String, IMMDevice)>> {
    let mut outputs = Vec::new();
    // SAFETY: plain COM calls; ids are freed by com_string
    unsafe {
        let devices = enumerator.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE)?;
        for i in 0..devices.GetCount()? {
            let device = devices.Item(i)?;
            outputs.push((com_string(device.GetId()?), device));
        }
    }
    Ok(outputs)
}

fn default_output(enumerator: &IMMDeviceEnumerator) -> windows::core::Result<IMMDevice> {
    // SAFETY: a plain COM call
    unsafe { enumerator.GetDefaultAudioEndpoint(eRender, eConsole) }
}

fn endpoint_volume(enumerator: &IMMDeviceEnumerator) -> windows::core::Result<IAudioEndpointVolume> {
    // SAFETY: a plain COM call on a live device
    unsafe { default_output(enumerator)?.Activate(CLSCTX_ALL, None) }
}

impl Sessions {
    fn number(&mut self, instance: &str) -> u32 {
        if let Some(number) = self.numbers.get(instance) {
            return *number;
        }
        self.next_number += 1;
        self.numbers.insert(instance.to_string(), self.next_number);
        self.next_number
    }

    fn output_number(&mut self, id: &str) -> u32 {
        let next = self.outputs.len() as u32 + 1;
        *self.outputs.entry(id.to_string()).or_insert(next)
    }

    /// Looks at every session of every active output; reports new, changed and gone ones.
    fn scan(&mut self, enumerator: &IMMDeviceEnumerator, events: &Sender<Event>) -> windows::core::Result<()> {
        let mut seen = HashSet::new();
        for (id, device) in active_outputs(enumerator)? {
            let sink = self.output_number(&id);
            // SAFETY: a plain COM call on a live device
            let Ok(manager) = (unsafe { device.Activate::<IAudioSessionManager2>(CLSCTX_ALL, None) }) else { continue };
            // SAFETY: plain COM calls
            let Ok(list) = (unsafe { manager.GetSessionEnumerator() }) else { continue };
            for i in 0..unsafe { list.GetCount() }.unwrap_or(0) {
                let Ok(control) = (unsafe { list.GetSession(i) }) else { continue };
                let Ok(control) = control.cast::<IAudioSessionControl2>() else { continue };
                if let Some(number) = self.look_at(&control, sink, events) {
                    seen.insert(number);
                }
            }
        }
        let gone: Vec<u32> = self.live.keys().filter(|n| !seen.contains(n)).copied().collect();
        for number in gone {
            self.live.remove(&number);
            self.numbers.retain(|_, n| *n != number);
            if events.send(Event::StreamRemoved(number)).is_err() {
                break;
            }
        }
        let pids: HashSet<u32> = self.live.values().map(|s| s.pid).collect();
        self.programs.retain(|pid, _| pids.contains(pid));
        Ok(())
    }

    /// One session: its stream number when it's alive, after reporting it if it's new or changed.
    fn look_at(&mut self, control: &IAudioSessionControl2, sink: u32, events: &Sender<Event>) -> Option<u32> {
        // SAFETY: plain COM calls on a live session
        let (state, instance, pid, system, display) = unsafe {
            (
                control.GetState().ok()?,
                com_string(control.GetSessionInstanceIdentifier().ok()?),
                control.GetProcessId().unwrap_or(0),
                control.IsSystemSoundsSession() == S_OK,
                control.GetDisplayName().map(com_string).unwrap_or_default(),
            )
        };
        if state == AudioSessionStateExpired {
            return None;
        }
        // a resource reference ("@%SystemRoot%\...") isn't a name
        let display = if display.starts_with('@') { String::new() } else { display };
        let (binary, app_name) = if system {
            (String::new(), "System sounds".to_string())
        } else if let Some((binary, description)) = self.programs.get(&pid).cloned().or_else(|| program(pid)) {
            self.programs.insert(pid, (binary.clone(), description.clone()));
            (binary, description)
        } else if !display.is_empty() {
            // a program Windows won't tell us about (a protected process): its session's own name
            (String::new(), display.clone())
        } else {
            return None;
        };
        let number = self.number(&instance);
        let session = match self.live.remove(&number) {
            Some(session) => session,
            None => {
                Session { pid, volume: control.cast().ok()?, meter: control.cast().ok()?, stream: Stream::default() }
            }
        };
        // SAFETY: plain COM calls on a live session
        let (volume, muted) = unsafe {
            (session.volume.GetMasterVolume().unwrap_or(1.0), session.volume.GetMute().is_ok_and(|m| m.as_bool()))
        };
        let stream = Stream {
            index: number,
            app_name,
            binary,
            app_id: String::new(),
            media_name: display,
            // system sounds count as event sounds: never taken by a catch-all knob
            media_role: if system { "event".into() } else { String::new() },
            node_name: String::new(),
            volume: f64::from(volume) * 100.0,
            channels: 1,
            writable: true,
            muted,
            corked: state != AudioSessionStateActive,
            sink,
        };
        if stream != session.stream {
            let _ = events.send(Event::Stream(stream.clone()));
        }
        self.live.insert(number, Session { stream, ..session });
        Some(number)
    }
}

/// Ok when the command sender is gone (shutdown), Err when Windows audio can't be reached.
fn session(commands: &Receiver<Command>, events: &Sender<Event>) -> Result<(), String> {
    // SAFETY: a plain COM call
    let enumerator: IMMDeviceEnumerator =
        unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) }.map_err(|e| e.to_string())?;
    let mut sessions = Sessions::default();
    sessions.scan(&enumerator, events).map_err(|e| e.to_string())?;
    // the output's volume, so a System knob without a mixer starts where it is
    if let Ok(endpoint) = endpoint_volume(&enumerator) {
        // SAFETY: a plain COM call
        if let Ok(level) = unsafe { endpoint.GetMasterVolumeLevelScalar() } {
            let _ = events.send(Event::OutputVolume((level * 100.0).round().clamp(0.0, 100.0) as u8));
        }
    }
    let _ = events.send(Event::AudioReady);

    let mut meters: HashMap<u32, Meter> = HashMap::new();
    let mut last_scan = Instant::now();
    let meter_every = Duration::from_millis(1000 / u64::from(METER_RATE));
    loop {
        let wait = if meters.is_empty() { SCAN_EVERY } else { meter_every };
        match commands.recv_timeout(wait) {
            Ok(command) => run_command(command, &enumerator, &mut sessions, &mut meters, events),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return Ok(()),
        }
        if last_scan.elapsed() >= SCAN_EVERY {
            last_scan = Instant::now();
            sessions.scan(&enumerator, events).map_err(|e| format!("can't list sessions ({e})"))?;
            meters.retain(|number, _| sessions.live.contains_key(number));
        }
        read_meters(&sessions, &mut meters, events);
    }
}

/// Peak readings of the watched sessions; reports `Event::Loud` the way the Linux meter does.
fn read_meters(sessions: &Sessions, meters: &mut HashMap<u32, Meter>, events: &Sender<Event>) {
    for (number, meter) in meters.iter_mut() {
        let Some(session) = sessions.live.get(number) else { continue };
        // SAFETY: a plain COM call on a live session
        let peak = unsafe { session.meter.GetPeakValue() }.unwrap_or(0.0);
        meter.loud_run = if peak >= LOUD { meter.loud_run.saturating_add(1) } else { 0 };
        if meter.loud_run >= LOUD_READINGS && meter.last_report.is_none_or(|t| t.elapsed() >= REPORT_EVERY) {
            meter.last_report = Some(Instant::now());
            let _ = events.send(Event::Loud(*number));
        }
    }
}

fn run_command(
    command: Command,
    enumerator: &IMMDeviceEnumerator,
    sessions: &mut Sessions,
    meters: &mut HashMap<u32, Meter>,
    events: &Sender<Event>,
) {
    let no_context: *const GUID = std::ptr::null();
    // SAFETY (all arms): plain COM calls on live objects
    // what we set is remembered as the session's state, so the next scan doesn't report our own change back as one
    // from the app (the guard then "put back" volumes the knob had already moved past)
    let result = match command {
        Command::SetStream { index, percent, .. } => match sessions.live.get_mut(&index) {
            Some(session) => unsafe { session.volume.SetMasterVolume(f32::from(percent) / 100.0, no_context) }
                .inspect(|()| session.stream.volume = f64::from(f32::from(percent) / 100.0) * 100.0),
            None => Ok(()),
        },
        Command::MuteStream { index, mute } => match sessions.live.get_mut(&index) {
            Some(session) => {
                unsafe { session.volume.SetMute(mute, no_context) }.inspect(|()| session.stream.muted = mute)
            }
            None => Ok(()),
        },
        Command::SetMaster { percent } => endpoint_volume(enumerator)
            .and_then(|e| unsafe { e.SetMasterVolumeLevelScalar(f32::from(percent) / 100.0, no_context) }),
        Command::MuteMaster { mute } => {
            endpoint_volume(enumerator).and_then(|e| unsafe { e.SetMute(mute, no_context) })
        }
        Command::SwitchOutput { enabled, disabled } => {
            switch_output(enumerator, &enabled, &disabled, events);
            Ok(())
        }
        Command::Meter { index, on, .. } => {
            if on {
                meters.entry(index).or_default();
            } else {
                meters.remove(&index);
            }
            Ok(())
        }
    };
    if let Err(e) = result {
        eprintln!("audio: {e}");
    }
}

#[allow(non_snake_case, reason = "COM method names")]
mod policy_config {
    use windows::core::PCWSTR;
    use windows::Win32::Media::Audio::ERole;

    /// Windows' own way to pick the default output; not in the public SDK (the volume mixers that switch outputs,
    /// like EarTrumpet and SoundSwitch, use it the same way). Only the methods up to SetDefaultEndpoint are called.
    #[windows_core::interface("f8679f50-850a-41cf-9c72-430f290290c8")]
    pub unsafe trait IPolicyConfig: windows::core::IUnknown {
        fn GetMixFormat(&self, device: PCWSTR, format: *mut *mut std::ffi::c_void) -> windows::core::HRESULT;
        fn GetDeviceFormat(
            &self,
            device: PCWSTR,
            default: i32,
            format: *mut *mut std::ffi::c_void,
        ) -> windows::core::HRESULT;
        fn ResetDeviceFormat(&self, device: PCWSTR) -> windows::core::HRESULT;
        fn SetDeviceFormat(
            &self,
            device: PCWSTR,
            endpoint: *mut std::ffi::c_void,
            mix: *mut std::ffi::c_void,
        ) -> windows::core::HRESULT;
        fn GetProcessingPeriod(&self, device: PCWSTR, default: i32, a: *mut i64, b: *mut i64)
            -> windows::core::HRESULT;
        fn SetProcessingPeriod(&self, device: PCWSTR, period: *mut i64) -> windows::core::HRESULT;
        fn GetShareMode(&self, device: PCWSTR, mode: *mut std::ffi::c_void) -> windows::core::HRESULT;
        fn SetShareMode(&self, device: PCWSTR, mode: *mut std::ffi::c_void) -> windows::core::HRESULT;
        fn GetPropertyValue(
            &self,
            device: PCWSTR,
            key: *const std::ffi::c_void,
            value: *mut std::ffi::c_void,
        ) -> windows::core::HRESULT;
        fn SetPropertyValue(
            &self,
            device: PCWSTR,
            key: *const std::ffi::c_void,
            value: *mut std::ffi::c_void,
        ) -> windows::core::HRESULT;
        pub fn SetDefaultEndpoint(&self, device: PCWSTR, role: ERole) -> windows::core::HRESULT;
    }
}
use policy_config::IPolicyConfig;

const POLICY_CONFIG_CLIENT: GUID = GUID::from_u128(0x870af99c_171d_4f9e_af0d_e63df40c2bc9);

fn set_default_output(id: &str) -> windows::core::Result<()> {
    let id = wide(id);
    // SAFETY: COM calls with a NUL-terminated device id that outlives them
    unsafe {
        let policy: IPolicyConfig = CoCreateInstance(&POLICY_CONFIG_CLIENT, None, CLSCTX_ALL)?;
        for role in [eConsole, eMultimedia, eCommunications] {
            policy.SetDefaultEndpoint(PCWSTR(id.as_ptr()), role).ok()?;
        }
    }
    Ok(())
}

fn switch_output(enumerator: &IMMDeviceEnumerator, enabled: &[String], disabled: &[String], events: &Sender<Event>) {
    let current = default_output(enumerator).ok().and_then(|d| unsafe { d.GetId() }.ok()).map(com_string);
    let Ok(devices) = active_outputs(enumerator) else { return eprintln!("audio: no answer about outputs") };
    let outputs: Vec<Output> = devices
        .iter()
        .map(|(id, device)| Output {
            name: id.clone(),
            description: friendly_name(device).unwrap_or_else(|| id.clone()),
            real: !parent_device(device).is_some_and(|parent| is_virtual_output(&parent)),
        })
        .collect();
    let Some(next) = next_output(&outputs, current.as_deref().unwrap_or_default(), enabled, disabled).cloned() else {
        let _ = events.send(Event::OutputSwitched(None));
        return;
    };
    match set_default_output(&next.name) {
        Ok(()) => {
            let _ = events.send(Event::OutputSwitched(Some(next.description)));
        }
        Err(e) => eprintln!("audio: can't switch the output ({e})"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_enumerated_outputs_are_virtual() {
        assert!(is_virtual_output(r"ROOT\STEAMSTREAMINGSPEAKERS\0000"));
        assert!(is_virtual_output(r"{1}.ROOT\STEAMSTREAMINGSPEAKERS\0000"));
        assert!(is_virtual_output(r"root\VBAudioVACWDM\0000"));
        assert!(!is_virtual_output(r"USB\VID_30BE&PID_100F&MI_00\7&2D23BB74&0&0000"));
        assert!(!is_virtual_output(r"{1}.USB\VID_30BE&PID_100F&MI_00\7&2D23BB74&0&0000"));
        assert!(!is_virtual_output(r"HDAUDIO\FUNC_01&VEN_10DE&DEV_00A3\5&1DD26AFF&0&0001"));
        assert!(!is_virtual_output(r"{1}.HDAUDIO\FUNC_01&VEN_10DE&DEV_00A3\5&1DD26AFF&0&0001"));
        assert!(!is_virtual_output(""));
    }
}
