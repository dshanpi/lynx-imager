; Hooks used by the real Tauri NSIS bundle. Keep this file small so upstream
; WebView2, updater and resource packaging behavior remains in Tauri's tested
; installer template.

Var LynxPreviousInstallDir

; REQ-REL-042: never close a process without the runtime hardware safety gate.
; The installer cannot query that gate, so running processes block installation.
!macro LynxRejectRunning executableName
  nsis_tauri_utils::FindProcessCurrentUser "${executableName}"
  Pop $R0
  ; Plugin result 1 alone means absent. Unknown/error results fail closed.
  ${If} $R0 != 1
    SetErrorLevel 2
    IfSilent +2
      MessageBox MB_OK|MB_ICONEXCLAMATION "${executableName} is still running, or its state could not be checked. Wait for hardware tasks to finish, exit LYNX normally, then run the installer again. No process has been stopped."
    Abort
  ${EndIf}
!macroend

!macro LynxRequireIdleProcesses
  !insertmacro LynxRejectRunning "lynx-imager.exe"
  ; Keep the legacy process guard so a v0.1.0 process cannot keep using the
  ; old UI while a renamed executable is being installed beside it.
  !insertmacro LynxRejectRunning "lynx-app.exe"
  !insertmacro LynxRejectRunning "openixcli.exe"
  !insertmacro LynxRejectRunning "openrockcli.exe"
  !insertmacro LynxRejectRunning "uuu.exe"
  !insertmacro LynxRejectRunning "dfu-util.exe"
!macroend

; Replace Tauri's default force-close path for both install and uninstall.
!ifmacrodef CheckIfAppIsRunning
  !macroundef CheckIfAppIsRunning
!endif
!macro CheckIfAppIsRunning executableName productName
  !insertmacro LynxRejectRunning "${executableName}"
!macroend

; Tauri includes hooks before !addplugindir. Defer plugin commands until MUI
; emits .onGUIInit after plugin registration. Preserve MUI's standard dialog
; initialization and both optional callbacks (NSIS MUI2 Interface.nsh).
!macroundef MUI_FUNCTION_GUIINIT
!macro MUI_FUNCTION_GUIINIT
  Function .onGUIInit
    !insertmacro LynxRequireIdleProcesses
    !insertmacro MUI_GUIINIT_OUTERDIALOG ""
    !ifdef MUI_PAGE_FUNCTION_GUIINIT
      Call "${MUI_PAGE_FUNCTION_GUIINIT}"
    !endif
    !ifdef MUI_CUSTOMFUNCTION_GUIINIT
      Call "${MUI_CUSTOMFUNCTION_GUIINIT}"
    !endif
  FunctionEnd
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  !insertmacro LynxRequireIdleProcesses
!macroend

!macro NSIS_HOOK_PREINSTALL
  SetShellVarContext current
  ReadRegStr $LynxPreviousInstallDir SHCTX "${MANUPRODUCTKEY}" ""

  ; Also runs for silent installs, before any overwrite/cleanup.
  !insertmacro LynxRequireIdleProcesses

  ; Install into a fresh, versioned program directory. User data and keyring
  ; state live outside this tree and are intentionally preserved.
  System::Call 'kernel32::GetTickCount() i .r0'
  StrCpy $INSTDIR "$LOCALAPPDATA\DshanPI\LYNX\app-${VERSION}-$0"
  SetOutPath $INSTDIR
!macroend

!macro NSIS_HOOK_POSTINSTALL
  ; Remove only a path previously registered by LYNX. Locked legacy helpers
  ; are deferred with /REBOOTOK and cannot roll back the completed install.
  StrCmp $LynxPreviousInstallDir "" lynx_previous_cleanup_done
  StrCmp $LynxPreviousInstallDir "$INSTDIR" lynx_previous_cleanup_done
  StrCmp $LynxPreviousInstallDir "$LOCALAPPDATA\LYNX" lynx_previous_cleanup_allowed
  StrCpy $1 "$LOCALAPPDATA\DshanPI\LYNX\app-"
  StrLen $2 $1
  StrCpy $3 $LynxPreviousInstallDir $2
  StrCmp $3 $1 lynx_previous_cleanup_allowed lynx_previous_cleanup_done
  lynx_previous_cleanup_allowed:
    RMDir /r /REBOOTOK "$LynxPreviousInstallDir"
  lynx_previous_cleanup_done:
!macroend
