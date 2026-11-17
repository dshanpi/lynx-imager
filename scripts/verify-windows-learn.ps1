# Software checks only; no device discovery, installation, flashing, or resets.
$ErrorActionPreference = 'Stop'
$repoRoot = Split-Path -Parent $PSScriptRoot
Set-Location $repoRoot
if (-not $env:OS -or $env:OS -ne 'Windows_NT') { throw 'Run on the target Windows x64 machine.' }
foreach ($tool in @('node','npm.cmd','cargo','rustc','git')) {
  if (-not (Get-Command $tool -ErrorAction SilentlyContinue)) { throw "Missing required tool: $tool" }
}
$evidenceDir = Join-Path $repoRoot ('release/windows-learn-check-' + (Get-Date -Format 'yyyyMMdd-HHmmss'))
New-Item -ItemType Directory -Force $evidenceDir | Out-Null
function Invoke-Checked([string]$Program, [string[]]$Arguments) {
  & $Program @Arguments
  if ($LASTEXITCODE -ne 0) { throw "$Program failed with exit code $LASTEXITCODE" }
}
Start-Transcript -Path (Join-Path $evidenceDir 'software-checks.log') | Out-Null
try {
  Invoke-Checked 'git' @('rev-parse','HEAD')
  Invoke-Checked 'node' @('--version')
  Invoke-Checked 'rustc' @('--version')
  Invoke-Checked 'npm.cmd' @('ci')
  Invoke-Checked 'npm.cmd' @('run','typecheck')
  Invoke-Checked 'npm.cmd' @('run','lint','--','--quiet')
  Invoke-Checked 'npm.cmd' @('run','check:ipc-contracts')
  Invoke-Checked 'npm.cmd' @('run','test:ui-contract')
  Invoke-Checked 'cargo' @('test','--manifest-path','src-tauri/Cargo.toml','--lib')
  Invoke-Checked 'npm.cmd' @('run','validate:package')
  Write-Output 'Software checks passed. Installation, UI, SSH, signing/updater and hardware remain NOT VERIFIED.'
} finally { Stop-Transcript | Out-Null }
