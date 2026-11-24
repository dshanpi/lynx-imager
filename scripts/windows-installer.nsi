!include "${__FILEDIR__}\imager-process-guard.nsh"
Unicode true

!include "MUI2.nsh"

!ifndef SOURCE_DIR
  !define SOURCE_DIR "..\src-tauri\target\x86_64-pc-windows-gnu\release"
!endif

!ifndef OUTPUT_FILE
  !define OUTPUT_FILE "..\release\windows\LYNX_0.1.0-rc.9_x64-setup.exe"
!endif

!ifndef PRODUCT_NAME
  !define PRODUCT_NAME "LYNX Imager"
!endif
!ifndef PRODUCT_BRAND
  !define PRODUCT_BRAND "DshanPI LYNX Imager"
!endif
!ifndef PRODUCT_VERSION
  !define PRODUCT_VERSION "0.1.0-rc.9"
!endif
!ifndef PRODUCT_FILE_VERSION
  !define PRODUCT_FILE_VERSION "0.1.0.0"
!endif
!ifndef PRODUCT_PUBLISHER
  !define PRODUCT_PUBLISHER "DshanPI"
!endif
!define PRODUCT_REG_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\DshanPI.LYNXImager"

Var PreviousInstallDir

Name "${PRODUCT_BRAND}"
OutFile "${OUTPUT_FILE}"
; .onInit replaces this base with a unique per-install program directory.
; Program files are deliberately separate from Tauri app data and the keyring.
InstallDir "$LOCALAPPDATA\DshanPI\LYNX Imager"
RequestExecutionLevel user
SetCompressor /SOLID lzma
Icon "..\src-tauri\icons\icon.ico"
UninstallIcon "..\src-tauri\icons\icon.ico"

VIProductVersion "${PRODUCT_FILE_VERSION}"
VIAddVersionKey /LANG=1033 "ProductName" "${PRODUCT_NAME}"
VIAddVersionKey /LANG=1033 "FileDescription" "${PRODUCT_BRAND} Installer"
VIAddVersionKey /LANG=1033 "CompanyName" "${PRODUCT_PUBLISHER}"
VIAddVersionKey /LANG=1033 "LegalCopyright" "Copyright DshanPI"
VIAddVersionKey /LANG=1033 "FileVersion" "${PRODUCT_VERSION}"
VIAddVersionKey /LANG=1033 "ProductVersion" "${PRODUCT_VERSION}"

!define MUI_ABORTWARNING
!define MUI_ICON "..\src-tauri\icons\icon.ico"
!define MUI_UNICON "..\src-tauri\icons\icon.ico"
!define MUI_FINISHPAGE_RUN "$INSTDIR\lynx-imager.exe"
!define MUI_FINISHPAGE_RUN_TEXT "启动 LYNX Imager"
!define MUI_FINISHPAGE_RUN_NOTCHECKED false

!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH

!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES

!insertmacro MUI_LANGUAGE "SimpChinese"
!insertmacro MUI_LANGUAGE "English"

Function .onInit
  SetShellVarContext current
  ReadRegStr $PreviousInstallDir HKCU "Software\DshanPI\LYNX Imager" "InstallDir"
  ; A fresh directory makes upgrades and same-version reinstalls independent of
  ; an old vendor flasher stuck in an uninterruptible USB driver call.
  System::Call 'kernel32::GetTickCount() i .r0'
  StrCpy $INSTDIR "$LOCALAPPDATA\DshanPI\LYNX Imager\app-${PRODUCT_VERSION}-$0"
FunctionEnd

Function TryRemovePreviousInstall
  StrCmp $PreviousInstallDir "" previous_cleanup_done
  StrCmp $PreviousInstallDir "$INSTDIR" previous_cleanup_done
  ; Only a versioned Imager instance is eligible for cleanup.
  ; /REBOOTOK means a locked old flasher delays cleanup but never installation.
  StrCpy $1 "$LOCALAPPDATA\DshanPI\LYNX Imager\app-"
  StrLen $2 $1
  StrCpy $3 $PreviousInstallDir $2
  StrCmp $3 $1 previous_cleanup_allowed previous_cleanup_done
  previous_cleanup_allowed:
    RMDir /r /REBOOTOK "$PreviousInstallDir"
  previous_cleanup_done:
FunctionEnd

Section "LYNX" SEC_MAIN
  SetShellVarContext current

  !insertmacro ImagerRequireIdle
  SetOutPath "$INSTDIR"

  File "${SOURCE_DIR}\lynx-imager.exe"
  File "${SOURCE_DIR}\WebView2Loader.dll"
  File "${SOURCE_DIR}\libusb-1.0.dll"
  File "${SOURCE_DIR}\CH347DLLA64.dll"
  File "${SOURCE_DIR}\MicrosoftEdgeWebview2Setup.exe"

  DetailPrint "Installing Microsoft Edge WebView2 Runtime..."
  ExecWait '"$INSTDIR\MicrosoftEdgeWebview2Setup.exe" /silent /install'
  Delete "$INSTDIR\MicrosoftEdgeWebview2Setup.exe"

  SetOutPath "$INSTDIR\openocd"
  File /r "${SOURCE_DIR}\openocd\*"

  SetOutPath "$INSTDIR\bin"
  File /r "${SOURCE_DIR}\bin\*"

  ; Fail installation instead of silently retaining a stale executor.
  nsExec::ExecToStack '"$INSTDIR\bin\openixcli.exe" --version'
  Pop $0
  Pop $1
  StrCmp $0 "0" openixcli_version_ok
    MessageBox MB_ICONSTOP "openixcli.exe 部署自检失败（退出码 $0）。"
    Abort
  openixcli_version_ok:
  FileOpen $2 "$INSTDIR\bin\openixcli-version.txt" w
  FileWrite $2 "$1$\r$\nexpected: openixcli 0.2.11$\r$\nraw-disk: command-sector0-v2$\r$\nfes-reconnect: constrained-topology-migration-v2$\r$\ngeneric-fes-id: any-fel-to-00161000-v1$\r$\nfes-post-action: confirmed-by-exit-v1$\r$\nstorage-geometry: command-env-v2$\r$\nraw-write: 10m-window-libefex-v1$\r$\n"
  FileClose $2

  SetOutPath "$INSTDIR"
  WriteUninstaller "$INSTDIR\Uninstall.exe"

  CreateDirectory "$SMPROGRAMS\DshanPI"
  CreateShortcut "$SMPROGRAMS\DshanPI\LYNX Imager.lnk" "$INSTDIR\lynx-imager.exe"
  CreateShortcut "$DESKTOP\LYNX Imager.lnk" "$INSTDIR\lynx-imager.exe"

  WriteRegStr HKCU "Software\DshanPI\LYNX Imager" "InstallDir" "$INSTDIR"
  WriteRegStr HKCU "${PRODUCT_REG_KEY}" "DisplayName" "${PRODUCT_BRAND}"
  WriteRegStr HKCU "${PRODUCT_REG_KEY}" "DisplayVersion" "${PRODUCT_VERSION}"
  WriteRegStr HKCU "${PRODUCT_REG_KEY}" "Publisher" "${PRODUCT_PUBLISHER}"
  WriteRegStr HKCU "${PRODUCT_REG_KEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "${PRODUCT_REG_KEY}" "DisplayIcon" "$INSTDIR\lynx-imager.exe"
  WriteRegStr HKCU "${PRODUCT_REG_KEY}" "UninstallString" "$\"$INSTDIR\Uninstall.exe$\""
  WriteRegDWORD HKCU "${PRODUCT_REG_KEY}" "NoModify" 1
  WriteRegDWORD HKCU "${PRODUCT_REG_KEY}" "NoRepair" 1
  WriteRegStr HKCU "Software\DshanPI\LYNX Imager" "PreviousInstallDir" "$PreviousInstallDir"

  Call TryRemovePreviousInstall
SectionEnd

Section "Uninstall"
  !insertmacro ImagerRequireIdle
  SetShellVarContext current
  ; A manually launched stale uninstaller may remove only its own instance. It
  ; must not delete shortcuts or protocol registration belonging to a newer one.
  ReadRegStr $0 HKCU "Software\DshanPI\LYNX Imager" "InstallDir"
  StrCmp $0 "$INSTDIR" uninstall_current uninstall_instance
  uninstall_current:
    Delete "$DESKTOP\LYNX Imager.lnk"
    Delete "$SMPROGRAMS\DshanPI\LYNX Imager.lnk"
    RMDir "$SMPROGRAMS\DshanPI"
    DeleteRegKey HKCU "${PRODUCT_REG_KEY}"
    DeleteRegKey HKCU "Software\DshanPI\LYNX Imager"
  uninstall_instance:
  RMDir /r "$INSTDIR"
SectionEnd
