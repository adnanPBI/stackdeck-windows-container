; StackDeck v0.6.0: prevent silent collision with historical per-user v0.5.0.
; The old NSIS defaulted to currentUser and wrote to %LOCALAPPDATA%.
; The new package is perMachine and writes to Program Files and HKLM.
; Cross-user HKCU migration cannot be trusted from an elevated installer.
;
; Deliberately fail closed. Do not execute or delete a legacy uninstaller here:
; operators must preserve personal data and verify user context first.
!macro NSIS_HOOK_PREINSTALL
  ReadRegStr $R0 HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\StackDeck" "DisplayName"
  StrCmp $R0 "" 0 SD_HISTORICAL_USER_INSTALL_FOUND
  ReadRegStr $R0 HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\com.stackdeck.desktop" "DisplayName"
  StrCmp $R0 "" 0 SD_HISTORICAL_USER_INSTALL_FOUND
  IfFileExists "$LOCALAPPDATA\StackDeck\uninstall.exe" SD_HISTORICAL_USER_INSTALL_FOUND SD_HISTORICAL_USER_INSTALL_CLEARED

SD_HISTORICAL_USER_INSTALL_FOUND:
  MessageBox MB_OK|MB_ICONSTOP /SD IDOK "An older per-user StackDeck installation is present. The v0.6.0 installer is machine-wide and will not overwrite the old user's installation. Back up the original user's StackDeck data and use Windows Apps to uninstall v0.5.0 while signed in as that user, then rerun this installer as an administrator. For a different Windows user, an administrator must first review that user's installation separately."
  SetErrorLevel 1603
  Quit

SD_HISTORICAL_USER_INSTALL_CLEARED:
!macroend
