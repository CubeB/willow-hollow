# Samples the CPU time of every thread of the running game.exe every $Interval ms, by
# thread name (Rust names its threads through SetThreadDescription), and writes one CSV
# row per sample per thread name: t_s, name, threads, cpu_ms_in_interval.
# Usage: powershell -File tools/bench/threads.ps1 -Out file.csv [-Interval 250] [-WaitSeconds 60]
# Start it just before the game; it waits for game.exe to appear and stops when it exits.
param([string]$Out, [int]$Interval = 250, [int]$WaitSeconds = 60)
Add-Type -TypeDefinition @"
using System;
using System.Runtime.InteropServices;
public static class ThreadNames {
    [DllImport("kernel32.dll", SetLastError=true)] static extern IntPtr OpenThread(uint access, bool inherit, uint id);
    [DllImport("kernel32.dll")] static extern bool CloseHandle(IntPtr h);
    [DllImport("kernel32.dll")] static extern int GetThreadDescription(IntPtr h, out IntPtr desc);
    [DllImport("kernel32.dll")] static extern IntPtr LocalFree(IntPtr p);
    public static string Name(int id) {
        IntPtr h = OpenThread(0x0800, false, (uint)id);
        if (h == IntPtr.Zero) return "?";
        try {
            IntPtr p;
            if (GetThreadDescription(h, out p) < 0) return "?";
            string s = Marshal.PtrToStringUni(p);
            LocalFree(p);
            return String.IsNullOrEmpty(s) ? "unnamed" : s;
        } finally { CloseHandle(h); }
    }
}
"@
$deadline = (Get-Date).AddSeconds($WaitSeconds)
$proc = $null
while (-not $proc -and (Get-Date) -lt $deadline) {
    $proc = Get-Process game -ErrorAction SilentlyContinue | Select-Object -First 1
    if (-not $proc) { Start-Sleep -Milliseconds 100 }
}
if (-not $proc) { "no game.exe" | Out-File $Out; exit 1 }
$start = Get-Date
$names = @{}
$last = @{}
$lines = New-Object System.Collections.Generic.List[string]
$lines.Add("t_s,name,threads,cpu_ms")
while (-not $proc.HasExited) {
    $proc.Refresh()
    $agg = @{}
    $count = @{}
    try { $threads = $proc.Threads } catch { break }
    foreach ($t in $threads) {
        try { $cpu = $t.TotalProcessorTime.TotalMilliseconds } catch { continue }
        if (-not $names.ContainsKey($t.Id)) { $names[$t.Id] = [ThreadNames]::Name($t.Id) }
        $n = $names[$t.Id] -replace '-\d+$', '-N'
        $prev = if ($last.ContainsKey($t.Id)) { $last[$t.Id] } else { $cpu }
        $last[$t.Id] = $cpu
        $agg[$n] = $agg[$n] + ($cpu - $prev)
        $count[$n] = $count[$n] + 1
    }
    $ts = ((Get-Date) - $start).TotalSeconds
    foreach ($k in $agg.Keys) { $lines.Add(("{0:F2},{1},{2},{3:F1}" -f $ts, $k, $count[$k], $agg[$k])) }
    Start-Sleep -Milliseconds $Interval
}
$lines | Out-File -Encoding utf8 $Out
