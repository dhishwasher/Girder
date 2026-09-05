Unicode true

!include "LogicLib.nsh"
!include "StrFunc.nsh"
!include "WinMessages.nsh"

${StrStr}
${UnStrRep}

!ifndef VERSION
  !error "VERSION must be provided to makensis"
!endif
!ifndef BINARY_PATH
  !error "BINARY_PATH must be provided to makensis"
!endif
!ifndef ICON_PATH
  !error "ICON_PATH must be provided to makensis"
!endif
!ifndef LICENSE_PATH
  !error "LICENSE_PATH must be provided to makensis"
!endif
!ifndef FONT_LICENSE_PATH
  !error "FONT_LICENSE_PATH must be provided to makensis"
!endif
!ifndef OUTPUT_PATH
  !error "OUTPUT_PATH must be provided to makensis"
!endif

!define APP_REG_KEY "Software\Girder"
!define UNINSTALL_REG_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\Girder"

Name "Girder ${VERSION}"
OutFile "${OUTPUT_PATH}"
InstallDir "$LOCALAPPDATA\Programs\Girder"
InstallDirRegKey HKCU "${APP_REG_KEY}" "InstallDir"
RequestExecutionLevel user
SetCompressor /SOLID lzma
Icon "${ICON_PATH}"
UninstallIcon "${ICON_PATH}"

Page directory
Page instfiles
UninstPage uninstConfirm
UninstPage instfiles

Function AddGirderToUserPath
  ReadRegStr $0 HKCU "Environment" "Path"
  StrCpy $1 ";$0;"
  StrCpy $2 ";$INSTDIR;"
  ${StrStr} $3 $1 $2
  StrCmp $3 "" 0 path_done
  StrCmp $0 "" path_empty path_append

  path_empty:
    StrCpy $0 "$INSTDIR"
    Goto path_write

  path_append:
    StrCpy $0 "$0;$INSTDIR"

  path_write:
    WriteRegExpandStr HKCU "Environment" "Path" "$0"
    SendMessage ${HWND_BROADCAST} ${WM_SETTINGCHANGE} 0 "STR:Environment" /TIMEOUT=5000

  path_done:
FunctionEnd

Function un.RemoveGirderFromUserPath
  ReadRegStr $0 HKCU "Environment" "Path"
  StrCpy $1 ";$0;"
  StrCpy $2 ";$INSTDIR;"
  ${UnStrRep} $1 $1 $2 ";"
  StrCmp $1 ";" path_empty

  StrCpy $0 $1 "" 1
  StrLen $2 $0
  IntOp $2 $2 - 1
  StrCpy $0 $0 $2
  Goto path_write

  path_empty:
    StrCpy $0 ""

  path_write:
    WriteRegExpandStr HKCU "Environment" "Path" "$0"
    SendMessage ${HWND_BROADCAST} ${WM_SETTINGCHANGE} 0 "STR:Environment" /TIMEOUT=5000
FunctionEnd

Section "Girder" SEC_GIRDER
  SetOutPath "$INSTDIR"
  File "/oname=girder.exe" "${BINARY_PATH}"
  File "/oname=icon.ico" "${ICON_PATH}"
  File "/oname=LICENSE" "${LICENSE_PATH}"
  File "/oname=DejaVuSansMono-LICENSE.txt" "${FONT_LICENSE_PATH}"

  WriteRegStr HKCU "${APP_REG_KEY}" "InstallDir" "$INSTDIR"
  WriteUninstaller "$INSTDIR\Uninstall.exe"

  CreateDirectory "$SMPROGRAMS\Girder"
  CreateShortcut "$SMPROGRAMS\Girder\Girder.lnk" "$INSTDIR\girder.exe" "--gui" "$INSTDIR\icon.ico"

  WriteRegStr HKCU "${UNINSTALL_REG_KEY}" "DisplayName" "Girder"
  WriteRegStr HKCU "${UNINSTALL_REG_KEY}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "${UNINSTALL_REG_KEY}" "Publisher" "Cory Maynard"
  WriteRegStr HKCU "${UNINSTALL_REG_KEY}" "DisplayIcon" "$INSTDIR\icon.ico"
  WriteRegStr HKCU "${UNINSTALL_REG_KEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "${UNINSTALL_REG_KEY}" "UninstallString" "$\"$INSTDIR\Uninstall.exe$\""
  WriteRegStr HKCU "${UNINSTALL_REG_KEY}" "QuietUninstallString" "$\"$INSTDIR\Uninstall.exe$\" /S"
  WriteRegDWORD HKCU "${UNINSTALL_REG_KEY}" "NoModify" 1
  WriteRegDWORD HKCU "${UNINSTALL_REG_KEY}" "NoRepair" 1

  Call AddGirderToUserPath
SectionEnd

Section "Uninstall"
  Call un.RemoveGirderFromUserPath

  Delete "$SMPROGRAMS\Girder\Girder.lnk"
  RMDir "$SMPROGRAMS\Girder"

  Delete "$INSTDIR\girder.exe"
  Delete "$INSTDIR\icon.ico"
  Delete "$INSTDIR\LICENSE"
  Delete "$INSTDIR\DejaVuSansMono-LICENSE.txt"
  Delete "$INSTDIR\Uninstall.exe"
  RMDir "$INSTDIR"

  DeleteRegKey HKCU "${UNINSTALL_REG_KEY}"
  DeleteRegKey HKCU "${APP_REG_KEY}"
SectionEnd
