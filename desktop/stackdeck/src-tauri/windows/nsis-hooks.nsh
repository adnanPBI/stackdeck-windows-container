Var StackDeckVmRoot
Var StackDeckLogPath
Var StackDeckHookExit

!macro NSIS_HOOK_POSTINSTALL
  DetailPrint "StackDeck: preparing Hyper-V runtime configuration."
  CreateDirectory "$LOCALAPPDATA\StackDeck\install-logs"
  StrCpy $StackDeckLogPath "$LOCALAPPDATA\StackDeck\install-logs\stackdeck-install.log"
  StrCpy $StackDeckVmRoot "$INSTDIR\VMs\stackdeck-linux"

  IfSilent StackDeckUseDefaultVmRoot 0
  nsDialogs::SelectFolderDialog "Select the StackDeck Hyper-V VM image/runtime folder. This folder will store stackdeck-linux VHDX, seed ISO, and runtime files." "$StackDeckVmRoot"
  Pop $0
  StrCmp $0 "error" +2 0
    StrCpy $0 ""
  StrCmp $0 "" 0 +2
    StrCpy $0 "$StackDeckVmRoot"
  StrCpy $StackDeckVmRoot "$0"
  Goto StackDeckConfigureRuntime

  StackDeckUseDefaultVmRoot:
  DetailPrint "StackDeck: silent install detected; using default Hyper-V runtime folder."

  StackDeckConfigureRuntime:

  DetailPrint "StackDeck: selected Hyper-V runtime folder: $StackDeckVmRoot"
  DetailPrint "StackDeck: install log: $StackDeckLogPath"

  nsExec::ExecToLog 'powershell.exe -NoProfile -ExecutionPolicy Bypass -File "$INSTDIR\resources\installer\Configure-StackDeckInstall.ps1" -InstallDir "$INSTDIR" -VmRoot "$StackDeckVmRoot" -LogPath "$StackDeckLogPath" -InstallerKind "NSIS"'
  Pop $StackDeckHookExit

  ${If} $StackDeckHookExit != 0
    DetailPrint "StackDeck: runtime configuration failed. Exit code: $StackDeckHookExit"
    MessageBox MB_ICONEXCLAMATION|MB_OK "StackDeck was installed, but Hyper-V runtime configuration did not complete.$\r$\n$\r$\nLog file:$\r$\n$StackDeckLogPath"
  ${Else}
    DetailPrint "StackDeck: runtime configuration completed."
    MessageBox MB_OK "StackDeck was installed and Hyper-V runtime configuration was saved.$\r$\n$\r$\nLog file:$\r$\n$StackDeckLogPath"
  ${EndIf}
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  CreateDirectory "$LOCALAPPDATA\StackDeck\install-logs"
  FileOpen $0 "$LOCALAPPDATA\StackDeck\install-logs\stackdeck-uninstall.log" a
  FileWrite $0 "StackDeck uninstall completed from $INSTDIR$\r$\n"
  FileClose $0
!macroend
