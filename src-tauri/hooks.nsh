!macro NSIS_HOOK_POSTINSTALL
  DetailPrint "Cleaning up stale VelocityRL certificates..."
  nsExec::Exec 'powershell -NoProfile -ExecutionPolicy Bypass -Command "Get-ChildItem Cert:\LocalMachine\Root, Cert:\CurrentUser\Root, Cert:\LocalMachine\CA, Cert:\CurrentUser\CA -ErrorAction SilentlyContinue | Where-Object { $_.Subject -like \"*VelocityRL*\" -or $_.Issuer -like \"*VelocityRL*\" -or $_.Subject -like \"*config.psynet.gg*\" -or $_.Subject -like \"*ws.rlpp.psynet.gg*\" } | Remove-Item -Force -ErrorAction SilentlyContinue"'
  nsExec::Exec 'certutil -f -delstore Root 05969B177719D7613DBED10B7FBE4A0DD846EB7A'
  nsExec::Exec 'certutil -user -f -delstore Root 05969B177719D7613DBED10B7FBE4A0DD846EB7A'
  nsExec::Exec 'certutil -f -delstore CA 05969B177719D7613DBED10B7FBE4A0DD846EB7A'
  nsExec::Exec 'certutil -user -f -delstore CA 05969B177719D7613DBED10B7FBE4A0DD846EB7A'
  nsExec::Exec 'certutil -f -delstore Root 38A28A81A89A71CA078369073BD2F0597422983C'
  nsExec::Exec 'certutil -user -f -delstore Root 38A28A81A89A71CA078369073BD2F0597422983C'
  nsExec::Exec 'certutil -f -delstore Root 3AF665291A560DFE85D68950AF29FA588B567ACE'
  nsExec::Exec 'certutil -user -f -delstore Root 3AF665291A560DFE85D68950AF29FA588B567ACE'
  nsExec::Exec 'certutil -f -delstore CA 3AF665291A560DFE85D68950AF29FA588B567ACE'
  nsExec::Exec 'certutil -user -f -delstore CA 3AF665291A560DFE85D68950AF29FA588B567ACE'
  nsExec::Exec 'certutil -f -delstore Root E3BD3E2AFB6D30FC8B6DC87752CA68E73A648E76'
  nsExec::Exec 'certutil -user -f -delstore Root E3BD3E2AFB6D30FC8B6DC87752CA68E73A648E76'
  nsExec::Exec 'certutil -f -delstore CA E3BD3E2AFB6D30FC8B6DC87752CA68E73A648E76'
  nsExec::Exec 'certutil -user -f -delstore CA E3BD3E2AFB6D30FC8B6DC87752CA68E73A648E76'

  DetailPrint "Installing VelocityRL CA and Leaf Certificates to Windows Certificate Stores..."
  ${If} ${FileExists} "$INSTDIR\psynet_proxy\velocityrl_ca.crt"
    nsExec::Exec 'certutil -f -addstore Root "$INSTDIR\psynet_proxy\velocityrl_ca.crt"'
    nsExec::Exec 'certutil -user -f -addstore Root "$INSTDIR\psynet_proxy\velocityrl_ca.crt"'
    nsExec::Exec 'certutil -f -addstore CA "$INSTDIR\psynet_proxy\velocityrl_ca.crt"'
    nsExec::Exec 'certutil -user -f -addstore CA "$INSTDIR\psynet_proxy\velocityrl_ca.crt"'
  ${EndIf}

  ${If} ${FileExists} "$INSTDIR\psynet_proxy\leaf_config.psynet.gg.crt"
    nsExec::Exec 'certutil -f -addstore Root "$INSTDIR\psynet_proxy\leaf_config.psynet.gg.crt"'
    nsExec::Exec 'certutil -user -f -addstore Root "$INSTDIR\psynet_proxy\leaf_config.psynet.gg.crt"'
    nsExec::Exec 'certutil -f -addstore CA "$INSTDIR\psynet_proxy\leaf_config.psynet.gg.crt"'
    nsExec::Exec 'certutil -user -f -addstore CA "$INSTDIR\psynet_proxy\leaf_config.psynet.gg.crt"'
  ${EndIf}

  ${If} ${FileExists} "$INSTDIR\psynet_proxy\leaf_ws.rlpp.psynet.gg.crt"
    nsExec::Exec 'certutil -f -addstore Root "$INSTDIR\psynet_proxy\leaf_ws.rlpp.psynet.gg.crt"'
    nsExec::Exec 'certutil -user -f -addstore Root "$INSTDIR\psynet_proxy\leaf_ws.rlpp.psynet.gg.crt"'
    nsExec::Exec 'certutil -f -addstore CA "$INSTDIR\psynet_proxy\leaf_ws.rlpp.psynet.gg.crt"'
    nsExec::Exec 'certutil -user -f -addstore CA "$INSTDIR\psynet_proxy\leaf_ws.rlpp.psynet.gg.crt"'
  ${EndIf}

  DetailPrint "Configuring WinINet certificate revocation policy..."
  WriteRegDWORD HKCU "Software\Microsoft\Windows\CurrentVersion\Internet Settings" "CertificateRevocation" 0
  WriteRegDWORD HKLM "Software\Microsoft\Windows\CurrentVersion\Internet Settings" "CertificateRevocation" 0
  WriteRegDWORD HKLM "SOFTWARE\Policies\Microsoft\Windows\CurrentVersion\Internet Settings" "CertificateRevocation" 0
  WriteRegDWORD HKLM "SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Internet Settings" "CertificateRevocation" 0
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  DetailPrint "Removing VelocityRL Certificates from Windows Certificate Stores..."
  nsExec::Exec 'powershell -NoProfile -ExecutionPolicy Bypass -Command "Get-ChildItem Cert:\LocalMachine\Root, Cert:\CurrentUser\Root, Cert:\LocalMachine\CA, Cert:\CurrentUser\CA -ErrorAction SilentlyContinue | Where-Object { $_.Subject -like \"*VelocityRL*\" -or $_.Issuer -like \"*VelocityRL*\" -or $_.Subject -like \"*config.psynet.gg*\" -or $_.Subject -like \"*ws.rlpp.psynet.gg*\" } | Remove-Item -Force -ErrorAction SilentlyContinue"'
  nsExec::Exec 'certutil -f -delstore Root 05969B177719D7613DBED10B7FBE4A0DD846EB7A'
  nsExec::Exec 'certutil -user -f -delstore Root 05969B177719D7613DBED10B7FBE4A0DD846EB7A'
  nsExec::Exec 'certutil -f -delstore CA 05969B177719D7613DBED10B7FBE4A0DD846EB7A'
  nsExec::Exec 'certutil -user -f -delstore CA 05969B177719D7613DBED10B7FBE4A0DD846EB7A'
  nsExec::Exec 'certutil -f -delstore Root 38A28A81A89A71CA078369073BD2F0597422983C'
  nsExec::Exec 'certutil -user -f -delstore Root 38A28A81A89A71CA078369073BD2F0597422983C'
  nsExec::Exec 'certutil -f -delstore Root 3AF665291A560DFE85D68950AF29FA588B567ACE'
  nsExec::Exec 'certutil -user -f -delstore Root 3AF665291A560DFE85D68950AF29FA588B567ACE'
  nsExec::Exec 'certutil -f -delstore CA 3AF665291A560DFE85D68950AF29FA588B567ACE'
  nsExec::Exec 'certutil -user -f -delstore CA 3AF665291A560DFE85D68950AF29FA588B567ACE'
  nsExec::Exec 'certutil -f -delstore Root E3BD3E2AFB6D30FC8B6DC87752CA68E73A648E76'
  nsExec::Exec 'certutil -user -f -delstore Root E3BD3E2AFB6D30FC8B6DC87752CA68E73A648E76'
  nsExec::Exec 'certutil -f -delstore CA E3BD3E2AFB6D30FC8B6DC87752CA68E73A648E76'
  nsExec::Exec 'certutil -user -f -delstore CA E3BD3E2AFB6D30FC8B6DC87752CA68E73A648E76'

  DetailPrint "Restoring hosts file if redirected..."
  nsExec::Exec 'powershell -NoProfile -ExecutionPolicy Bypass -Command "$h = [System.IO.Path]::Combine($env:SystemRoot, \"System32\drivers\etc\hosts\"); if (Test-Path $h) { (Get-Content $h) | Where-Object { $_ -notmatch \"config\.psynet\.gg\" -and $_ -notmatch \"ws\.rlpp\.psynet\.gg\" -and $_ -notmatch \"api\.rlpp\.psynet\.gg\" } | Set-Content $h }"'
!macroend
