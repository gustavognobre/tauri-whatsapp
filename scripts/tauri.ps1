$ErrorActionPreference = "Stop"

$vswherePath = "C:\Program Files (x86)\Microsoft Visual Studio\Installer\vswhere.exe"
if (-not (Test-Path $vswherePath)) {
  Write-Error "Visual Studio Build Tools nao encontrado. Instale Microsoft.VisualStudio.2022.BuildTools."
}

$installationPath = & $vswherePath `
  -latest `
  -products * `
  -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 `
  -property installationPath | Select-Object -First 1

if (-not $installationPath) {
  Write-Error "Visual C++ Build Tools nao encontrado. Instale o workload Desktop development with C++."
}

$vsDevCmdPath = Join-Path $installationPath "Common7\Tools\VsDevCmd.bat"
if (-not (Test-Path $vsDevCmdPath)) {
  Write-Error "VsDevCmd.bat nao encontrado em $vsDevCmdPath"
}

$tauriCmdPath = Join-Path $PSScriptRoot "..\node_modules\.bin\tauri.cmd"
$tauriCmdPath = (Resolve-Path $tauriCmdPath).Path

$cargoBin = Join-Path $env:USERPROFILE ".cargo\bin"
$quotedArgs = @(
  $args | ForEach-Object {
    '"' + ($_ -replace '"', '\"') + '"'
  }
) -join " "

$command = @(
  "call `"$vsDevCmdPath`" -arch=x64 -host_arch=x64"
  "set `"PATH=$cargoBin;%PATH%`""
  "`"$tauriCmdPath`" $quotedArgs".Trim()
) -join " && "

& cmd.exe /d /c $command
exit $LASTEXITCODE
