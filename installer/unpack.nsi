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
!include "LogicLib.nsh"
!include "StrFunc.nsh"
${StrStr}
${UnStrStr}

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
;   알아서 풀기          → opizenAI_Unpack.exe --smart  (폴더가 하나면 그대로, 아니면 파일 이름 폴더)
;   여기에 압축 풀기     → opizenAI_Unpack.exe --here   (그대로 풀기)
;   새 폴더에 압축 풀기  → opizenAI_Unpack.exe --folder (항상 새 폴더)
;   opizenAI_Unpack으로 열기…     → opizenAI_Unpack.exe          (전체 창)
!define CTX_ROOT "Software\Classes\SystemFileAssociations"
!macro AddContext EXT
  DeleteRegKey HKLM "${CTX_ROOT}\.${EXT}\shell\opizenAI_Unpack"
  WriteRegStr HKLM "${CTX_ROOT}\.${EXT}\shell\opizenAI_Unpack" "MUIVerb" "opizenAI_Unpack"
  WriteRegStr HKLM "${CTX_ROOT}\.${EXT}\shell\opizenAI_Unpack" "Icon" "$INSTDIR\opizenAI_Unpack.exe,0"
  WriteRegStr HKLM "${CTX_ROOT}\.${EXT}\shell\opizenAI_Unpack" "SubCommands" ""
  WriteRegStr HKLM "${CTX_ROOT}\.${EXT}\shell\opizenAI_Unpack" "MultiSelectModel" "Player"
  WriteRegStr HKLM "${CTX_ROOT}\.${EXT}\shell\opizenAI_Unpack\shell\0smart" "MUIVerb" "알아서 풀기"
  WriteRegStr HKLM "${CTX_ROOT}\.${EXT}\shell\opizenAI_Unpack\shell\0smart" "MultiSelectModel" "Player"
  WriteRegStr HKLM "${CTX_ROOT}\.${EXT}\shell\opizenAI_Unpack\shell\0smart\command" "" '"$INSTDIR\opizenAI_Unpack.exe" --smart "%1"'
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

; ---------------------------------------------------------------------------
; 파일 연결 (더블클릭하면 opizenAI_Unpack으로 열기)
; 연결 전 기본 프로그램은 Software\opizenAI_Unpack\Backup 에 저장했다가 제거할 때 되돌립니다.
; Windows가 기억하는 사용자 선택(UserChoice)이 있으면 지워야 새 연결이 적용됩니다.
; ---------------------------------------------------------------------------
!define PROGID "opizenAI_Unpack.Archive"
!define FILEEXTS "Software\Microsoft\Windows\CurrentVersion\Explorer\FileExts"

!macro AssocExt EXT
  ReadRegStr $0 HKLM "Software\Classes\.${EXT}" ""
  ${If} $0 != "${PROGID}"
    WriteRegStr HKLM "Software\${APPNAME}\Backup" ".${EXT}" "$0"
  ${EndIf}
  WriteRegStr HKLM "Software\Classes\.${EXT}" "" "${PROGID}"
  WriteRegStr HKLM "Software\Classes\.${EXT}\OpenWithProgids" "${PROGID}" ""
  ; 현재 사용자 설정이 시스템 설정을 덮어쓰고 있으면 그것도 바꿈
  ReadRegStr $0 HKCU "Software\Classes\.${EXT}" ""
  ${If} $0 != ""
  ${AndIf} $0 != "${PROGID}"
    WriteRegStr HKCU "Software\${APPNAME}\Backup" ".${EXT}" "$0"
    WriteRegStr HKCU "Software\Classes\.${EXT}" "" "${PROGID}"
  ${EndIf}
  DeleteRegKey HKCU "${FILEEXTS}\.${EXT}\UserChoice"
!macroend

!macro UnassocExt EXT
  ReadRegStr $0 HKLM "Software\Classes\.${EXT}" ""
  ${If} $0 == "${PROGID}"
    ReadRegStr $1 HKLM "Software\${APPNAME}\Backup" ".${EXT}"
    ${If} $1 != ""
      WriteRegStr HKLM "Software\Classes\.${EXT}" "" "$1"
    ${Else}
      DeleteRegValue HKLM "Software\Classes\.${EXT}" ""
    ${EndIf}
  ${EndIf}
  ReadRegStr $0 HKCU "Software\Classes\.${EXT}" ""
  ${If} $0 == "${PROGID}"
    ReadRegStr $1 HKCU "Software\${APPNAME}\Backup" ".${EXT}"
    ${If} $1 != ""
      WriteRegStr HKCU "Software\Classes\.${EXT}" "" "$1"
    ${Else}
      DeleteRegValue HKCU "Software\Classes\.${EXT}" ""
    ${EndIf}
  ${EndIf}
  ReadRegStr $0 HKCU "${FILEEXTS}\.${EXT}\UserChoice" "ProgId"
  ${If} $0 == "${PROGID}"
    DeleteRegKey HKCU "${FILEEXTS}\.${EXT}\UserChoice"
  ${EndIf}
!macroend

!macro CapExt EXT
  WriteRegStr HKLM "Software\${APPNAME}\Capabilities\FileAssociations" ".${EXT}" "${PROGID}"
!macroend

; zip 외에 연결할 수 있는 형식
!macro ForEachOtherExt MACRO
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

; ---------------------------------------------------------------------------
; Windows 11: 우클릭하면 "추가 옵션 표시" 없이 바로 전체(클래식) 메뉴가 나오게 함
; Microsoft가 제공하는 사용자별 설정이며, 모든 프로그램의 우클릭 메뉴에 적용됩니다.
; ---------------------------------------------------------------------------
!define CLASSIC_MENU "Software\Classes\CLSID\{86ca1aa0-34aa-4e8b-a509-50c905bae2a2}"

; 탐색기를 다시 시작해 메뉴 변경을 바로 적용
!macro RestartExplorer UN
  nsExec::Exec 'taskkill /f /im explorer.exe'
  Pop $0
  Sleep 2500
  ; 보통 Windows가 탐색기를 자동으로 다시 띄우지만, 안 뜨면 직접 실행
  nsExec::ExecToStack 'tasklist /FI "IMAGENAME eq explorer.exe" /NH'
  Pop $0
  Pop $1
  ${${UN}StrStr} $2 $1 "explorer.exe"
  ${If} $2 == ""
    Exec '"$WINDIR\explorer.exe"'
  ${EndIf}
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

  ; 파일 형식(ProgID): 더블클릭하면 창을 열고 내용을 보여 줌
  WriteRegStr HKLM "Software\Classes\${PROGID}" "" "압축 파일"
  WriteRegStr HKLM "Software\Classes\${PROGID}" "FriendlyTypeName" "압축 파일 (opizenAI_Unpack)"
  WriteRegStr HKLM "Software\Classes\${PROGID}\DefaultIcon" "" "$INSTDIR\opizenAI_Unpack.exe,0"
  WriteRegStr HKLM "Software\Classes\${PROGID}\shell\open\command" "" '"$INSTDIR\opizenAI_Unpack.exe" "%1"'

  ; Windows 설정 > 기본 앱 목록에 표시
  WriteRegStr HKLM "Software\${APPNAME}\Capabilities" "ApplicationName" "${APPNAME}"
  WriteRegStr HKLM "Software\${APPNAME}\Capabilities" "ApplicationDescription" "압축 풀기 / 압축하기"
  WriteRegStr HKLM "Software\${APPNAME}\Capabilities" "ApplicationIcon" "$INSTDIR\opizenAI_Unpack.exe,0"
  WriteRegStr HKLM "Software\${APPNAME}\Capabilities\FileAssociations" ".zip" "${PROGID}"
  !insertmacro ForEachOtherExt CapExt
  WriteRegStr HKLM "Software\RegisteredApplications" "${APPNAME}" "Software\${APPNAME}\Capabilities"
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
  !insertmacro ForEachExt AddContext
  !insertmacro AddCompress "*"
  !insertmacro AddCompress "Directory"
SectionEnd

Section "zip 파일 연결 (더블클릭으로 열기)" SecAssocZip
  !insertmacro AssocExt "zip"
SectionEnd

Section /o "그 외 압축 파일 연결 (7z, tar, gz, bz2, xz, zst)" SecAssocOther
  !insertmacro ForEachOtherExt AssocExt
SectionEnd

Section "Windows 11: 우클릭 메뉴 바로 표시" SecWin11Menu
  ReadRegStr $0 HKLM "SOFTWARE\Microsoft\Windows NT\CurrentVersion" "CurrentBuild"
  ${If} $0 >= 22000
    ClearErrors
    ReadRegStr $1 HKCU "${CLASSIC_MENU}\InprocServer32" ""
    ${IfNot} ${Errors}
      ; 이미 클래식 메뉴를 쓰는 중 (직접 설정한 경우) — 제거할 때 건드리지 않음
    ${Else}
      WriteRegStr HKCU "${CLASSIC_MENU}\InprocServer32" "" ""
      WriteRegDWORD HKCU "Software\${APPNAME}" "ClassicMenu" 1
      !insertmacro RestartExplorer ""
    ${EndIf}
  ${EndIf}
SectionEnd

Section "바탕화면 바로가기" SecDesktop
  CreateShortcut "$DESKTOP\opizenAI_Unpack.lnk" "$INSTDIR\opizenAI_Unpack.exe"
SectionEnd

Section "-알림"
  ; 탐색기에 연결 정보가 바뀌었음을 알림
  System::Call 'shell32::SHChangeNotify(i 0x08000000, i 0, p 0, p 0)'
SectionEnd

!insertmacro MUI_FUNCTION_DESCRIPTION_BEGIN
  !insertmacro MUI_DESCRIPTION_TEXT ${SecMain} "opizenAI_Unpack 프로그램(GUI, 명령줄 unpack.exe)과 시작 메뉴 바로가기"
  !insertmacro MUI_DESCRIPTION_TEXT ${SecContext} "압축 파일 우클릭 → 알아서 풀기 / 여기에 / 새 폴더에 압축 풀기, 모든 파일·폴더 우클릭 → ZIP/7Z로 압축, 압축하기..."
  !insertmacro MUI_DESCRIPTION_TEXT ${SecAssocZip} "zip 파일을 더블클릭하면 opizenAI_Unpack으로 엽니다. 제거하면 원래 프로그램으로 돌아갑니다."
  !insertmacro MUI_DESCRIPTION_TEXT ${SecAssocOther} "7z, tar, tar.gz 등 다른 압축 파일도 opizenAI_Unpack으로 엽니다."
  !insertmacro MUI_DESCRIPTION_TEXT ${SecWin11Menu} "Windows 11에서 ‘추가 옵션 표시’를 누르지 않아도 우클릭 메뉴가 바로 나옵니다. 모든 프로그램의 우클릭 메뉴가 Windows 10 방식으로 바뀌며, 제거하면 되돌립니다."
  !insertmacro MUI_DESCRIPTION_TEXT ${SecDesktop} "바탕화면에 바로가기를 만듭니다"
!insertmacro MUI_FUNCTION_DESCRIPTION_END

Section "Uninstall"
  ; 파일 연결을 원래대로 (Backup 키를 지우기 전에)
  !insertmacro UnassocExt "zip"
  !insertmacro ForEachOtherExt UnassocExt
  DeleteRegValue HKLM "Software\RegisteredApplications" "${APPNAME}"
  ; 설치 때 켠 Windows 11 클래식 메뉴만 되돌림
  ReadRegDWORD $0 HKCU "Software\${APPNAME}" "ClassicMenu"
  ${If} $0 == 1
    DeleteRegKey HKCU "${CLASSIC_MENU}"
    !insertmacro RestartExplorer "Un"
  ${EndIf}
  DeleteRegKey HKCU "Software\${APPNAME}"
  !insertmacro ForEachExt RemoveContext
  !insertmacro RemoveCompress "*"
  !insertmacro RemoveCompress "Directory"
  DeleteRegKey HKLM "Software\Classes\${PROGID}"
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
