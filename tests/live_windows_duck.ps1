# Live Auto-lower check on Windows (the counterpart of tests/live_duck.sh): a "talk" test app plays silence, a ping,
# silence and a tone; the "music" test app should dip and come back. Throwaway apps; no mixer opened; nothing else
# touched. Run it in a desktop session, like tests/live_windows.ps1.
# Usage: powershell -ExecutionPolicy Bypass -File tests\live_windows_duck.ps1 [path\to\cranked-audiod.exe]
param([string]$Bin = "$PSScriptRoot\..\target\x86_64-pc-windows-msvc\release\cranked-audiod.exe")
$ErrorActionPreference = 'Stop'
$Bin = (Resolve-Path $Bin).Path
$work = Join-Path $env:TEMP "cranked-duck-$PID"
New-Item -ItemType Directory -Force $work | Out-Null
$env:CRANKED_SOCKET = "$work\socket"; $env:XDG_STATE_HOME = "$work\state"
@'
popups = false
[auto_lower]
enabled = true
lower = [1]
triggers = [2]
voice_chat = false
amount = 50
[[channel]]
name = "Music"
apps = ["cranked-selftest-music"]
[[channel]]
name = "Talk"
apps = ["cranked-selftest-talk"]
'@ | Set-Content "$work\config.toml"

. "$PSScriptRoot\live_windows_common.ps1"
New-TestPrograms $work cranked-selftest-music, cranked-selftest-talk
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
    $music = Play cranked-selftest-music
    Start-Sleep -Milliseconds 800
    $info = New-Object System.Diagnostics.ProcessStartInfo 'cmd.exe', "/c `"`"$Bin`" --config `"$work\config.toml`" --no-mixer --stdin -v 2> `"$work\log`"`""
    $info.UseShellExecute = $false; $info.RedirectStandardInput = $true; $info.CreateNoWindow = $true
    $daemon = [System.Diagnostics.Process]::Start($info)
    function Send($line) { $daemon.StandardInput.WriteLine($line); $daemon.StandardInput.Flush() }
    Start-Sleep -Milliseconds 1500
    Send 'knob 0 60'; Send 'knob 1 80'; Start-Sleep -Milliseconds 500

    # talk: 0.5 s silence, a 0.3 s ping, 1.2 s silence, a 1 s tone, then silence
    $talk = Play cranked-selftest-talk 'talk'
    Start-Sleep -Milliseconds 1700
    Check 'a short ping on the trigger changes nothing' ([Mixer]::Volume($music)) '60 '
    Start-Sleep -Milliseconds 1450
    Check 'a longer sound lowers the music by half' ([Mixer]::Volume($music)) '30 '
    Start-Sleep -Milliseconds 500
    Check 'and holds it down through a short pause' ([Mixer]::Volume($music)) '30 '
    Start-Sleep -Milliseconds 2500
    Check 'then fades back up' ([Mixer]::Volume($music)) '60 '
    Check 'the trigger app was metered' ((Select-String -Quiet -Path "$work\log" -Pattern 'auto-lower: watching stream')) 'True'

    '--- daemon log:'; Get-Content "$work\log"
} finally {
    Get-Process cranked-audiod -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq $Bin } | Stop-Process -Force
    if ($daemon) { $daemon.StandardInput.Close() }
    $players | ForEach-Object { Stop-Process -Id $_.Id -Force -ErrorAction SilentlyContinue }
    Start-Sleep -Milliseconds 300
    Remove-Item -Recurse -Force $work -ErrorAction SilentlyContinue
}
"failures: $fails"; exit $fails
