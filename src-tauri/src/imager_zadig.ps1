$ErrorActionPreference = 'Stop'
$env:PSModulePath = [IO.Path]::Combine($PSHOME, 'Modules')
$exe = $env:LYNX_DRIVER_PATH
$signature = Get-AuthenticodeSignature -LiteralPath $exe
if ($signature.Status -ne 'Valid' -or $signature.SignerCertificate.GetNameInfo('SimpleName', $false) -ne 'Akeo Consulting') { exit 2 }
try {
    $installer = Start-Process -FilePath $exe -WorkingDirectory ([Environment]::SystemDirectory) -Verb RunAs -Wait -PassThru
    if ($installer.ExitCode -ne 0) { exit 1 }
} catch {
    if ($_.Exception.GetBaseException().NativeErrorCode -eq 1223) { exit 3 }
    exit 1
}
exit 0
