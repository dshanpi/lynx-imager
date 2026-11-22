# Return 0 when idle, 2 for a running Imager task, or 3 if inspection failed.
$ErrorActionPreference = 'Stop'
try {
  $root = [IO.Path]::GetFullPath((Join-Path $env:LOCALAPPDATA 'DshanPI\LYNX Imager')) + '\'
  $filter = "Name='lynx-imager.exe' OR Name='openixcli.exe' OR Name='openrockcli.exe' OR Name='uuu.exe' OR Name='dfu-util.exe'"
  $userSid = [Security.Principal.WindowsIdentity]::GetCurrent().User.Value
  foreach ($process in (Get-CimInstance Win32_Process -Filter $filter)) {
    if ($process.ExecutablePath -and $process.ExecutablePath.StartsWith($root, [StringComparison]::OrdinalIgnoreCase)) { exit 2 }
    if ($process.Name -eq 'lynx-imager.exe' -or !$process.ExecutablePath) {
      $owner = Invoke-CimMethod -InputObject $process -MethodName GetOwnerSid
      if ($owner.ReturnValue -ne 0) { exit 3 }
      if ($owner.Sid -eq $userSid) {
        if ($process.Name -eq 'lynx-imager.exe') { exit 2 }
        exit 3
      }
    }
  }
  exit 0
} catch {
  exit 3
}
