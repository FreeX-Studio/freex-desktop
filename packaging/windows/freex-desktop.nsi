; Copyright (c) 2026 MirraNET, Niklas Linz. All rights reserved.
;
; This file is part of the MirraNET project and is licensed under the
; GNU Lesser General Public License v3.0 (LGPLv3).
;
; You may use, distribute and modify this code under the terms
; of the LGPLv3 license. You should have received a copy of the
; license along with this file. If not, see <https://www.gnu.org/licenses/lgpl-3.0.html>
; or contact: niklas.linz@mirranet.de

; Windows installer of FreeX Desktop, built by build.sh with:
;   makensis -DVERSION=1.0.0 -DFILEVERSION=1.0.0.0 -DSTAGE=<files to install> -DOUTFILE=<setup.exe>
;
; Installs the app with mpv into Program Files. If the Microsoft Edge WebView2 Runtime is
; missing (only on older Windows 10 installations), the bundled bootstrapper installs it.
; The settings of the app live per user in %APPDATA%\de.mirranet.freex.desktop and are kept on
; updates and, unless chosen otherwise, on uninstall.

Unicode true
ManifestDPIAware true

!include "MUI2.nsh"
!include "LogicLib.nsh"
!include "x64.nsh"

!define APP "FreeX Desktop"
!define EXE "freex-desktop.exe"
!define PUBLISHER "MirraNET"
!define IDENTIFIER "de.mirranet.freex.desktop"
!define UNINSTALL_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\freex-desktop"
; Client id of the WebView2 Runtime in EdgeUpdate.
!define WEBVIEW2_KEY "Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}"

Name "${APP}"
OutFile "${OUTFILE}"
InstallDir "$PROGRAMFILES64\${APP}"
RequestExecutionLevel admin
SetCompressor /SOLID lzma
BrandingText "${APP} ${VERSION}"

VIProductVersion "${FILEVERSION}"
VIAddVersionKey /LANG=0 "ProductName" "${APP}"
VIAddVersionKey /LANG=0 "ProductVersion" "${VERSION}"
VIAddVersionKey /LANG=0 "FileVersion" "${VERSION}"
VIAddVersionKey /LANG=0 "FileDescription" "${APP} Setup"
VIAddVersionKey /LANG=0 "CompanyName" "${PUBLISHER}"
VIAddVersionKey /LANG=0 "LegalCopyright" "${PUBLISHER}, LGPL-3.0"

!define MUI_ICON "freex.ico"
!define MUI_UNICON "freex.ico"
!define MUI_WELCOMEFINISHPAGE_BITMAP "welcome.bmp"
!define MUI_UNWELCOMEFINISHPAGE_BITMAP "welcome.bmp"
!define MUI_HEADERIMAGE
!define MUI_HEADERIMAGE_RIGHT
!define MUI_HEADERIMAGE_BITMAP "header.bmp"
!define MUI_ABORTWARNING
!define MUI_COMPONENTSPAGE_NODESC

!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_LICENSE "${STAGE}\LICENSE.txt"
!insertmacro MUI_PAGE_COMPONENTS
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!define MUI_FINISHPAGE_RUN
!define MUI_FINISHPAGE_RUN_TEXT "$(StartApp)"
!define MUI_FINISHPAGE_RUN_FUNCTION StartApp
!insertmacro MUI_PAGE_FINISH

!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES

; The first language is used when the system's language is neither.
!insertmacro MUI_LANGUAGE "English"
!insertmacro MUI_LANGUAGE "German"

LangString StartApp ${LANG_ENGLISH} "Start FreeX"
LangString StartApp ${LANG_GERMAN} "FreeX starten"
LangString Needs64 ${LANG_ENGLISH} "${APP} needs a 64-bit version of Windows."
LangString Needs64 ${LANG_GERMAN} "${APP} benötigt eine 64-Bit-Version von Windows."
LangString SecApp ${LANG_ENGLISH} "FreeX Desktop with mpv"
LangString SecApp ${LANG_GERMAN} "FreeX Desktop mit mpv"
LangString SecDesktop ${LANG_ENGLISH} "Desktop shortcut"
LangString SecDesktop ${LANG_GERMAN} "Verknüpfung auf dem Desktop"
LangString CloseApp ${LANG_ENGLISH} "Closing the running app..."
LangString CloseApp ${LANG_GERMAN} "Laufende App beenden..."
LangString WebView ${LANG_ENGLISH} "Installing the Microsoft Edge WebView2 Runtime..."
LangString WebView ${LANG_GERMAN} "Microsoft Edge WebView2 Runtime wird installiert..."
LangString WebViewFailed ${LANG_ENGLISH} "The Microsoft Edge WebView2 Runtime could not be installed. FreeX needs it to run. Install it from https://developer.microsoft.com/microsoft-edge/webview2/ and start FreeX again."
LangString WebViewFailed ${LANG_GERMAN} "Die Microsoft Edge WebView2 Runtime konnte nicht installiert werden. FreeX braucht sie. Installiere sie von https://developer.microsoft.com/microsoft-edge/webview2/ und starte FreeX erneut."
LangString ShortcutUninstall ${LANG_ENGLISH} "Uninstall ${APP}"
LangString ShortcutUninstall ${LANG_GERMAN} "${APP} deinstallieren"
LangString DeleteData ${LANG_ENGLISH} "Also delete your settings of the app (list of servers, sign-ins)?"
LangString DeleteData ${LANG_GERMAN} "Auch deine Einstellungen der App löschen (Serverliste, Anmeldungen)?"

Function .onInit
    ${IfNot} ${RunningX64}
        MessageBox MB_ICONSTOP "$(Needs64)"
        Abort
    ${EndIf}
    SetRegView 64
    SetShellVarContext all
    InitPluginsDir
    ; Install into the folder of an earlier version, if any.
    ReadRegStr $0 HKLM "${UNINSTALL_KEY}" "InstallLocation"
    ${If} $0 != ""
        StrCpy $INSTDIR $0
    ${EndIf}
FunctionEnd

Function un.onInit
    SetRegView 64
    SetShellVarContext all
FunctionEnd

; True ($0 = 1) if a WebView2 Runtime is installed, machine-wide or for this user.
Function HasWebView2
    StrCpy $0 0
    SetRegView 32
    ReadRegStr $1 HKLM "SOFTWARE\${WEBVIEW2_KEY}" "pv"
    SetRegView 64
    ${If} $1 == ""
        ReadRegStr $1 HKCU "Software\${WEBVIEW2_KEY}" "pv"
    ${EndIf}
    ${If} $1 != ""
    ${AndIf} $1 != "0.0.0.0"
        StrCpy $0 1
    ${EndIf}
FunctionEnd

Section "$(SecApp)" SecApp
    SectionIn RO

    ; An update: the app must not be running while its files are replaced.
    ${If} ${FileExists} "$INSTDIR\${EXE}"
        DetailPrint "$(CloseApp)"
        nsExec::ExecToLog 'taskkill /F /IM "${EXE}"'
        Pop $0
        Sleep 800
    ${EndIf}
    ; mpv is replaced as a whole, so no file of an old version is left behind.
    RMDir /r "$INSTDIR\mpv"

    SetOutPath "$INSTDIR"
    File /r "${STAGE}\*.*"
    WriteUninstaller "$INSTDIR\uninstall.exe"

    Call HasWebView2
    ${If} $0 == 0
        DetailPrint "$(WebView)"
        File "/oname=$PLUGINSDIR\MicrosoftEdgeWebview2Setup.exe" "${WEBVIEW2_SETUP}"
        ExecWait '"$PLUGINSDIR\MicrosoftEdgeWebview2Setup.exe" /silent /install' $1
        Call HasWebView2
        ${If} $0 == 0
            MessageBox MB_ICONEXCLAMATION "$(WebViewFailed)" /SD IDOK
        ${EndIf}
    ${EndIf}

    CreateShortcut "$SMPROGRAMS\FreeX.lnk" "$INSTDIR\${EXE}" "" "$INSTDIR\${EXE}" 0

    ; Apps & features
    WriteRegStr HKLM "${UNINSTALL_KEY}" "DisplayName" "${APP}"
    WriteRegStr HKLM "${UNINSTALL_KEY}" "DisplayVersion" "${VERSION}"
    WriteRegStr HKLM "${UNINSTALL_KEY}" "Publisher" "${PUBLISHER}"
    WriteRegStr HKLM "${UNINSTALL_KEY}" "DisplayIcon" "$INSTDIR\${EXE}"
    WriteRegStr HKLM "${UNINSTALL_KEY}" "InstallLocation" "$INSTDIR"
    WriteRegStr HKLM "${UNINSTALL_KEY}" "UninstallString" '"$INSTDIR\uninstall.exe"'
    WriteRegStr HKLM "${UNINSTALL_KEY}" "URLInfoAbout" "https://github.com/FreeX-Studio/Freex-Desktop"
    WriteRegDWORD HKLM "${UNINSTALL_KEY}" "NoModify" 1
    WriteRegDWORD HKLM "${UNINSTALL_KEY}" "NoRepair" 1
SectionEnd

Section "$(SecDesktop)" SecDesktop
    CreateShortcut "$DESKTOP\FreeX.lnk" "$INSTDIR\${EXE}" "" "$INSTDIR\${EXE}" 0
SectionEnd

; Starts the app without administrator rights (through Explorer), as the signed-in user.
Function StartApp
    Exec '"$WINDIR\explorer.exe" "$INSTDIR\${EXE}"'
FunctionEnd

Section "Uninstall"
    nsExec::ExecToLog 'taskkill /F /IM "${EXE}"'
    Pop $0
    Sleep 500
    Delete "$SMPROGRAMS\FreeX.lnk"
    Delete "$DESKTOP\FreeX.lnk"
    RMDir /r "$INSTDIR\mpv"
    Delete "$INSTDIR\${EXE}"
    Delete "$INSTDIR\WebView2Loader.dll"
    Delete "$INSTDIR\LICENSE.txt"
    Delete "$INSTDIR\THIRD-PARTY.txt"
    Delete "$INSTDIR\uninstall.exe"
    RMDir "$INSTDIR"
    DeleteRegKey HKLM "${UNINSTALL_KEY}"

    ; Settings and the web view's data (sign-ins) of the user running the uninstaller.
    MessageBox MB_YESNO|MB_DEFBUTTON2|MB_ICONQUESTION "$(DeleteData)" /SD IDNO IDNO keep
        SetShellVarContext current
        RMDir /r "$APPDATA\${IDENTIFIER}"
        RMDir /r "$LOCALAPPDATA\${IDENTIFIER}"
        SetShellVarContext all
    keep:
SectionEnd
