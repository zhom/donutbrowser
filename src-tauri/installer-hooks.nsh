!macro NSIS_HOOK_PREINSTALL
  IfFileExists "$INSTDIR\donut-proxy.exe" 0 donut_proxy_preinstall_done

  DetailPrint "Stopping Donut proxy workers before replacing application files"
  nsExec::ExecToStack '"$SYSDIR\taskkill.exe" /F /T /IM "donut-proxy.exe"'
  Pop $0
  Pop $1
  Sleep 1000

  ; Removing the old sidecar first prevents NSIS from retaining a same-version
  ; or previously locked executable while updating the main application.
  Delete "$INSTDIR\donut-proxy.exe"

  donut_proxy_preinstall_done:
!macroend
