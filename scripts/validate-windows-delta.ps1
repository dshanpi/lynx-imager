param(
  [Parameter(Mandatory = $true)][string]$ReleaseDirectory,
  [Parameter(Mandatory = $true)][string]$PreviousInstaller,
  [Parameter(Mandatory = $true)][string]$FromVersion,
  [Parameter(Mandatory = $true)][string]$ToVersion
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$productKey = 'HKCU:\Software\dshanpi\LYNX'
$uninstallKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\LYNX'
$installRoot = Join-Path $env:LOCALAPPDATA 'DshanPI\LYNX'
$failureMarker = Join-Path $env:APPDATA 'com.dshanpi.lynx\update-delta-failure'

function Assert-True([bool]$Condition, [string]$Message) {
  if (-not $Condition) { throw $Message }
}

function Invoke-Installer([string]$Path, [string[]]$Arguments, [string]$Stage) {
  Write-Host "Starting ${Stage}: $Path $($Arguments -join ' ')"
  $process = Start-Process -FilePath $Path -ArgumentList $Arguments -PassThru
  if (-not $process.WaitForExit(180000)) {
    Write-Host "$Stage exceeded the 180-second installer timeout; terminating PID $($process.Id)."
    & "$env:SystemRoot\System32\taskkill.exe" /PID $process.Id /T /F | Out-Host
    throw "$Stage timed out after 180 seconds."
  }
  Write-Host "$Stage exited with code $($process.ExitCode)."
  return $process.ExitCode
}

function Stop-LynxProcesses {
  @('lynx-imager', 'openixcli', 'openrockcli', 'uuu', 'dfu-util') | ForEach-Object {
    Stop-Process -Name $_ -Force -ErrorAction SilentlyContinue
  }
}

function Wait-PathAbsent([string]$Path, [string]$Stage) {
  $deadline = (Get-Date).AddSeconds(30)
  while (Test-Path -LiteralPath $Path) {
    if ((Get-Date) -ge $deadline) {
      throw "$Stage left $Path behind after 30 seconds."
    }
    Start-Sleep -Milliseconds 500
  }
  Write-Host "$Stage removed $Path."
}

function Reset-LynxInstall {
  Stop-LynxProcesses
  Remove-Item -LiteralPath $productKey -Recurse -Force -ErrorAction SilentlyContinue
  Remove-Item -LiteralPath $uninstallKey -Recurse -Force -ErrorAction SilentlyContinue
  Remove-Item -LiteralPath $installRoot -Recurse -Force -ErrorAction SilentlyContinue
  Remove-Item -LiteralPath $failureMarker -Force -ErrorAction SilentlyContinue
}

function Get-InstallDirectory {
  return (Get-ItemProperty -LiteralPath $productKey).'(default)'
}

function Assert-InstalledVersion([string]$ExpectedVersion) {
  $directory = Get-InstallDirectory
  $displayVersion = Get-ItemPropertyValue -LiteralPath $uninstallKey -Name DisplayVersion
  Assert-True ($displayVersion -eq $ExpectedVersion) `
    "Installed version is $displayVersion, expected $ExpectedVersion"
  Assert-True ($directory -match "app-$([regex]::Escape($ExpectedVersion))-\d+$") `
    "Unexpected versioned install directory: $directory"
  foreach ($relative in @(
    'lynx-imager.exe',
    'bin\openixcli.exe',
    'bin\openrockcli.exe',
    'bin\uuu.exe',
    'bin\dfu-util.exe'
  )) {
    Assert-True (Test-Path -LiteralPath (Join-Path $directory $relative) -PathType Leaf) `
      "Installed LYNX is missing $relative"
  }
  return $directory
}

$releaseRoot = (Resolve-Path -LiteralPath $ReleaseDirectory).Path
$oldInstaller = (Resolve-Path -LiteralPath $PreviousInstaller).Path
$fullInstaller = @(Get-ChildItem -LiteralPath $releaseRoot -Filter "LYNX_${ToVersion}_x64-setup.exe")
$deltaInstaller = @(Get-ChildItem -LiteralPath $releaseRoot -Filter "LYNX_${ToVersion}_from_${FromVersion}_x64-delta.exe")
Assert-True ($fullInstaller.Count -eq 1) 'Expected exactly one current full Windows installer.'
Assert-True ($deltaInstaller.Count -eq 1) 'Expected exactly one exact-version Windows delta installer.'
Assert-True ($deltaInstaller[0].Length -lt $fullInstaller[0].Length) 'Delta is not smaller than full installer.'
Assert-True ($deltaInstaller[0].Length -le 32MB) 'Delta exceeds the 32 MiB ceiling.'

try {
  Reset-LynxInstall

  # Failure path: an exact-version mismatch must leave the current tree untouched.
  Assert-True ((Invoke-Installer $oldInstaller @('/S', '/UPDATE') 'Install public base for failure path') -eq 0) `
    "Failed to install LYNX $FromVersion"
  $oldDirectory = Assert-InstalledVersion $FromVersion
  Stop-LynxProcesses
  $oldHash = (Get-FileHash -LiteralPath (Join-Path $oldDirectory 'lynx-imager.exe') -Algorithm SHA256).Hash
  Set-ItemProperty -LiteralPath $uninstallKey -Name DisplayVersion -Value '0.0.0-invalid'
  $deltaExit = Invoke-Installer $deltaInstaller[0].FullName @('/S') 'Run mismatched delta'
  Assert-True ($deltaExit -eq 20) "Delta precondition exit was $deltaExit, expected 20"
  Assert-True (Test-Path -LiteralPath $failureMarker -PathType Leaf) 'Delta failure marker is absent.'
  Assert-True ((Get-Content -LiteralPath $failureMarker -First 1).Trim() -eq 'PRECONDITION') `
    'Delta failure marker is not PRECONDITION.'
  Assert-True ((Get-InstallDirectory) -eq $oldDirectory) 'Failed delta changed the registered install directory.'
  Assert-True ((Get-FileHash -LiteralPath (Join-Path $oldDirectory 'lynx-imager.exe') -Algorithm SHA256).Hash -eq $oldHash) `
    'Failed delta changed the installed application.'

  # The signed full package is the mandatory recovery path after a delta failure.
  Stop-LynxProcesses
  Assert-True ((Invoke-Installer $fullInstaller[0].FullName @('/S', '/UPDATE') 'Install full fallback') -eq 0) `
    'Full installer fallback failed.'
  $fallbackDirectory = Assert-InstalledVersion $ToVersion
  Assert-True ($fallbackDirectory -ne $oldDirectory) 'Full fallback reused the previous program directory.'
  Stop-LynxProcesses
  Assert-True ((Invoke-Installer (Join-Path $fallbackDirectory 'uninstall.exe') @('/S') 'Uninstall full fallback') -eq 0) `
    'Full fallback uninstall failed.'
  Wait-PathAbsent (Join-Path $fallbackDirectory 'lynx-imager.exe') 'Full fallback uninstall'

  # Success path: reinstall the exact public base and execute the Draft delta.
  Reset-LynxInstall
  Assert-True ((Invoke-Installer $oldInstaller @('/S', '/UPDATE') 'Install public base for delta path') -eq 0) `
    "Failed to reinstall LYNX $FromVersion"
  $oldDirectory = Assert-InstalledVersion $FromVersion
  Stop-LynxProcesses
  Assert-True ((Invoke-Installer $deltaInstaller[0].FullName @('/S') 'Install exact-version delta') -eq 0) `
    'Exact-version delta installation failed.'
  Start-Sleep -Seconds 5
  $deltaDirectory = Assert-InstalledVersion $ToVersion
  Assert-True ($deltaDirectory -ne $oldDirectory) 'Delta reused the previous program directory.'
  Assert-True (-not (Test-Path -LiteralPath (Join-Path $oldDirectory 'lynx-imager.exe'))) `
    'Delta left the previous application executable behind.'
  Assert-True (-not (Test-Path -LiteralPath $failureMarker)) 'Successful delta left a failure marker.'
  Stop-LynxProcesses
  Assert-True ((Invoke-Installer (Join-Path $deltaDirectory 'uninstall.exe') @('/S') 'Uninstall delta result') -eq 0) `
    'Delta-installed version uninstall failed.'
  Wait-PathAbsent (Join-Path $deltaDirectory 'lynx-imager.exe') 'Delta-result uninstall'

  Write-Host "Windows delta acceptance passed: $FromVersion -> $ToVersion"
} finally {
  Reset-LynxInstall
}
