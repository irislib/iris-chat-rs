; NSIS installer for Iris Chat (Windows x86_64).
; Driven by scripts/windows-build (windows-installer command).
; Variables injected via /D... at compile time:
;   /DIRIS_VERSION=2.6.26       Marketing version (no leading v)
;   /DIRIS_BUILD_NUM=30         Numeric build counter
;   /DIRIS_PUBLISH_DIR=...      Path to dotnet publish output (source files)
;   /DIRIS_OUTPUT=...           Output installer .exe path
;   /DIRIS_EXE_NAME=IrisChat.exe Main executable name in publish dir

!ifndef IRIS_VERSION
  !define IRIS_VERSION "0.0.0"
!endif
!ifndef IRIS_BUILD_NUM
  !define IRIS_BUILD_NUM "0"
!endif
!ifndef IRIS_EXE_NAME
  !define IRIS_EXE_NAME "IrisChat.exe"
!endif
!ifndef IRIS_OUTPUT
  !define IRIS_OUTPUT "IrisChat-setup.exe"
!endif
!ifndef IRIS_PUBLISH_DIR
  !error "IRIS_PUBLISH_DIR must be defined via /DIRIS_PUBLISH_DIR=..."
!endif

!define APP_NAME       "Iris Chat"
!define APP_PUBLISHER  "Iris"
!define APP_URL        "https://iris.to"
!define APP_REGKEY     "Software\Microsoft\Windows\CurrentVersion\Uninstall\IrisChat"

Name "${APP_NAME} ${IRIS_VERSION}"
OutFile "${IRIS_OUTPUT}"
Unicode true
SetCompressor /SOLID lzma
RequestExecutionLevel admin
ShowInstDetails show
ShowUninstDetails show

InstallDir "$PROGRAMFILES64\Iris Chat"
InstallDirRegKey HKLM "Software\IrisChat" "InstallDir"

VIProductVersion "${IRIS_VERSION}.${IRIS_BUILD_NUM}"
VIAddVersionKey "ProductName"     "${APP_NAME}"
VIAddVersionKey "CompanyName"     "${APP_PUBLISHER}"
VIAddVersionKey "FileDescription" "${APP_NAME} installer"
VIAddVersionKey "FileVersion"     "${IRIS_VERSION}.${IRIS_BUILD_NUM}"
VIAddVersionKey "ProductVersion"  "${IRIS_VERSION}"
VIAddVersionKey "LegalCopyright"  ""

!include "MUI2.nsh"

!define MUI_ABORTWARNING
!ifdef IRIS_ICON_PATH
  !define MUI_ICON   "${IRIS_ICON_PATH}"
  !define MUI_UNICON "${IRIS_ICON_PATH}"
!endif

!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!define MUI_FINISHPAGE_RUN "$INSTDIR\${IRIS_EXE_NAME}"
!insertmacro MUI_PAGE_FINISH

!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES

!insertmacro MUI_LANGUAGE "English"

Section "Iris Chat" SecMain
  SectionIn RO
  SetOutPath "$INSTDIR"
  SetOverwrite on

  File /r "${IRIS_PUBLISH_DIR}\*.*"

  WriteUninstaller "$INSTDIR\Uninstall.exe"

  CreateDirectory "$SMPROGRAMS\Iris Chat"
  CreateShortCut  "$SMPROGRAMS\Iris Chat\Iris Chat.lnk"   "$INSTDIR\${IRIS_EXE_NAME}"
  CreateShortCut  "$SMPROGRAMS\Iris Chat\Uninstall.lnk"   "$INSTDIR\Uninstall.exe"
  CreateShortCut  "$DESKTOP\Iris Chat.lnk"                "$INSTDIR\${IRIS_EXE_NAME}"

  WriteRegStr HKLM "Software\IrisChat" "InstallDir" "$INSTDIR"
  WriteRegStr HKLM "Software\IrisChat" "Version"    "${IRIS_VERSION}"

  WriteRegStr   HKLM "${APP_REGKEY}" "DisplayName"          "${APP_NAME}"
  WriteRegStr   HKLM "${APP_REGKEY}" "DisplayVersion"       "${IRIS_VERSION}"
  WriteRegStr   HKLM "${APP_REGKEY}" "Publisher"            "${APP_PUBLISHER}"
  WriteRegStr   HKLM "${APP_REGKEY}" "URLInfoAbout"         "${APP_URL}"
  WriteRegStr   HKLM "${APP_REGKEY}" "InstallLocation"      "$INSTDIR"
  WriteRegStr   HKLM "${APP_REGKEY}" "DisplayIcon"          "$INSTDIR\${IRIS_EXE_NAME}"
  WriteRegStr   HKLM "${APP_REGKEY}" "UninstallString"      "$\"$INSTDIR\Uninstall.exe$\""
  WriteRegStr   HKLM "${APP_REGKEY}" "QuietUninstallString" "$\"$INSTDIR\Uninstall.exe$\" /S"
  WriteRegDWORD HKLM "${APP_REGKEY}" "NoModify" 1
  WriteRegDWORD HKLM "${APP_REGKEY}" "NoRepair" 1
SectionEnd

Section "Uninstall"
  Delete "$DESKTOP\Iris Chat.lnk"
  Delete "$SMPROGRAMS\Iris Chat\Iris Chat.lnk"
  Delete "$SMPROGRAMS\Iris Chat\Uninstall.lnk"
  RMDir  "$SMPROGRAMS\Iris Chat"

  RMDir /r "$INSTDIR"

  DeleteRegKey HKLM "Software\IrisChat"
  DeleteRegKey HKLM "${APP_REGKEY}"
SectionEnd
