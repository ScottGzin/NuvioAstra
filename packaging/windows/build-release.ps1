# Builds the Nuvio Astra release for Windows x64:
#   dist\Nuvio Astra\                       portable folder ("Nuvio Astra.exe", libmpv-2.dll, nuvio_engine.dll, icon)
#   dist\Nuvio-Astra-<version>-x64-setup.exe per-user installer (NSIS)
#
# Usage (from any folder):
#   powershell -NoProfile -ExecutionPolicy Bypass -File packaging\windows\build-release.ps1 [-LibMpv <libmpv-2.dll>] [-Engine <nuvio_engine.dll>] [-SkipBuild]
#
# - Cargo target folder: $env:CARGO_TARGET_DIR, else nextgen\target.
# - libmpv-2.dll: -LibMpv, else $env:NUVIO_LIBMPV, else next to the release build.
# - nuvio_engine.dll (P2P engine of the original app, libtorrent): -Engine, else $env:NUVIO_ENGINE_DLL,
#   else next to the release build, else the original app's copy
#   (composeApp\src\desktopMain\native\windows\engine). Checked before packaging: x64, the C API the app
#   binds, and only Windows system DLLs / the Universal CRT as dependencies (nothing else to ship).
# - makensis: $env:NSIS_HOME, else the NSIS kept by the Tauri bundler (%LOCALAPPDATA%\tauri\NSIS),
#   else Program Files. Nothing is downloaded.
# The data folder (%APPDATA%\NuvioNextGen) is neither read nor written by this script.
param([string]$LibMpv = "", [string]$Engine = "", [switch]$SkipBuild, [string]$Out = "")
$ErrorActionPreference = "Stop"
$root = Resolve-Path (Join-Path $PSScriptRoot "..\..")
$app = Join-Path $root "apps\nuvio-desktop"
$conf = Get-Content (Join-Path $app "tauri.conf.json") -Raw | ConvertFrom-Json
$product = $conf.productName
$version = $conf.version
if ($product -ne "Nuvio Astra") { throw "productName is '$product' (expected 'Nuvio Astra')" }
$target = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $root "target" }
if (-not $Out) { $Out = Join-Path $root "dist" }

if (-not $SkipBuild) {
  Push-Location (Join-Path $app "web"); try { npm run build; if ($LASTEXITCODE) { throw "frontend build failed" } } finally { Pop-Location }
  Push-Location $root; try { cargo build --release -p nuvio-desktop; if ($LASTEXITCODE) { throw "cargo build failed" } } finally { Pop-Location }
}
$exe = Join-Path $target "release\nuvio-desktop.exe"
if (-not (Test-Path $exe)) { throw "release build not found: $exe" }
if (-not $LibMpv) { $LibMpv = if ($env:NUVIO_LIBMPV) { $env:NUVIO_LIBMPV } else { Join-Path $target "release\libmpv-2.dll" } }
if (-not (Test-Path $LibMpv)) { throw "libmpv-2.dll not found: $LibMpv (use -LibMpv)" }

if (-not $Engine) {
  $Engine = @(
    $env:NUVIO_ENGINE_DLL,
    (Join-Path $target "release\nuvio_engine.dll"),
    (Join-Path $root "..\composeApp\src\desktopMain\native\windows\engine\nuvio_engine.dll")
  ) | Where-Object { $_ -and (Test-Path $_) } | Select-Object -First 1
}
if (-not $Engine -or -not (Test-Path $Engine)) { throw "nuvio_engine.dll not found (use -Engine)" }
$Engine = (Resolve-Path $Engine).Path
$LibMpv = (Resolve-Path $LibMpv).Path

# Export and import names of a PE x64 DLL (no external tool needed).
function Get-PeInfo([string]$Path) {
  $b = [IO.File]::ReadAllBytes($Path)
  $pe = [BitConverter]::ToInt32($b, 0x3c)
  $machine = [BitConverter]::ToUInt16($b, $pe + 4)
  $nsec = [BitConverter]::ToUInt16($b, $pe + 6)
  $optSize = [BitConverter]::ToUInt16($b, $pe + 20)
  $opt = $pe + 24
  $dd = $opt + 112
  $secs = 0..($nsec - 1) | ForEach-Object { $o = $opt + $optSize + $_ * 40; [pscustomobject]@{ VSize = [BitConverter]::ToUInt32($b, $o + 8); VA = [BitConverter]::ToUInt32($b, $o + 12); RSize = [BitConverter]::ToUInt32($b, $o + 16); Raw = [BitConverter]::ToUInt32($b, $o + 20) } }
  $off = { param($rva) foreach ($s in $secs) { if ($rva -ge $s.VA -and $rva -lt $s.VA + [Math]::Max($s.VSize, $s.RSize)) { return [int]($rva - $s.VA + $s.Raw) } }; throw "bad RVA" }
  $str = { param($o) $e = $o; while ($b[$e] -ne 0) { $e++ }; [Text.Encoding]::ASCII.GetString($b, $o, $e - $o) }
  $exp = & $off ([BitConverter]::ToUInt32($b, $dd))
  $n = [BitConverter]::ToUInt32($b, $exp + 24)
  $names = & $off ([BitConverter]::ToUInt32($b, $exp + 32))
  $exports = 0..($n - 1) | ForEach-Object { & $str (& $off ([BitConverter]::ToUInt32($b, $names + 4 * $_))) }
  $imports = @(); $i = & $off ([BitConverter]::ToUInt32($b, $dd + 8))
  while ([BitConverter]::ToUInt32($b, $i + 12) -ne 0) { $imports += & $str (& $off ([BitConverter]::ToUInt32($b, $i + 12))); $i += 20 }
  [pscustomobject]@{ Machine = $machine; Exports = $exports; Imports = $imports }
}
$info = Get-PeInfo $Engine
if ($info.Machine -ne 0x8664) { throw "nuvio_engine.dll is not x64" }
$needed = "nuvio_engine_api_version", "nuvio_engine_create", "nuvio_engine_destroy", "nuvio_engine_add_torrent", "nuvio_engine_poll_event", "nuvio_engine_prepare_stream", "nuvio_engine_stop_stream", "nuvio_engine_remove_torrent", "nuvio_engine_get_stats", "nuvio_engine_get_stream_stats", "nuvio_engine_reclaim_disk_cache"
$missing = $needed | Where-Object { $info.Exports -notcontains $_ }
if ($missing) { throw "nuvio_engine.dll lacks: $($missing -join ', ')" }
$system = "kernel32", "user32", "advapi32", "ws2_32", "bcrypt", "crypt32", "mswsock", "iphlpapi", "ntdll", "shell32", "ole32"
$foreign = $info.Imports | Where-Object { $d = $_.ToLower() -replace '\.dll$', ''; -not ($system -contains $d -or $d.StartsWith("api-ms-win-")) }
if ($foreign) { throw "nuvio_engine.dll depends on DLLs that are not shipped: $($foreign -join ', ')" }
Write-Host ("P2P engine: {0} ({1} bytes), {2} exports, dependencies: {3}" -f $Engine, (Get-Item $Engine).Length, $info.Exports.Count, ($info.Imports -join ', '))

# Portable folder, named after the product.
$portable = Join-Path $Out $product
if (Test-Path $portable) { Remove-Item $portable -Recurse -Force }
New-Item -ItemType Directory $portable | Out-Null
Copy-Item $exe (Join-Path $portable "$product.exe")
Copy-Item $LibMpv (Join-Path $portable "libmpv-2.dll")
Copy-Item $Engine (Join-Path $portable "nuvio_engine.dll")
# Licenses: Nuvio Astra (GPL-3.0), third-party notices, the Nuvio Engine's bundled licenses.
$licenses = Join-Path $root "packaging\licenses"
if (-not (Test-Path (Join-Path $licenses "THIRD_PARTY_NOTICES.md"))) { throw "packaging\licenses is incomplete" }
Copy-Item $licenses (Join-Path $portable "licenses") -Recurse
Copy-Item (Join-Path $app "icons\icon.ico") (Join-Path $portable "nuvio-astra.ico")

# Installer.
$makensis = @(
  $(if ($env:NSIS_HOME) { Join-Path $env:NSIS_HOME "makensis.exe" }),
  (Join-Path $env:LOCALAPPDATA "tauri\NSIS\makensis.exe"),
  "${env:ProgramFiles(x86)}\NSIS\makensis.exe",
  "$env:ProgramFiles\NSIS\makensis.exe"
) | Where-Object { $_ -and (Test-Path $_) } | Select-Object -First 1
if (-not $makensis) { throw "makensis.exe not found (set NSIS_HOME)" }
$setup = Join-Path $Out ("{0}-{1}-x64-setup.exe" -f ($product -replace ' ', '-'), $version)
& $makensis /V2 "/DVERSION=$version" "/DSRCDIR=$portable" "/DOUTFILE=$setup" (Join-Path $PSScriptRoot "nuvio-astra.nsi")
if ($LASTEXITCODE) { throw "makensis failed ($LASTEXITCODE)" }

Write-Host "Portable : $portable"
Write-Host "Installer: $setup"
