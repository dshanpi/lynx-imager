$env:PSModulePath = Join-Path $PSHOME 'Modules'
$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [Text.UTF8Encoding]::new($false)
$disks = @(Get-Disk -ErrorAction Stop)
$drives = @(Get-CimInstance Win32_DiskDrive)
$cards = @(foreach ($disk in $disks) {
  if ($disk.IsBoot -or $disk.IsSystem -or $disk.IsReadOnly -or $disk.IsOffline -or $disk.Size -le 0) { continue }
  $drive = $drives | Where-Object { $_.Index -eq $disk.Number } | Select-Object -First 1
  if (!$drive -or !$drive.PNPDeviceID) { continue }
  if ($disk.BusType -ne 'SD' -and !($disk.BusType -eq 'USB' -and ($drive.MediaType -eq 'Removable Media' -or $drive.Capabilities -contains 7))) { continue }
  $parts = @()
  if ($disk.NumberOfPartitions -gt 0) { $parts = @(Get-Partition -DiskNumber $disk.Number) }
  if ($parts | Where-Object { $_.IsBoot -or $_.IsSystem }) { continue }
  $volumes = @($parts | ForEach-Object { $_.AccessPaths } | Where-Object { $_ -and $_.StartsWith('\\?\Volume{') } | Select-Object -Unique)
  [pscustomobject]@{ key=[int]$disk.Number; name=$disk.FriendlyName; bytes=[uint64]$disk.Size; sector=[int]$disk.LogicalSectorSize; identity=($drive.PNPDeviceID+'|'+$disk.SerialNumber+'|'+$disk.UniqueId); volumes=$volumes }
})
ConvertTo-Json -InputObject $cards -Compress -Depth 4
