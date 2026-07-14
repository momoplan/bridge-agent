!macro NSIS_HOOK_POSTINSTALL
  Delete "$DESKTOP\百积木.lnk"
  CreateShortCut "$DESKTOP\百积木.lnk" "$INSTDIR\bridge-agent-desktop.exe" "" "$INSTDIR\bridge-agent-desktop.exe" 0
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  Delete "$DESKTOP\百积木.lnk"
!macroend
