$ErrorActionPreference = 'Stop'
$env:PSModulePath = [IO.Path]::Combine($PSHOME, 'Modules')
try {
    $inf = $env:LYNX_DRIVER_PATH
    $tool = Join-Path ([Environment]::SystemDirectory) 'pnputil.exe'
    $process = Start-Process -FilePath $tool -WorkingDirectory ([Environment]::SystemDirectory) -ArgumentList @('/add-driver', ('"' + $inf + '"'), '/install') -Verb RunAs -Wait -PassThru
    if ($process.ExitCode -eq 3010) { exit 4 }
    if ($process.ExitCode -eq 0) {
        if ($env:LYNX_DRIVER_KIND -ne 'FEL') { exit 5 }
        try {
            $devices = @(Get-CimInstance Win32_PnPEntity -Filter "Present = TRUE" |
                Where-Object { $_.PNPDeviceID -like 'USB\VID_1F3A&PID_EFE8\*' })
            if ($devices.Count -eq 0) { exit 7 }
            if (@($devices | Where-Object { $_.Service -ne 'usbUDisc' -or $_.ConfigManagerErrorCode -ne 0 }).Count) { exit 8 }
            exit 6
        } catch { exit 8 }
    }
} catch {
    if ($_.Exception.GetBaseException().NativeErrorCode -eq 1223) { exit 3 }
}
exit 1
