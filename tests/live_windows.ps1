# Live check against Windows audio (the Windows counterpart of tests/live.sh): two throwaway silent players and a
# media-key watcher,
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

. "$PSScriptRoot\live_windows_common.ps1"
New-TestPrograms $work cranked-selftest-a, cranked-selftest-b
$players = New-Object System.Collections.ArrayList
function Play($name, [string[]]$arguments = @('loop')) {
    $p = Start-Process "$work\$name.exe" -ArgumentList $arguments -WindowStyle Hidden -PassThru
    [void]$players.Add($p); return [uint32]$p.Id
}

$fails = 0
function Check($what, $got, $want) {
    if ("$got" -eq "$want") { "ok   $what ($got)" } else { "FAIL ${what}: got '$got', want '$want'"; $script:fails++ }
}

$daemon = $null
# cleanup runs even when a step throws, so no test app or service is left behind
try {
    $a = Play cranked-selftest-a; $b = Play cranked-selftest-b
    # stands in for the media player: notes the media keys the service sends (and keeps them from real players)
    $keys = Play keywatch "$work\keys"
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
    Send 'down 0'; Start-Sleep -Milliseconds 100; Send 'up 0'; Start-Sleep -Milliseconds 400
    Check 'press A sends play/pause' (Get-Content "$work\keys") 'ready PlayPause '
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
} finally {
    Get-Process cranked-audiod -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq $Bin } | Stop-Process -Force
    if ($daemon) { $daemon.StandardInput.Close() }
    $players | ForEach-Object { Stop-Process -Id $_.Id -Force -ErrorAction SilentlyContinue }
    Start-Sleep -Milliseconds 300
    Remove-Item -Recurse -Force $work -ErrorAction SilentlyContinue
}
"failures: $fails"; exit $fails
