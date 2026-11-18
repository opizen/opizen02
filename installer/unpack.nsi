; opizenAI_Unpack Windows 설치 프로그램 (NSIS)
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
  !define OUTFILE "..\dist\opizenAI_Unpack-Setup-${VERSION}-x64.exe"
!endif

!define APPNAME "opizenAI_Unpack"
!define PUBLISHER "opizen"
!define UNINST_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\${APPNAME}"

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
!define MUI_FINISHPAGE_RUN "$INSTDIR\opizenAI_Unpack.exe"
!define MUI_FINISHPAGE_RUN_TEXT "opizenAI_Unpack 실행"

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
; 하위 메뉴(opizenAI_Unpack ▸)로 구성:
;   여기에 압축 풀기     → opizenAI_Unpack.exe --here   (스마트 해제, 작은 진행 창)
;   새 폴더에 압축 풀기  → opizenAI_Unpack.exe --folder (항상 새 폴더)
;   opizenAI_Unpack으로 열기…     → opizenAI_Unpack.exe          (전체 창)
!define CTX_ROOT "Software\Classes\SystemFileAssociations"
!macro AddContext EXT
  DeleteRegKey HKLM "${CTX_ROOT}\.${EXT}\shell\opizenAI_Unpack"
  WriteRegStr HKLM "${CTX_ROOT}\.${EXT}\shell\opizenAI_Unpack" "MUIVerb" "opizenAI_Unpack"
  WriteRegStr HKLM "${CTX_ROOT}\.${EXT}\shell\opizenAI_Unpack" "Icon" "$INSTDIR\opizenAI_Unpack.exe,0"
  WriteRegStr HKLM "${CTX_ROOT}\.${EXT}\shell\opizenAI_Unpack" "SubCommands" ""
  WriteRegStr HKLM "${CTX_ROOT}\.${EXT}\shell\opizenAI_Unpack" "MultiSelectModel" "Player"
  WriteRegStr HKLM "${CTX_ROOT}\.${EXT}\shell\opizenAI_Unpack\shell\1here" "MUIVerb" "여기에 압축 풀기"
  WriteRegStr HKLM "${CTX_ROOT}\.${EXT}\shell\opizenAI_Unpack\shell\1here" "MultiSelectModel" "Player"
  WriteRegStr HKLM "${CTX_ROOT}\.${EXT}\shell\opizenAI_Unpack\shell\1here\command" "" '"$INSTDIR\opizenAI_Unpack.exe" --here "%1"'
  WriteRegStr HKLM "${CTX_ROOT}\.${EXT}\shell\opizenAI_Unpack\shell\2folder" "MUIVerb" "새 폴더에 압축 풀기"
  WriteRegStr HKLM "${CTX_ROOT}\.${EXT}\shell\opizenAI_Unpack\shell\2folder" "MultiSelectModel" "Player"
  WriteRegStr HKLM "${CTX_ROOT}\.${EXT}\shell\opizenAI_Unpack\shell\2folder\command" "" '"$INSTDIR\opizenAI_Unpack.exe" --folder "%1"'
  WriteRegStr HKLM "${CTX_ROOT}\.${EXT}\shell\opizenAI_Unpack\shell\3open" "MUIVerb" "opizenAI_Unpack으로 열기..."
  WriteRegDWORD HKLM "${CTX_ROOT}\.${EXT}\shell\opizenAI_Unpack\shell\3open" "CommandFlags" 0x20 ; 위에 구분선
  WriteRegStr HKLM "${CTX_ROOT}\.${EXT}\shell\opizenAI_Unpack\shell\3open\command" "" '"$INSTDIR\opizenAI_Unpack.exe" "%1"'
  ; "연결 프로그램" 목록에 opizenAI_Unpack 표시
  WriteRegStr HKLM "Software\Classes\.${EXT}\OpenWithProgids" "opizenAI_Unpack.Archive" ""
!macroend
!macro RemoveContext EXT
  DeleteRegKey HKLM "${CTX_ROOT}\.${EXT}\shell\opizenAI_Unpack"
  DeleteRegValue HKLM "Software\Classes\.${EXT}\OpenWithProgids" "opizenAI_Unpack.Archive"
!macroend
; 모든 파일(*)과 폴더(Directory)에 "압축하기" 하위 메뉴
;   ZIP으로 압축 → --zip   /   7Z로 압축 → --7z   /   압축하기... → --compress
; 여러 개를 선택하면 탐색기가 항목마다 프로그램을 실행하지만, 프로그램이 이를 모아 압축 파일 하나로 만듭니다.
!macro AddCompress CLASS
  DeleteRegKey HKLM "Software\Classes\${CLASS}\shell\opizenAI_Unpack.Compress"
  WriteRegStr HKLM "Software\Classes\${CLASS}\shell\opizenAI_Unpack.Compress" "MUIVerb" "opizenAI_Unpack 압축"
  WriteRegStr HKLM "Software\Classes\${CLASS}\shell\opizenAI_Unpack.Compress" "Icon" "$INSTDIR\opizenAI_Unpack.exe,0"
  WriteRegStr HKLM "Software\Classes\${CLASS}\shell\opizenAI_Unpack.Compress" "SubCommands" ""
  WriteRegStr HKLM "Software\Classes\${CLASS}\shell\opizenAI_Unpack.Compress" "MultiSelectModel" "Player"
  WriteRegStr HKLM "Software\Classes\${CLASS}\shell\opizenAI_Unpack.Compress\shell\1zip" "MUIVerb" "ZIP으로 압축"
  WriteRegStr HKLM "Software\Classes\${CLASS}\shell\opizenAI_Unpack.Compress\shell\1zip" "MultiSelectModel" "Player"
  WriteRegStr HKLM "Software\Classes\${CLASS}\shell\opizenAI_Unpack.Compress\shell\1zip\command" "" '"$INSTDIR\opizenAI_Unpack.exe" --zip "%1"'
  WriteRegStr HKLM "Software\Classes\${CLASS}\shell\opizenAI_Unpack.Compress\shell\27z" "MUIVerb" "7Z로 압축"
  WriteRegStr HKLM "Software\Classes\${CLASS}\shell\opizenAI_Unpack.Compress\shell\27z" "MultiSelectModel" "Player"
  WriteRegStr HKLM "Software\Classes\${CLASS}\shell\opizenAI_Unpack.Compress\shell\27z\command" "" '"$INSTDIR\opizenAI_Unpack.exe" --7z "%1"'
  WriteRegStr HKLM "Software\Classes\${CLASS}\shell\opizenAI_Unpack.Compress\shell\3ask" "MUIVerb" "압축하기..."
  WriteRegStr HKLM "Software\Classes\${CLASS}\shell\opizenAI_Unpack.Compress\shell\3ask" "MultiSelectModel" "Player"
  WriteRegDWORD HKLM "Software\Classes\${CLASS}\shell\opizenAI_Unpack.Compress\shell\3ask" "CommandFlags" 0x20
  WriteRegStr HKLM "Software\Classes\${CLASS}\shell\opizenAI_Unpack.Compress\shell\3ask\command" "" '"$INSTDIR\opizenAI_Unpack.exe" --compress "%1"'
!macroend
!macro RemoveCompress CLASS
  DeleteRegKey HKLM "Software\Classes\${CLASS}\shell\opizenAI_Unpack.Compress"
!macroend

!macro ForEachExt MACRO
  !insertmacro ${MACRO} "zip"
  !insertmacro ${MACRO} "jar"
  !insertmacro ${MACRO} "tbz"
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
    MessageBox MB_ICONSTOP "opizenAI_Unpack은 64비트 Windows에서만 설치할 수 있습니다."
    Abort
  ${EndIf}
  SetRegView 64
FunctionEnd

Function un.onInit
  SetRegView 64
FunctionEnd

Section "opizenAI_Unpack (필수)" SecMain
  SectionIn RO
  SetOutPath "$INSTDIR"
  File "${BINDIR}\opizenAI_Unpack.exe"
  File "${BINDIR}\unpack.exe"
  File "..\README.md"
  File "..\LICENSE"

  CreateDirectory "$SMPROGRAMS\${APPNAME}"
  CreateShortcut "$SMPROGRAMS\${APPNAME}\opizenAI_Unpack.lnk" "$INSTDIR\opizenAI_Unpack.exe"
  CreateShortcut "$SMPROGRAMS\${APPNAME}\opizenAI_Unpack 제거.lnk" "$INSTDIR\uninstall.exe"

  WriteRegStr HKLM "Software\${APPNAME}" "InstallDir" "$INSTDIR"
  WriteUninstaller "$INSTDIR\uninstall.exe"
  WriteRegStr HKLM "${UNINST_KEY}" "DisplayName" "${APPNAME}"
  WriteRegStr HKLM "${UNINST_KEY}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKLM "${UNINST_KEY}" "Publisher" "${PUBLISHER}"
  WriteRegStr HKLM "${UNINST_KEY}" "DisplayIcon" "$INSTDIR\opizenAI_Unpack.exe"
  WriteRegStr HKLM "${UNINST_KEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKLM "${UNINST_KEY}" "UninstallString" '"$INSTDIR\uninstall.exe"'
  WriteRegStr HKLM "${UNINST_KEY}" "QuietUninstallString" '"$INSTDIR\uninstall.exe" /S'
  WriteRegDWORD HKLM "${UNINST_KEY}" "NoModify" 1
  WriteRegDWORD HKLM "${UNINST_KEY}" "NoRepair" 1
SectionEnd

Section "탐색기 우클릭 메뉴" SecContext
  ; "연결 프로그램"에서 쓰는 ProgID
  WriteRegStr HKLM "Software\Classes\opizenAI_Unpack.Archive" "" "압축 파일"
  WriteRegStr HKLM "Software\Classes\opizenAI_Unpack.Archive\DefaultIcon" "" "$INSTDIR\opizenAI_Unpack.exe,0"
  WriteRegStr HKLM "Software\Classes\opizenAI_Unpack.Archive\shell\open\command" "" '"$INSTDIR\opizenAI_Unpack.exe" "%1"'
  !insertmacro ForEachExt AddContext
  !insertmacro AddCompress "*"
  !insertmacro AddCompress "Directory"
  ; 탐색기에 연결 정보가 바뀌었음을 알림
  System::Call 'shell32::SHChangeNotify(i 0x08000000, i 0, p 0, p 0)'
SectionEnd

Section "바탕화면 바로가기" SecDesktop
  CreateShortcut "$DESKTOP\opizenAI_Unpack.lnk" "$INSTDIR\opizenAI_Unpack.exe"
SectionEnd

!insertmacro MUI_FUNCTION_DESCRIPTION_BEGIN
  !insertmacro MUI_DESCRIPTION_TEXT ${SecMain} "opizenAI_Unpack 프로그램(GUI, 명령줄 unpack.exe)과 시작 메뉴 바로가기"
  !insertmacro MUI_DESCRIPTION_TEXT ${SecContext} "압축 파일 우클릭 → 압축 풀기 메뉴, 모든 파일·폴더 우클릭 → ZIP/7Z로 압축, 압축하기..."
  !insertmacro MUI_DESCRIPTION_TEXT ${SecDesktop} "바탕화면에 바로가기를 만듭니다"
!insertmacro MUI_FUNCTION_DESCRIPTION_END

Section "Uninstall"
  !insertmacro ForEachExt RemoveContext
  !insertmacro RemoveCompress "*"
  !insertmacro RemoveCompress "Directory"
  DeleteRegKey HKLM "Software\Classes\opizenAI_Unpack.Archive"
  System::Call 'shell32::SHChangeNotify(i 0x08000000, i 0, p 0, p 0)'
  Delete "$DESKTOP\opizenAI_Unpack.lnk"
  RMDir /r "$SMPROGRAMS\${APPNAME}"
  Delete "$INSTDIR\opizenAI_Unpack.exe"
  Delete "$INSTDIR\unpack.exe"
  Delete "$INSTDIR\README.md"
  Delete "$INSTDIR\LICENSE"
  Delete "$INSTDIR\uninstall.exe"
  RMDir "$INSTDIR"
  DeleteRegKey HKLM "${UNINST_KEY}"
  DeleteRegKey HKLM "Software\${APPNAME}"
SectionEnd
