#requires -Version 5.1
<#
.SYNOPSIS
  Builds the Windows app into apps\windows\dist\SJTUCanvasDownloader (and optionally a zip).

.DESCRIPTION
  1. sjtu-canvas-engine (Rust, release, static C runtime)
  2. the WinUI 3 app, published self-contained (.NET and Windows App SDK)
  3. layout:  SJTUCanvasDownloader.exe
              engine\sjtu-canvas-engine.exe
  4. a check that every .exe/.dll can load on a clean Windows PC
  5. optional Authenticode signing of every unsigned binary (a signed build
     avoids the SmartScreen warning once the certificate has reputation, and
     is required where Smart App Control is on):
       -SignThumbprint <SHA-1 of a certificate in the user's store>, or
       -SignPfx <file.pfx> with the password in SJTU_CANVAS_SIGN_PASSWORD
     (or SJTU_CANVAS_SIGN_THUMBPRINT / SJTU_CANVAS_SIGN_PFX), timestamped by
     -TimestampUrl (default http://timestamp.digicert.com).

  Requirements: Rust (MSVC toolchain; for -Arch arm64 also the
  aarch64-pc-windows-msvc target and the ARM64 C++ build tools), .NET 10 SDK,
  Python 3 for the DLL check. The .NET SDK is taken from PATH, or from
  .dev\dotnet in the repository if present.

.EXAMPLE
  ./apps/windows/build.ps1 -Zip
.EXAMPLE
  $env:SJTU_CANVAS_SIGN_PASSWORD = "..."; ./apps/windows/build.ps1 -Zip -SignPfx C:\keys\canvas.pfx
#>
param(
    [ValidateSet("x64", "arm64")] [string]$Arch = "x64",
    [switch]$Zip,
    [string]$SignThumbprint = $env:SJTU_CANVAS_SIGN_THUMBPRINT,
    [string]$SignPfx = $env:SJTU_CANVAS_SIGN_PFX,
    [string]$TimestampUrl = $(if ($env:SJTU_CANVAS_SIGN_TIMESTAMP) { $env:SJTU_CANVAS_SIGN_TIMESTAMP } else { "http://timestamp.digicert.com" })
)

$ErrorActionPreference = "Stop"
$Repo = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path
$Dist = Join-Path $PSScriptRoot "dist"
$App = Join-Path $Dist "SJTUCanvasDownloader"
$RustTarget = if ($Arch -eq "arm64") { "aarch64-pc-windows-msvc" } else { "x86_64-pc-windows-msvc" }
$Platform = if ($Arch -eq "arm64") { "ARM64" } else { "x64" }

function Invoke-Native([string]$What, [scriptblock]$Command) {
    $previous = $ErrorActionPreference
    $ErrorActionPreference = "Continue"
    try { & $Command 2>&1 | ForEach-Object { "$_" } } finally { $ErrorActionPreference = $previous }
    if ($LASTEXITCODE -ne 0) { throw "$What failed (exit code $LASTEXITCODE)" }
}

function Find-SignTool {
    $onPath = Get-Command signtool.exe -ErrorAction SilentlyContinue
    if ($onPath) { return $onPath.Source }
    $roots = @()
    $installed = Get-ItemProperty "HKLM:\SOFTWARE\Microsoft\Windows Kits\Installed Roots" -ErrorAction SilentlyContinue
    if ($installed -and $installed.KitsRoot10) { $roots += $installed.KitsRoot10 }
    $roots += Join-Path ${env:ProgramFiles(x86)} "Windows Kits\10"
    foreach ($root in $roots) {
        $tool = Get-ChildItem (Join-Path $root "bin\*\x64\signtool.exe") -ErrorAction SilentlyContinue |
            Sort-Object { [version]$_.Directory.Parent.Name } -Descending | Select-Object -First 1
        if ($tool) { return $tool.FullName }
    }
    throw "signtool.exe not found; install the Windows SDK."
}

function Find-Python {
    foreach ($name in "python", "python3", "py") {
        $command = Get-Command $name -ErrorAction SilentlyContinue
        if ($command) { return $command.Source }
    }
    return $null
}

$localDotnet = Join-Path $Repo ".dev\dotnet"
if (Test-Path (Join-Path $localDotnet "dotnet.exe")) {
    $env:DOTNET_ROOT = $localDotnet
    $env:PATH = "$localDotnet;$env:PATH"
}
$env:DOTNET_CLI_TELEMETRY_OPTOUT = "1"
$env:DOTNET_NOLOGO = "1"

Write-Host "==> Engine (release, $RustTarget)"
# Run from the repository so .cargo\config.toml (static C runtime) applies.
Push-Location $Repo
try {
    Invoke-Native "cargo build" { cargo build --release --locked --target $RustTarget --manifest-path (Join-Path $Repo "engine\Cargo.toml") }
} finally { Pop-Location }
$engineExe = Join-Path $Repo "engine\target\$RustTarget\release\sjtu-canvas-engine.exe"

Write-Host "==> WinUI app (win-$Arch)"
if (Test-Path $App) { Remove-Item -Recurse -Force $App }
Invoke-Native "dotnet publish" {
    dotnet publish (Join-Path $PSScriptRoot "SJTUCanvasDownloader\SJTUCanvasDownloader.csproj") -c Release -r "win-$Arch" "-p:Platform=$Platform" --self-contained -o $App
}

Write-Host "==> Layout"
$engineDir = Join-Path $App "engine"
New-Item -ItemType Directory -Force $engineDir | Out-Null
Copy-Item $engineExe $engineDir
Copy-Item (Join-Path $Repo "LICENSE"), (Join-Path $Repo "THIRD_PARTY_NOTICES.md") $App
# Debug symbols are not shipped.
Get-ChildItem $App -Recurse -File -Filter *.pdb | Remove-Item -Force

Write-Host "==> Clean-PC DLL check"
$python = Find-Python
if ($python) {
    Invoke-Native "DLL import check" { & $python -B (Join-Path $PSScriptRoot "tools\check-dlls.py") $App }
} else {
    Write-Warning "Python not found; skipped the DLL import check."
}

if ($SignThumbprint -or $SignPfx) {
    Write-Host "==> Authenticode signing"
    $signtool = Find-SignTool
    $unsigned = Get-ChildItem $App -Recurse -File -Include *.exe, *.dll |
        Where-Object { (Get-AuthenticodeSignature $_.FullName).Status -eq "NotSigned" }
    $signArgs = @("sign", "/fd", "sha256", "/tr", $TimestampUrl, "/td", "sha256")
    if ($SignPfx) {
        $signArgs += @("/f", $SignPfx)
        if ($env:SJTU_CANVAS_SIGN_PASSWORD) { $signArgs += @("/p", $env:SJTU_CANVAS_SIGN_PASSWORD) }
    } else {
        $signArgs += @("/sha1", $SignThumbprint)
    }
    Write-Host ("Signing {0} files" -f $unsigned.Count)
    for ($index = 0; $index -lt $unsigned.Count; $index += 40) {
        $batch = @($unsigned[$index..([Math]::Min($index + 39, $unsigned.Count - 1))] | ForEach-Object FullName)
        Invoke-Native "signtool sign" { & $signtool @signArgs @batch }
    }
    Invoke-Native "signtool verify" { & $signtool verify /pa /q (Join-Path $App "SJTUCanvasDownloader.exe") (Join-Path $engineDir "sjtu-canvas-engine.exe") }
}

if ($Zip) {
    $archive = Join-Path $Dist "SJTUCanvasDownloader-win-$Arch.zip"
    if (Test-Path $archive) { Remove-Item -Force $archive }
    Write-Host "==> $archive"
    Compress-Archive -Path $App -DestinationPath $archive -CompressionLevel Optimal
}

$size = (Get-ChildItem $App -Recurse -File | Measure-Object Length -Sum).Sum / 1MB
Write-Host ("Done: {0} ({1:N0} MB)" -f $App, $size)
