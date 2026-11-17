param(
  [Parameter(Mandatory = $true)][string]$CurrentInstaller,
  [string]$BaselineInstaller,
  [string]$BaselineVersion
)

# Runs only on an isolated CI runner with no connected hardware or saved tasks.
$ErrorActionPreference = 'Stop'
$expectedVersion = (Get-Content (Join-Path $PSScriptRoot '../package.json') | ConvertFrom-Json).version
if ([bool]$BaselineInstaller -ne [bool]$BaselineVersion) {
  throw 'Baseline installer and version must be supplied together.'
}

function Invoke-Installer([string]$Path, [string[]]$Arguments) {
  $process = Start-Process -FilePath $Path -ArgumentList $Arguments -PassThru
  if (-not $process.WaitForExit(180000)) { throw "Installer timed out: $Path" }
  if ($process.ExitCode -ne 0) { throw "Installer failed with $($process.ExitCode): $Path" }
}

function Get-InstalledDirectory([string]$Version) {
  $directory = (Get-ItemProperty 'HKCU:\Software\DshanPI\LYNX').'(default)'
  $displayVersion = (Get-ItemProperty 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\LYNX').DisplayVersion
  if ($displayVersion -ne $Version) { throw "Installed version is $displayVersion, expected $Version" }
  if ($directory -notmatch "app-$([regex]::Escape($Version))-\d+$") {
    throw "Incorrect versioned install directory: $directory"
  }
  if (-not (Test-Path (Join-Path $directory 'lynx-app.exe'))) { throw 'Installed executable missing.' }
  return $directory
}

function Test-Application([string]$Directory) {
  $application = Start-Process -FilePath (Join-Path $Directory 'lynx-app.exe') -PassThru
  Start-Sleep -Seconds 10
  if ($application.HasExited) { throw "Installed LYNX exited early with $($application.ExitCode)." }
  # Normal close goes through LYNX's hardware/task shutdown guard.
  if (-not $application.CloseMainWindow()) { throw 'LYNX did not expose a closable window.' }
  if (-not $application.WaitForExit(15000)) { throw 'LYNX rejected or did not complete normal shutdown.' }
}

if ($BaselineInstaller) {
  Invoke-Installer $BaselineInstaller @('/S')
  $baselineDirectory = Get-InstalledDirectory $BaselineVersion
  Test-Application $baselineDirectory
}
Invoke-Installer $CurrentInstaller @('/S')
$directory = Get-InstalledDirectory $expectedVersion
if ($BaselineInstaller -and $directory -eq $baselineDirectory) { throw 'Upgrade reused the old program directory.' }
Test-Application $directory

# Same-version reinstall is required even when 0.1.0 has no upgrade baseline.
Invoke-Installer $CurrentInstaller @('/S', '/UPDATE')
$directory = Get-InstalledDirectory $expectedVersion
Test-Application $directory
Invoke-Installer (Join-Path $directory 'uninstall.exe') @('/S')
$deadline = (Get-Date).AddSeconds(30)
while ((Test-Path (Join-Path $directory 'lynx-app.exe')) -and (Get-Date) -lt $deadline) {
  Start-Sleep -Milliseconds 250
}
if (Test-Path (Join-Path $directory 'lynx-app.exe')) { throw 'Uninstall left the application executable behind.' }
