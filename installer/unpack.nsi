; Unpack Windows 설치 프로그램 (NSIS)
; 빌드: makensis -DVERSION=0.2.0 -DBINDIR=<exe 폴더> -DOUTFILE=<출력 경로> installer/unpack.nsi

Unicode true
SetCompressor /SOLID lzma

!ifndef VERSION
  !define VERSION "0.0.0"
!endif
!ifndef BINDIR
  !define BINDIR "..\target\x86_64-pc-windows-gnu\release"
!endif
!ifndef OUTFILE
  !define OUTFILE "..\dist\UnpackSetup-${VERSION}-x64.exe"
!endif

!define APPNAME "Unpack"
!define PUBLISHER "opizen"
!define UNINST_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\${APPNAME}"
!define CTX_LABEL "Unpack으로 압축 풀기"

Name "${APPNAME} ${VERSION}"
OutFile "${OUTFILE}"
InstallDir "$PROGRAMFILES64\${APPNAME}"
InstallDirRegKey HKLM "Software\${APPNAME}" "InstallDir"
RequestExecutionLevel admin
BrandingText "${APPNAME} ${VERSION}"

VIProductVersion "${VERSION}.0"
VIAddVersionKey /LANG=1042 "ProductName" "${APPNAME}"
VIAddVersionKey /LANG=1042 "FileDescription" "${APPNAME} 설치 프로그램"
VIAddVersionKey /LANG=1042 "FileVersion" "${VERSION}"
VIAddVersionKey /LANG=1042 "ProductVersion" "${VERSION}"
VIAddVersionKey /LANG=1042 "CompanyName" "${PUBLISHER}"
VIAddVersionKey /LANG=1042 "LegalCopyright" "MIT License"

!include "MUI2.nsh"
!include "x64.nsh"

!define MUI_ICON "..\assets\unpack.ico"
!define MUI_UNICON "..\assets\unpack.ico"
!define MUI_ABORTWARNING
!define MUI_FINISHPAGE_RUN "$INSTDIR\unpack-gui.exe"
!define MUI_FINISHPAGE_RUN_TEXT "Unpack 실행"

!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_LICENSE "..\LICENSE"
!insertmacro MUI_PAGE_COMPONENTS
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH

!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES

!insertmacro MUI_LANGUAGE "Korean"

; 우클릭 메뉴 등록/해제 (Windows 11에서는 "추가 옵션 표시" 안에 나타남)
!macro AddContext EXT
  WriteRegStr HKLM "Software\Classes\SystemFileAssociations\.${EXT}\shell\Unpack" "" "${CTX_LABEL}"
  WriteRegStr HKLM "Software\Classes\SystemFileAssociations\.${EXT}\shell\Unpack" "Icon" "$INSTDIR\unpack-gui.exe,0"
  WriteRegStr HKLM "Software\Classes\SystemFileAssociations\.${EXT}\shell\Unpack\command" "" '"$INSTDIR\unpack-gui.exe" --auto "%1"'
!macroend
!macro RemoveContext EXT
  DeleteRegKey HKLM "Software\Classes\SystemFileAssociations\.${EXT}\shell\Unpack"
!macroend
!macro ForEachExt MACRO
  !insertmacro ${MACRO} "zip"
  !insertmacro ${MACRO} "7z"
  !insertmacro ${MACRO} "tar"
  !insertmacro ${MACRO} "gz"
  !insertmacro ${MACRO} "tgz"
  !insertmacro ${MACRO} "bz2"
  !insertmacro ${MACRO} "tbz2"
  !insertmacro ${MACRO} "xz"
  !insertmacro ${MACRO} "txz"
  !insertmacro ${MACRO} "zst"
  !insertmacro ${MACRO} "tzst"
!macroend

Function .onInit
  ${IfNot} ${RunningX64}
    MessageBox MB_ICONSTOP "Unpack은 64비트 Windows에서만 설치할 수 있습니다."
    Abort
  ${EndIf}
  SetRegView 64
FunctionEnd

Function un.onInit
  SetRegView 64
FunctionEnd

Section "Unpack (필수)" SecMain
  SectionIn RO
  SetOutPath "$INSTDIR"
  File "${BINDIR}\unpack-gui.exe"
  File "${BINDIR}\unpack.exe"
  File "..\README.md"
  File "..\LICENSE"

  CreateDirectory "$SMPROGRAMS\${APPNAME}"
  CreateShortcut "$SMPROGRAMS\${APPNAME}\Unpack.lnk" "$INSTDIR\unpack-gui.exe"
  CreateShortcut "$SMPROGRAMS\${APPNAME}\Unpack 제거.lnk" "$INSTDIR\uninstall.exe"

  WriteRegStr HKLM "Software\${APPNAME}" "InstallDir" "$INSTDIR"
  WriteUninstaller "$INSTDIR\uninstall.exe"
  WriteRegStr HKLM "${UNINST_KEY}" "DisplayName" "${APPNAME}"
  WriteRegStr HKLM "${UNINST_KEY}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKLM "${UNINST_KEY}" "Publisher" "${PUBLISHER}"
  WriteRegStr HKLM "${UNINST_KEY}" "DisplayIcon" "$INSTDIR\unpack-gui.exe"
  WriteRegStr HKLM "${UNINST_KEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKLM "${UNINST_KEY}" "UninstallString" '"$INSTDIR\uninstall.exe"'
  WriteRegStr HKLM "${UNINST_KEY}" "QuietUninstallString" '"$INSTDIR\uninstall.exe" /S'
  WriteRegDWORD HKLM "${UNINST_KEY}" "NoModify" 1
  WriteRegDWORD HKLM "${UNINST_KEY}" "NoRepair" 1
SectionEnd

Section "탐색기 우클릭 메뉴 ($\"${CTX_LABEL}$\")" SecContext
  !insertmacro ForEachExt AddContext
SectionEnd

Section "바탕화면 바로가기" SecDesktop
  CreateShortcut "$DESKTOP\Unpack.lnk" "$INSTDIR\unpack-gui.exe"
SectionEnd

!insertmacro MUI_FUNCTION_DESCRIPTION_BEGIN
  !insertmacro MUI_DESCRIPTION_TEXT ${SecMain} "Unpack 프로그램(GUI, 명령줄 unpack.exe)과 시작 메뉴 바로가기"
  !insertmacro MUI_DESCRIPTION_TEXT ${SecContext} "압축 파일을 마우스 오른쪽 버튼으로 눌러 바로 풀 수 있게 합니다"
  !insertmacro MUI_DESCRIPTION_TEXT ${SecDesktop} "바탕화면에 바로가기를 만듭니다"
!insertmacro MUI_FUNCTION_DESCRIPTION_END

Section "Uninstall"
  !insertmacro ForEachExt RemoveContext
  Delete "$DESKTOP\Unpack.lnk"
  RMDir /r "$SMPROGRAMS\${APPNAME}"
  Delete "$INSTDIR\unpack-gui.exe"
  Delete "$INSTDIR\unpack.exe"
  Delete "$INSTDIR\README.md"
  Delete "$INSTDIR\LICENSE"
  Delete "$INSTDIR\uninstall.exe"
  RMDir "$INSTDIR"
  DeleteRegKey HKLM "${UNINST_KEY}"
  DeleteRegKey HKLM "Software\${APPNAME}"
SectionEnd
