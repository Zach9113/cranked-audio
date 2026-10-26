# Live check against Windows audio (the Windows counterpart of tests/live.sh): two throwaway silent players,
# nothing else touched, no mixer opened. Run it in a desktop session (audio sessions are per logon session, so not
# over SSH directly; a scheduled task with an interactive logon works).
# Usage: powershell -ExecutionPolicy Bypass -File tests\live_windows.ps1 [path\to\cranked-audiod.exe]
param([string]$Bin = "$PSScriptRoot\..\target\x86_64-pc-windows-msvc\release\cranked-audiod.exe")
$ErrorActionPreference = 'Stop'
$Bin = (Resolve-Path $Bin).Path
$work = Join-Path $env:TEMP "cranked-live-$PID"
New-Item -ItemType Directory -Force $work | Out-Null
$env:CRANKED_SOCKET = "$work\socket"; $env:XDG_STATE_HOME = "$work\state"
@'
popups = false

[[channel]]
name = "A"
apps = ["cranked-selftest-a"]
press = "play_pause"
hold = "mute"
[[channel]]
name = "B"
apps = ["cranked-selftest-b"]
min = 10
max = 60
'@ | Set-Content "$work\config.toml"

# Reads and changes apps' volumes the way Windows' own mixer does (WASAPI session volumes).
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class Mixer {
    [ComImport, Guid("BCDE0395-E52F-467C-8E3D-C4579291692E")] class MMDeviceEnumerator {}
    [Guid("A95664D2-9614-4F35-A746-DE8DB63617E6"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    interface IMMDeviceEnumerator { int EnumAudioEndpoints(); [PreserveSig] int GetDefaultAudioEndpoint(int flow, int role, out IMMDevice device); }
    [Guid("D666063F-1587-4E43-81F1-B948E807363F"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    interface IMMDevice { [PreserveSig] int Activate(ref Guid iid, int context, IntPtr parameters, [MarshalAs(UnmanagedType.IUnknown)] out object result); }
    [Guid("77AA99A0-1BD6-484F-8BC7-2C654C9A9B6F"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    interface IAudioSessionManager2 { int GetAudioSessionControl(); int GetSimpleAudioVolume(); [PreserveSig] int GetSessionEnumerator(out IAudioSessionEnumerator list); }
    [Guid("E2F5BB11-0570-40CA-ACDD-3AA01277DEE8"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    interface IAudioSessionEnumerator { [PreserveSig] int GetCount(out int count); [PreserveSig] int GetSession(int i, out IAudioSessionControl2 session); }
    [Guid("bfb7ff88-7239-4fc9-8fa2-07c950be9c6d"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    interface IAudioSessionControl2 {
        int GetState(); int GetDisplayName(); int SetDisplayName(); int GetIconPath(); int SetIconPath(); int GetGroupingParam();
        int SetGroupingParam(); int RegisterAudioSessionNotification(); int UnregisterAudioSessionNotification();
        int GetSessionIdentifier(); int GetSessionInstanceIdentifier(); [PreserveSig] int GetProcessId(out uint pid);
    }
    [Guid("87CE5498-68D6-44E5-9215-6DA47EF883D8"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    interface ISimpleAudioVolume {
        [PreserveSig] int SetMasterVolume(float level, ref Guid context); [PreserveSig] int GetMasterVolume(out float level);
        [PreserveSig] int SetMute(bool mute, ref Guid context); [PreserveSig] int GetMute(out bool mute);
    }
    [Guid("5CDF2C82-841E-4546-9722-0CF74078229A"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    interface IAudioEndpointVolume {
        int RegisterControlChangeNotify(); int UnregisterControlChangeNotify(); int GetChannelCount(); int SetMasterVolumeLevel();
        [PreserveSig] int SetMasterVolumeLevelScalar(float level, ref Guid context); int GetMasterVolumeLevel();
        [PreserveSig] int GetMasterVolumeLevelScalar(out float level);
    }
    static IMMDevice Output() {
        IMMDevice device;
        ((IMMDeviceEnumerator)new MMDeviceEnumerator()).GetDefaultAudioEndpoint(0, 0, out device);
        return device;
    }
    static ISimpleAudioVolume[] Sessions(uint pid) {
        Guid iid = typeof(IAudioSessionManager2).GUID; object manager;
        Output().Activate(ref iid, 23, IntPtr.Zero, out manager);
        IAudioSessionEnumerator list; ((IAudioSessionManager2)manager).GetSessionEnumerator(out list);
        int count; list.GetCount(out count);
        var found = new System.Collections.Generic.List<ISimpleAudioVolume>();
        for (int i = 0; i < count; i++) {
            IAudioSessionControl2 session; list.GetSession(i, out session);
            uint owner; session.GetProcessId(out owner);
            if (owner == pid) found.Add((ISimpleAudioVolume)session);
        }
        return found.ToArray();
    }
    public static string Volume(uint pid) {
        var text = "";
        foreach (var s in Sessions(pid)) { float level; s.GetMasterVolume(out level); text += Math.Round(level * 100) + " "; }
        return text;
    }
    public static string Muted(uint pid) {
        var text = "";
        foreach (var s in Sessions(pid)) { bool mute; s.GetMute(out mute); text += (mute ? "true" : "false") + " "; }
        return text;
    }
    public static void SetVolume(uint pid, float level) {
        Guid none = Guid.Empty;
        foreach (var s in Sessions(pid)) s.SetMasterVolume(level, ref none);
    }
    public static int OutputVolume() {
        Guid iid = typeof(IAudioEndpointVolume).GUID; object endpoint; float level;
        Output().Activate(ref iid, 23, IntPtr.Zero, out endpoint);
        ((IAudioEndpointVolume)endpoint).GetMasterVolumeLevelScalar(out level);
        return (int)Math.Round(level * 100);
    }
}
'@

# A tiny player that loops a second of silence: the selftest apps are copies of it under their own names.
$player = @'
using System.IO; using System.Media;
public static class Player {
    public static void Main() {
        var wav = new MemoryStream(); var w = new BinaryWriter(wav); int rate = 48000, bytes = rate * 2;
        w.Write("RIFF".ToCharArray()); w.Write(36 + bytes); w.Write("WAVEfmt ".ToCharArray()); w.Write(16); w.Write((short)1);
        w.Write((short)1); w.Write(rate); w.Write(rate * 2); w.Write((short)2); w.Write((short)16);
        w.Write("data".ToCharArray()); w.Write(bytes); w.Write(new byte[bytes]); wav.Position = 0;
        new SoundPlayer(wav).PlayLooping(); System.Threading.Thread.Sleep(System.Threading.Timeout.Infinite);
    }
}
'@
Add-Type -TypeDefinition $player -OutputAssembly "$work\cranked-selftest-a.exe" -OutputType ConsoleApplication
Copy-Item "$work\cranked-selftest-a.exe" "$work\cranked-selftest-b.exe"
$players = New-Object System.Collections.ArrayList
function Play($name) {
    $p = Start-Process "$work\$name.exe" -WindowStyle Hidden -PassThru
    [void]$players.Add($p); return [uint32]$p.Id
}

$fails = 0
function Check($what, $got, $want) {
    if ("$got" -eq "$want") { "ok   $what ($got)" } else { "FAIL ${what}: got '$got', want '$want'"; $script:fails++ }
}

$a = Play cranked-selftest-a; $b = Play cranked-selftest-b
Start-Sleep -Milliseconds 800
$outputBefore = [Mixer]::OutputVolume()
# cmd passes our pipe on as the service's stdin and writes its log to a file
$info = New-Object System.Diagnostics.ProcessStartInfo 'cmd.exe', "/c `"`"$Bin`" --config `"$work\config.toml`" --no-mixer --stdin -v 2> `"$work\log`"`""
$info.UseShellExecute = $false; $info.RedirectStandardInput = $true; $info.CreateNoWindow = $true
$daemon = [System.Diagnostics.Process]::Start($info)
function Send($line) { $daemon.StandardInput.WriteLine($line); $daemon.StandardInput.Flush() }
Start-Sleep -Milliseconds 1500

Send 'knob 0 30'; Send 'knob 1 50'; Start-Sleep -Milliseconds 700
Check 'knob A to 30' ([Mixer]::Volume($a)) '30 '
Check 'knob B 50 maps onto 10-60' ([Mixer]::Volume($b)) '35 '
[Mixer]::SetVolume($a, 0.9); Start-Sleep -Milliseconds 700
Check 'guard puts A back' ([Mixer]::Volume($a)) '30 '
$a2 = Play cranked-selftest-a; Start-Sleep -Milliseconds 1200
Check 'new A app gets the knob''s volume' ([Mixer]::Volume($a2)) '30 '
foreach ($p in 31..60) { Send "knob 0 $p"; Start-Sleep -Milliseconds 10 }; Start-Sleep -Milliseconds 700
Check 'fast turn ends on the last value' "$([Mixer]::Volume($a))$([Mixer]::Volume($a2))" '60 60 '
Send 'down 0'; Start-Sleep -Milliseconds 700
Check 'hold A mutes its apps before the release' "$([Mixer]::Muted($a))$([Mixer]::Muted($a2))" 'true true '
Send 'up 0'; Start-Sleep -Milliseconds 300
Send 'down 0'; Start-Sleep -Milliseconds 700; Send 'up 0'; Start-Sleep -Milliseconds 500
Check 'hold again unmutes' "$([Mixer]::Muted($a))$([Mixer]::Muted($a2))" 'false false '
Check 'B was never muted' ([Mixer]::Muted($b)) 'false '
Check 'the output volume was left alone' ([Mixer]::OutputVolume()) $outputBefore

Start-Sleep 2
$service = Get-Process cranked-audiod | Where-Object { $_.Path -eq $Bin } | Select-Object -First 1
$cpu0 = $service.TotalProcessorTime; Start-Sleep 2; $service.Refresh()
'--- idle cost after the test (2 s idle sample):'
"cpu in 2 s: $([math]::Round(($service.TotalProcessorTime - $cpu0).TotalMilliseconds)) ms   working set: $([math]::Round($service.WorkingSet64 / 1MB, 1)) MB   threads: $($service.Threads.Count)"
'--- daemon log:'; Get-Content "$work\log"
$service | Stop-Process -Force
$daemon.StandardInput.Close(); $players | ForEach-Object { Stop-Process -Id $_.Id -Force -ErrorAction SilentlyContinue }
Start-Sleep -Milliseconds 300
Remove-Item -Recurse -Force $work -ErrorAction SilentlyContinue
"failures: $fails"; exit $fails
