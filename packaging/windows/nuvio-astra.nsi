; Nuvio Astra — Windows x64 installer (NSIS 3, Unicode). Built by build-release.ps1, which passes:
;   /DVERSION=<x.y.z> /DSRCDIR=<folder with "Nuvio Astra.exe", libmpv-2.dll, nuvio_engine.dll,
;   nuvio-astra.ico, licenses\> /DOUTFILE=<setup.exe>
;
; Saved as UTF-8 WITH BOM: without it makensis reads the file as ANSI and accents are garbled.
;
; Per-user install (no administrator rights): %LOCALAPPDATA%\Programs\Nuvio Astra, Start menu and desktop
; shortcuts, entry in "Apps & features". The user's data stays in %APPDATA%\NuvioNextGen (technical folder
; name kept on purpose) and is never touched by install, update or uninstall.
;
; Updates (official Tauri updater, tauri-plugin-updater) run this installer as
;   setup.exe /P /UPDATE /R [/ARGS ...]
;   /P       passive: no questions, only the progress page, closes by itself
;   /UPDATE  the app is closing: wait until its files can be replaced
;   /R       open the app again at the end
; The previous installation folder (registry) is reused, so an update replaces the program files in
; place. /S (silent) also works, as in any NSIS installer.

Unicode true
!include "MUI2.nsh"
!include "FileFunc.nsh"
!include "LogicLib.nsh"

!define APPNAME "Nuvio Astra"
!define EXENAME "Nuvio Astra.exe"
!define UNINSTNAME "Desinstalar Nuvio Astra.exe"
!define UNINSTKEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\NuvioAstra"

Name "${APPNAME}"
; No custom Caption / run text: NSIS's own strings ("Nuvio Astra Setup", "Run Nuvio Astra") are
; translated into every language below.
OutFile "${OUTFILE}"
InstallDir "$LOCALAPPDATA\Programs\${APPNAME}"
InstallDirRegKey HKCU "Software\${APPNAME}" "InstallDir"
RequestExecutionLevel user
SetCompressor /SOLID lzma
BrandingText "${APPNAME} ${VERSION}"

Var Passive
Var Updating
Var Relaunch

!define MUI_ICON "${SRCDIR}\nuvio-astra.ico"
!define MUI_UNICON "${SRCDIR}\nuvio-astra.ico"
!define MUI_ABORTWARNING
!define MUI_FINISHPAGE_RUN "$INSTDIR\${EXENAME}"

!define MUI_PAGE_CUSTOMFUNCTION_PRE SkipWhenPassive
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!define MUI_PAGE_CUSTOMFUNCTION_PRE SkipWhenPassive
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
; Languages: NSIS opens the installer (and the uninstaller) in the Windows display language when it is
; one of these, without asking; any other language gets the first one, English. Same languages as the
; app, except Bengali, Tamil and Urdu, which NSIS doesn't have (they get English).
!insertmacro MUI_LANGUAGE "English"
!insertmacro MUI_LANGUAGE "PortugueseBR"
!insertmacro MUI_LANGUAGE "Portuguese"
!insertmacro MUI_LANGUAGE "Spanish"
!insertmacro MUI_LANGUAGE "French"
!insertmacro MUI_LANGUAGE "German"
!insertmacro MUI_LANGUAGE "Italian"
!insertmacro MUI_LANGUAGE "Dutch"
!insertmacro MUI_LANGUAGE "Polish"
!insertmacro MUI_LANGUAGE "Czech"
!insertmacro MUI_LANGUAGE "Slovak"
!insertmacro MUI_LANGUAGE "Hungarian"
!insertmacro MUI_LANGUAGE "Romanian"
!insertmacro MUI_LANGUAGE "Bulgarian"
!insertmacro MUI_LANGUAGE "Croatian"
!insertmacro MUI_LANGUAGE "Greek"
!insertmacro MUI_LANGUAGE "Russian"
!insertmacro MUI_LANGUAGE "Ukrainian"
!insertmacro MUI_LANGUAGE "Turkish"
!insertmacro MUI_LANGUAGE "Arabic"
!insertmacro MUI_LANGUAGE "Hebrew"
!insertmacro MUI_LANGUAGE "Indonesian"
!insertmacro MUI_LANGUAGE "Vietnamese"
!insertmacro MUI_LANGUAGE "Japanese"
!insertmacro MUI_LANGUAGE "SimpChinese"
!insertmacro MUI_LANGUAGE "Norwegian"

VIProductVersion "${VERSION}.0"
VIAddVersionKey /LANG=${LANG_ENGLISH} "ProductName" "${APPNAME}"
VIAddVersionKey /LANG=${LANG_ENGLISH} "FileDescription" "${APPNAME} Setup"
VIAddVersionKey /LANG=${LANG_ENGLISH} "CompanyName" "${APPNAME}"
VIAddVersionKey /LANG=${LANG_ENGLISH} "FileVersion" "${VERSION}"
VIAddVersionKey /LANG=${LANG_ENGLISH} "ProductVersion" "${VERSION}"
VIAddVersionKey /LANG=${LANG_ENGLISH} "LegalCopyright" "GPL-3.0"

Function .onInit
  StrCpy $Passive 0
  StrCpy $Updating 0
  StrCpy $Relaunch 0
  ${GetParameters} $0
  ClearErrors
  ${GetOptions} $0 "/P" $1
  ${IfNot} ${Errors}
    StrCpy $Passive 1
    SetAutoClose true
  ${EndIf}
  ClearErrors
  ${GetOptions} $0 "/UPDATE" $1
  ${IfNot} ${Errors}
    StrCpy $Updating 1
  ${EndIf}
  ClearErrors
  ${GetOptions} $0 "/R" $1
  ${IfNot} ${Errors}
    StrCpy $Relaunch 1
  ${EndIf}
FunctionEnd

Function SkipWhenPassive
  ${If} $Passive == 1
    Abort
  ${EndIf}
FunctionEnd

Section "${APPNAME}" Main
  SetOutPath "$INSTDIR"

  ; Update: the app is closing; wait (up to 15 s) until its executable can be replaced.
  ${If} $Updating == 1
    StrCpy $2 0
    ${Do}
      ClearErrors
      Delete "$INSTDIR\${EXENAME}"
      ${IfNot} ${Errors}
        ${Break}
      ${EndIf}
      IntOp $2 $2 + 1
      ${If} $2 >= 60
        ${Break}
      ${EndIf}
      Sleep 250
    ${Loop}
  ${EndIf}

  File "${SRCDIR}\${EXENAME}"
  File "${SRCDIR}\libmpv-2.dll"
  ; P2P engine of the original app (Nuvio Engine, libtorrent), loaded by the app at runtime.
  File "${SRCDIR}\nuvio_engine.dll"
  File "${SRCDIR}\nuvio-astra.ico"
  SetOutPath "$INSTDIR\licenses"
  File /r "${SRCDIR}\licenses\*.*"
  SetOutPath "$INSTDIR"
  WriteUninstaller "$INSTDIR\${UNINSTNAME}"

  CreateShortcut "$SMPROGRAMS\${APPNAME}.lnk" "$INSTDIR\${EXENAME}" "" "$INSTDIR\${EXENAME}" 0
  ; Desktop shortcut only on a first install (an update keeps the user's choice).
  ${If} $Updating == 0
    CreateShortcut "$DESKTOP\${APPNAME}.lnk" "$INSTDIR\${EXENAME}" "" "$INSTDIR\${EXENAME}" 0
  ${EndIf}

  WriteRegStr HKCU "Software\${APPNAME}" "InstallDir" "$INSTDIR"
  WriteRegStr HKCU "${UNINSTKEY}" "DisplayName" "${APPNAME}"
  WriteRegStr HKCU "${UNINSTKEY}" "DisplayIcon" "$INSTDIR\${EXENAME},0"
  WriteRegStr HKCU "${UNINSTKEY}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "${UNINSTKEY}" "Publisher" "${APPNAME}"
  WriteRegStr HKCU "${UNINSTKEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "${UNINSTKEY}" "UninstallString" '"$INSTDIR\${UNINSTNAME}"'
  WriteRegStr HKCU "${UNINSTKEY}" "QuietUninstallString" '"$INSTDIR\${UNINSTNAME}" /S'
  WriteRegDWORD HKCU "${UNINSTKEY}" "NoModify" 1
  WriteRegDWORD HKCU "${UNINSTKEY}" "NoRepair" 1

  ${If} $Relaunch == 1
    Exec '"$INSTDIR\${EXENAME}"'
  ${EndIf}
SectionEnd

Section "Uninstall"
  ; Program files only. %APPDATA%\NuvioNextGen (profiles, addons, library, settings, session) is kept.
  Delete "$INSTDIR\${EXENAME}"
  Delete "$INSTDIR\libmpv-2.dll"
  Delete "$INSTDIR\nuvio_engine.dll"
  Delete "$INSTDIR\nuvio-astra.ico"
  RMDir /r "$INSTDIR\licenses"
  Delete "$INSTDIR\${UNINSTNAME}"
  RMDir "$INSTDIR"
  Delete "$SMPROGRAMS\${APPNAME}.lnk"
  Delete "$DESKTOP\${APPNAME}.lnk"
  DeleteRegKey HKCU "${UNINSTKEY}"
  DeleteRegKey HKCU "Software\${APPNAME}"
SectionEnd
