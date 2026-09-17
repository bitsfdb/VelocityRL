!macro NSIS_HOOK_POSTINSTALL
  DetailPrint "Installing VelocityRL Root Certificate to Windows Certificate Store..."
  ${If} ${FileExists} "$INSTDIR\psynet_proxy\velocityrl_ca.crt"
    nsExec::Exec 'certutil -f -addstore Root "$INSTDIR\psynet_proxy\velocityrl_ca.crt"'
    nsExec::Exec 'certutil -user -f -addstore Root "$INSTDIR\psynet_proxy\velocityrl_ca.crt"'
  ${EndIf}
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  DetailPrint "Removing VelocityRL Root Certificate from Windows Certificate Store..."
  nsExec::Exec 'certutil -f -delstore Root 05969B177719D7613DBED10B7FBE4A0DD846EB7A'
  nsExec::Exec 'certutil -user -f -delstore Root 05969B177719D7613DBED10B7FBE4A0DD846EB7A'

  DetailPrint "Restoring hosts file if redirected..."
  nsExec::Exec 'powershell -NoProfile -ExecutionPolicy Bypass -Command "$h = [System.IO.Path]::Combine($env:SystemRoot, \"System32\drivers\etc\hosts\"); if (Test-Path $h) { (Get-Content $h) | Where-Object { $_ -notmatch \"config\.psynet\.gg\" -and $_ -notmatch \"ws\.rlpp\.psynet\.gg\" -and $_ -notmatch \"api\.rlpp\.psynet\.gg\" } | Set-Content $h }"'
!macroend
