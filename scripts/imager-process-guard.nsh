!ifndef LYNX_IMAGER_PROCESS_GUARD
!define LYNX_IMAGER_PROCESS_GUARD
!define ImagerGuardScript "${__FILEDIR__}\imager-process-guard.ps1"
!macro ImagerRequireIdle
  InitPluginsDir
  File /oname=$PLUGINSDIR\imager-process-guard.ps1 "${ImagerGuardScript}"
  nsExec::ExecToStack '"$SYSDIR\WindowsPowerShell\v1.0\powershell.exe" -NoProfile -NonInteractive -ExecutionPolicy Bypass -File "$PLUGINSDIR\imager-process-guard.ps1"'
  Pop $R0
  Pop $R1
  StrCmp $R0 "0" +5
    SetErrorLevel 2
    IfSilent +2
      MessageBox MB_OK|MB_ICONEXCLAMATION "Wait for LYNX Imager hardware tasks to finish, then close LYNX Imager and retry. Process state must be available to continue."
    Abort
!macroend
!endif
