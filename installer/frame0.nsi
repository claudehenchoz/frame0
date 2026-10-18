; Frame Zero installer
; Copyright (C) 2026 Claude Henchoz
; SPDX-License-Identifier: GPL-3.0-or-later
;
; Build: makensis /DVERSION=1.2.3 installer\frame0.nsi
;
; Per-user install (no UAC prompt) into %LOCALAPPDATA%\Programs\Frame Zero.
; Pages: install directory (confirms the install), progress, and a finish page
; with "Run Frame Zero" and "Start Frame Zero with Windows" checkboxes.
; /S installs silently (no launch, no autostart), /D=<dir> overrides the path.
; A running tray instance is closed (frame0.exe --cmd 199) before files are
; replaced or removed.

!ifndef VERSION
  !define VERSION "0.0.0"
!endif
!define APP      "Frame Zero"
!define EXE      "frame0.exe"
!define UNINST   "Software\Microsoft\Windows\CurrentVersion\Uninstall\frame0"
!define APPPATHS "Software\Microsoft\Windows\CurrentVersion\App Paths\${EXE}"
!define RUNKEY   "Software\Microsoft\Windows\CurrentVersion\Run"

Unicode true
SetCompressor /FINAL lzma
ManifestDPIAware true
ManifestSupportedOS Win7 Win8 Win8.1 Win10
RequestExecutionLevel user
XPStyle on

Name "${APP}"
Caption "${APP} ${VERSION} Setup"
OutFile "frame0-${VERSION}-setup.exe"
InstallDir "$LOCALAPPDATA\Programs\${APP}"
BrandingText " "
ShowInstDetails nevershow
ShowUninstDetails nevershow

!include "MUI2.nsh"
!define MUI_ICON   "..\res\installer.ico"
!define MUI_UNICON "..\res\installer.ico"
!define MUI_ABORTWARNING

!define MUI_DIRECTORYPAGE_TEXT_TOP "Setup will install ${APP} ${VERSION} in the following folder. To install in a different folder, click Browse. Click Install to start the installation."
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!define MUI_FINISHPAGE_RUN "$INSTDIR\${EXE}"
!define MUI_FINISHPAGE_RUN_TEXT "Run ${APP}"
; The "show readme" checkbox doubles as the autostart option.
!define MUI_FINISHPAGE_SHOWREADME ""
!define MUI_FINISHPAGE_SHOWREADME_TEXT "Start ${APP} with Windows"
!define MUI_FINISHPAGE_SHOWREADME_FUNCTION EnableAutostart
!insertmacro MUI_PAGE_FINISH

!insertmacro MUI_UNPAGE_INSTFILES

!insertmacro MUI_LANGUAGE "English"

Function EnableAutostart
  WriteRegStr HKCU "${RUNKEY}" "${APP}" '"$INSTDIR\${EXE}"'
FunctionEnd

; Ask a running tray instance to exit so its exe can be replaced / deleted.
!macro CloseRunning
  IfFileExists "$INSTDIR\${EXE}" 0 +3
    ExecWait '"$INSTDIR\${EXE}" --cmd 199'
    Sleep 1000
!macroend

Section
  SetOutPath "$INSTDIR"
  !insertmacro CloseRunning
  File "..\target\x86_64-pc-windows-msvc\release\${EXE}"
  File "..\LICENSE"
  WriteUninstaller "$INSTDIR\uninstall.exe"

  ; Start > Run / Win+R "frame0" resolves the exe via App Paths.
  WriteRegStr HKCU "${APPPATHS}" "" "$INSTDIR\${EXE}"
  WriteRegStr HKCU "${APPPATHS}" "Path" "$INSTDIR"

  CreateShortcut "$SMPROGRAMS\${APP}.lnk" "$INSTDIR\${EXE}"

  ; Apps & features entry.
  WriteRegStr HKCU "${UNINST}" "DisplayName" "${APP}"
  WriteRegStr HKCU "${UNINST}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "${UNINST}" "Publisher" "Claude Henchoz"
  WriteRegStr HKCU "${UNINST}" "DisplayIcon" "$INSTDIR\${EXE},0"
  WriteRegStr HKCU "${UNINST}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "${UNINST}" "URLInfoAbout" "https://github.com/claudehenchoz/frame0"
  WriteRegStr HKCU "${UNINST}" "UninstallString" '"$INSTDIR\uninstall.exe"'
  WriteRegStr HKCU "${UNINST}" "QuietUninstallString" '"$INSTDIR\uninstall.exe" /S'
  WriteRegDWORD HKCU "${UNINST}" "NoModify" 1
  WriteRegDWORD HKCU "${UNINST}" "NoRepair" 1
  WriteRegDWORD HKCU "${UNINST}" "EstimatedSize" 400
SectionEnd

Section "Uninstall"
  !insertmacro CloseRunning
  Delete "$INSTDIR\${EXE}"
  Delete "$INSTDIR\LICENSE"
  Delete "$INSTDIR\uninstall.exe"
  RMDir "$INSTDIR"
  Delete "$SMPROGRAMS\${APP}.lnk"

  DeleteRegKey HKCU "${APPPATHS}"
  DeleteRegValue HKCU "${RUNKEY}" "${APP}"
  DeleteRegKey HKCU "${UNINST}"
  ; Settings and user stamps in %APPDATA%\frame0 are left in place.
SectionEnd
