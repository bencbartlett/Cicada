<#
.SYNOPSIS
Carve speed (docs/15 measurement protocol, criterion 1): cold + warm
`cicada run <pipeline> --node <node> --time`, 3 runs each, a fresh
--cache-dir per cold run (the warm run of a pair reuses its cold run's
cache), reporting best/median of the solve wall time (`time: total`), the
target node's own compute time, and the whole-process wall time.

.EXAMPLE
corpus\measure\carve.ps1                                   # corpus/wall.cic --node carved, 3 runs
corpus\measure\carve.ps1 -Pipeline examples\03-voronoi.cic -Node carved -Runs 3
$env:CICADA_BIN = "$env:LOCALAPPDATA\cargo-target\release\cicada.exe"; corpus\measure\carve.ps1

.NOTES
Windows PowerShell 5.1 compatible. The engine binary: -Bin / $env:CICADA_BIN,
default $env:CARGO_TARGET_DIR\release\cicada.exe - build it with
`cargo build --release -p cicada-cli`; record numbers come from a RELEASE
build, never debug. -Threads passes --threads. Prints a JSON result, then
one summary line; nonzero exit = a run failed. -Out also writes the JSON
to a file.
#>
param(
  [string]$Pipeline = "corpus/wall.cic",
  [string]$Node = "carved",
  [int]$Runs = 3,
  [string]$Bin = "",
  [int]$Threads = 0,
  [string]$Out = ""
)
$ErrorActionPreference = "Stop"

if ($Bin -eq "") {
  if ($env:CICADA_BIN) { $Bin = $env:CICADA_BIN }
  else {
    $targetDir = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { "target" }
    $Bin = Join-Path $targetDir "release\cicada.exe"
  }
}
if (-not (Test-Path $Bin)) {
  Write-Error "no engine binary at $Bin - build it: cargo build --release -p cicada-cli (or set CICADA_BIN / -Bin)"
  exit 2
}
if (-not (Test-Path $Pipeline)) { Write-Error "no pipeline at $Pipeline"; exit 2 }

$scratch = Join-Path ([System.IO.Path]::GetTempPath()) ("cicada-carve-" + [System.Diagnostics.Process]::GetCurrentProcess().Id)
New-Item -ItemType Directory -Force -Path $scratch | Out-Null

function Invoke-Run([string]$cache, [string]$label) {
  $runArgs = @("run", $Pipeline, "--node", $Node, "--time", "--cache-dir", $cache)
  if ($Threads -gt 0) { $runArgs += @("--threads", "$Threads") }
  $sw = [System.Diagnostics.Stopwatch]::StartNew()
  # stderr is left alone on purpose (PS 5.1 wraps redirected native stderr in ErrorRecords).
  $lines = & $Bin @runArgs
  $status = $LASTEXITCODE
  $sw.Stop()
  if ($status -ne 0) {
    Write-Error ("{0} run failed (exit {1}):`n{2}" -f $label, $status, ($lines -join "`n"))
    exit 1
  }
  $total = $lines | Where-Object { $_ -match '^time: total ([0-9.]+) ms wall .* ([0-9]+) computed, ([0-9]+) from cache' } | Select-Object -Last 1
  if (-not $total) {
    Write-Error ("no 'time: total' line in the {0} run's output:`n{1}" -f $label, ($lines -join "`n"))
    exit 1
  }
  $null = $total -match '^time: total ([0-9.]+) ms wall .* ([0-9]+) computed, ([0-9]+) from cache'
  $solve = [double]$Matches[1]; $computed = [int]$Matches[2]; $hits = [int]$Matches[3]
  $nodeMs = $null
  $nodeLine = $lines | Where-Object { $_ -match ('^time: ' + [regex]::Escape($Node) + ' .* ([0-9.]+) ms') } | Select-Object -Last 1
  if ($nodeLine) { $null = $nodeLine -match '([0-9.]+) ms'; $nodeMs = [double]$Matches[1] }
  $r = [ordered]@{ label = $label; solve_ms = $solve; node_ms = $nodeMs; process_ms = [int]$sw.ElapsedMilliseconds; computed = $computed; from_cache = $hits }
  Write-Host ("{0}: solve {1} ms, {2} {3} ms, process {4} ms ({5} computed, {6} cached)" -f $label, $solve, $Node, $nodeMs, $r.process_ms, $computed, $hits)
  return $r
}

$results = @()
try {
  for ($i = 1; $i -le $Runs; $i++) {
    $cache = Join-Path $scratch "cache-$i"
    $results += Invoke-Run $cache "cold-$i"
    $results += Invoke-Run $cache "warm-$i"
  }
} finally {
  Remove-Item -Recurse -Force $scratch -ErrorAction SilentlyContinue
}

function Get-Stat([string]$prefix, [string]$field) {
  $vals = @($results | Where-Object { $_.label -like "$prefix*" } | ForEach-Object { $_[$field] } | Where-Object { $_ -ne $null } | Sort-Object)
  if ($vals.Count -eq 0) { return [ordered]@{ best = $null; median = $null } }
  # median = the middle element (the upper one for an even count)
  $median = $vals[[math]::Floor($vals.Count / 2)]
  return [ordered]@{ best = $vals[0]; median = $median }
}

$result = [ordered]@{
  harness = "carve"
  pipeline = $Pipeline
  node = $Node
  bin = $Bin
  runs = $Runs
  threads = $Threads
  cold = [ordered]@{ solve_ms = (Get-Stat "cold" "solve_ms"); node_ms = (Get-Stat "cold" "node_ms"); process_ms = (Get-Stat "cold" "process_ms") }
  warm = [ordered]@{ solve_ms = (Get-Stat "warm" "solve_ms"); node_ms = (Get-Stat "warm" "node_ms"); process_ms = (Get-Stat "warm" "process_ms") }
  target = [ordered]@{ cold_solve_ms_lt = 10000; warm_solve_ms_lt = 100 }
  runs_detail = $results
}
$json = $result | ConvertTo-Json -Depth 6
Write-Output $json
if ($Out -ne "") { $json | Out-File -Encoding utf8 $Out }
$cs = $result.cold.solve_ms; $ws = $result.warm.solve_ms; $cp = $result.cold.process_ms; $wp = $result.warm.process_ms
Write-Output ("carve {0} --node {1} x{2}: cold solve best/median {3}/{4} ms, warm solve best/median {5}/{6} ms (targets: cold < 10000 ms, warm < 100 ms); process wall cold {7}/{8} ms / warm {9}/{10} ms" -f $Pipeline, $Node, $Runs, $cs.best, $cs.median, $ws.best, $ws.median, $cp.best, $cp.median, $wp.best, $wp.median)
