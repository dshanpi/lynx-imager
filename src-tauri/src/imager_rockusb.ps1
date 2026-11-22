$ErrorActionPreference = 'Stop'
$env:PSModulePath = [IO.Path]::Combine($PSHOME, 'Modules')
try {
    $inf = $env:LYNX_DRIVER_PATH
    $tool = Join-Path ([Environment]::SystemDirectory) 'pnputil.exe'
    $process = Start-Process -FilePath $tool -WorkingDirectory ([Environment]::SystemDirectory) -ArgumentList @('/add-driver', ('"' + $inf + '"'), '/install') -Verb RunAs -Wait -PassThru
    if ($process.ExitCode -eq 3010) { exit 4 }
    if ($process.ExitCode -eq 0) { exit 5 }
} catch {
    if ($_.Exception.GetBaseException().NativeErrorCode -eq 1223) { exit 3 }
}
exit 1
