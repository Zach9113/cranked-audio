# Shared by the Windows live checks (dot-source it): reads Windows' per-app volumes, and builds the throwaway test
# programs: a player (loops silence, or with "talk" plays silence, a ping, a pause and a tone like live_duck.sh's talk
# stream) and a key watcher that writes down the media keys it sees.

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

$testPrograms = @'
using System; using System.IO; using System.Media; using System.Runtime.InteropServices;
public static class Player {
    // 48 kHz mono 16-bit: (seconds, tone?) pieces; the tone is 440 Hz at about -12 dBFS
    static MemoryStream Wav(params double[] pieces) {
        int rate = 48000; var samples = new MemoryStream(); var s = new BinaryWriter(samples); long n = 0;
        for (int p = 0; p < pieces.Length; p += 2)
            for (int i = 0; i < (int)(rate * pieces[p]); i++, n++)
                s.Write((short)(pieces[p + 1] > 0 ? 8000 * Math.Sin(2 * Math.PI * 440 * n / rate) : 0));
        var wav = new MemoryStream(); var w = new BinaryWriter(wav); int bytes = (int)samples.Length;
        w.Write("RIFF".ToCharArray()); w.Write(36 + bytes); w.Write("WAVEfmt ".ToCharArray()); w.Write(16); w.Write((short)1);
        w.Write((short)1); w.Write(rate); w.Write(rate * 2); w.Write((short)2); w.Write((short)16);
        w.Write("data".ToCharArray()); w.Write(bytes); w.Write(samples.ToArray()); wav.Position = 0;
        return wav;
    }
    public static void Main(string[] args) {
        if (args.Length > 0 && args[0] == "talk") new SoundPlayer(Wav(0.5, 0, 0.3, 1, 1.2, 0, 1.0, 1, 6, 0)).PlaySync();
        new SoundPlayer(Wav(1, 0)).PlayLooping();
        System.Threading.Thread.Sleep(System.Threading.Timeout.Infinite);
    }
}
public static class KeyWatch {
    delegate IntPtr Hook(int code, IntPtr message, IntPtr data);
    [DllImport("user32.dll")] static extern IntPtr SetWindowsHookEx(int id, Hook hook, IntPtr module, uint thread);
    [DllImport("user32.dll")] static extern IntPtr CallNextHookEx(IntPtr hook, int code, IntPtr message, IntPtr data);
    [DllImport("user32.dll")] static extern int GetMessage(out MSG message, IntPtr window, uint min, uint max);
    [DllImport("kernel32.dll")] static extern IntPtr GetModuleHandle(string name);
    [StructLayout(LayoutKind.Sequential)] struct MSG { public IntPtr hwnd; public uint message; public IntPtr w, l; public uint time; public int x, y; }
    static Hook keep; static string file;
    public static void Main(string[] args) {
        file = args[0];
        keep = (code, message, data) => {
            // WM_KEYDOWN; the virtual key is the first field of KBDLLHOOKSTRUCT
            if (code >= 0 && (int)message == 0x100) {
                int key = Marshal.ReadInt32(data);
                string name = key == 0xB3 ? "PlayPause" : key == 0xB0 ? "Next" : key == 0xB1 ? "Previous" : null;
                // swallowed, so the test's media keys don't reach a real player
                if (name != null) { File.AppendAllText(file, name + " "); return (IntPtr)1; }
            }
            return CallNextHookEx(IntPtr.Zero, code, message, data);
        };
        SetWindowsHookEx(13, keep, GetModuleHandle(null), 0);
        File.AppendAllText(file, "ready ");
        MSG m; while (GetMessage(out m, IntPtr.Zero, 0, 0) > 0) {}
    }
}
'@
# Builds the player as NAME.exe for each name (apps are told apart by program name), and keywatch.exe.
function New-TestPrograms($work, [string[]]$names) {
    # one source, two programs: the entry point is picked when each .exe is compiled
    $compiler = New-Object Microsoft.CSharp.CSharpCodeProvider
    foreach ($program in 'Player', 'KeyWatch') {
        $options = New-Object System.CodeDom.Compiler.CompilerParameters
        $options.GenerateExecutable = $true; $options.MainClass = $program
        $options.OutputAssembly = "$work\$($program.ToLower()).exe"
        [void]$options.ReferencedAssemblies.Add('System.dll')
        $built = $compiler.CompileAssemblyFromSource($options, $testPrograms)
        if ($built.Errors.HasErrors) { throw ($built.Errors | Out-String) }
    }
    foreach ($name in $names) { Copy-Item "$work\player.exe" "$work\$name.exe" }
}
