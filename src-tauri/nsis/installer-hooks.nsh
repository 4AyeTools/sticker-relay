!macro NSIS_HOOK_SET_DEFAULT_INSTALL_DIR
  ; Keep a current 表情递 installation where the user placed it. Only fall
  ; back to the legacy 咻咻搬 location when no current product is registered.
  ReadRegStr $R7 HKCU "${MANUPRODUCTKEY}" ""
  ${If} $R7 == ""
    ReadRegStr $R8 HKCU "Software\ayecode\咻咻搬" ""
    ReadRegStr $R9 HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\咻咻搬" "UninstallString"
    ${If} $R8 != ""
    ${AndIf} $R9 != ""
      StrCpy $INSTDIR $R8
    ${EndIf}
  ${EndIf}
!macroend

!macro NSIS_HOOK_PREINSTALL
  ; v0.3.0 renamed the product from 咻咻搬 to 表情递. Reuse the previous
  ; installation directory and run the old uninstaller in update mode so
  ; application data remains untouched.
  ReadRegStr $R8 HKCU "Software\ayecode\咻咻搬" ""
  ReadRegStr $R9 HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\咻咻搬" "UninstallString"
  ${If} $R8 != ""
  ${AndIf} $R9 != ""
    ExecWait '$R9 /S /UPDATE _?=$R8' $R7
    ${If} $R7 = 0
      StrCpy $INSTDIR $R8
      SetOutPath $INSTDIR
      Delete "$DESKTOP\咻咻搬.lnk"
      Delete "$SMPROGRAMS\咻咻搬.lnk"
      DeleteRegKey HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\咻咻搬"
      DeleteRegKey HKCU "Software\ayecode\咻咻搬"
    ${EndIf}
  ${EndIf}
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  ; Keep the verified Feishu CLI binary outside Tauri's removable app-data
  ; directory. This also protects installations upgraded from v0.2.x when the
  ; user chooses to clear the rest of the application data during uninstall.
  ${If} $DeleteAppDataCheckboxState = 1
  ${AndIf} $UpdateMode <> 1
    ${If} ${FileExists} "$APPDATA\${BUNDLEID}\components\lark-cli\lark-cli.exe"
    ${AndIfNot} ${FileExists} "$LOCALAPPDATA\com.ayecode.wechatfeishustickers-components\lark-cli\lark-cli.exe"
      CreateDirectory "$LOCALAPPDATA\com.ayecode.wechatfeishustickers-components\lark-cli"
      CopyFiles /SILENT "$APPDATA\${BUNDLEID}\components\lark-cli\lark-cli.exe" "$LOCALAPPDATA\com.ayecode.wechatfeishustickers-components\lark-cli"
      CopyFiles /SILENT "$APPDATA\${BUNDLEID}\components\lark-cli\component.json" "$LOCALAPPDATA\com.ayecode.wechatfeishustickers-components\lark-cli"
    ${EndIf}
  ${EndIf}
!macroend
