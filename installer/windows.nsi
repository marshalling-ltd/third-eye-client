; Third Eye Client — Windows installer
; Built by the GitHub Actions release workflow.
; Run from the repository root: makensis installer\windows.nsi

Unicode true

!define APP_NAME      "Third Eye Client"
!define APP_EXE       "third-eye-client.exe"
!define INSTALL_DIR   "$PROGRAMFILES64\Third Eye Client"
!define REG_KEY       "Software\Microsoft\Windows\CurrentVersion\Uninstall\Third Eye Client"

Name            "${APP_NAME}"
OutFile         "Third.Eye.Client.windows-x64-setup.exe"
InstallDir      "${INSTALL_DIR}"
InstallDirRegKey HKLM "${REG_KEY}" "InstallLocation"
RequestExecutionLevel admin
SetCompressor   /SOLID lzma
ShowInstDetails show

!include "MUI2.nsh"

; Pages
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!define MUI_FINISHPAGE_RUN          "$INSTDIR\${APP_EXE}"
!define MUI_FINISHPAGE_RUN_TEXT     "Launch ${APP_NAME}"
!insertmacro MUI_PAGE_FINISH

!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES

!insertmacro MUI_LANGUAGE "English"

; ---------------------------------------------------------------------------
; Install
; ---------------------------------------------------------------------------
Section "Install"
  ; Kill any running instance so files can be overwritten.
  ExecWait 'taskkill /F /IM "${APP_EXE}"' $0

  SetOutPath "$INSTDIR"
  File "package\${APP_EXE}"

  SetOutPath "$INSTDIR\bin"
  File "package\bin\ffmpeg.exe"

  ; Write uninstaller.
  SetOutPath "$INSTDIR"
  WriteUninstaller "$INSTDIR\Uninstall.exe"

  ; Start Menu shortcut.
  CreateDirectory "$SMPROGRAMS\${APP_NAME}"
  CreateShortcut  "$SMPROGRAMS\${APP_NAME}\${APP_NAME}.lnk" "$INSTDIR\${APP_EXE}"
  CreateShortcut  "$SMPROGRAMS\${APP_NAME}\Uninstall.lnk"   "$INSTDIR\Uninstall.exe"

  ; Desktop shortcut.
  CreateShortcut "$DESKTOP\${APP_NAME}.lnk" "$INSTDIR\${APP_EXE}"

  ; Registry — add/update entry (enables "Programs and Features" uninstall).
  WriteRegStr   HKLM "${REG_KEY}" "DisplayName"     "${APP_NAME}"
  WriteRegStr   HKLM "${REG_KEY}" "UninstallString"  '"$INSTDIR\Uninstall.exe"'
  WriteRegStr   HKLM "${REG_KEY}" "InstallLocation"  "$INSTDIR"
  WriteRegStr   HKLM "${REG_KEY}" "Publisher"        "Marshalling"
  WriteRegDWORD HKLM "${REG_KEY}" "NoModify"         1
  WriteRegDWORD HKLM "${REG_KEY}" "NoRepair"         1
SectionEnd

; ---------------------------------------------------------------------------
; Uninstall
; ---------------------------------------------------------------------------
Section "Uninstall"
  ExecWait 'taskkill /F /IM "${APP_EXE}"' $0

  Delete "$INSTDIR\${APP_EXE}"
  Delete "$INSTDIR\bin\ffmpeg.exe"
  Delete "$INSTDIR\Uninstall.exe"
  RMDir  "$INSTDIR\bin"
  RMDir  "$INSTDIR"

  Delete "$SMPROGRAMS\${APP_NAME}\${APP_NAME}.lnk"
  Delete "$SMPROGRAMS\${APP_NAME}\Uninstall.lnk"
  RMDir  "$SMPROGRAMS\${APP_NAME}"

  Delete "$DESKTOP\${APP_NAME}.lnk"

  DeleteRegKey HKLM "${REG_KEY}"
SectionEnd
