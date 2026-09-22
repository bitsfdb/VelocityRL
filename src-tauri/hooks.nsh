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

  ${If} ${FileExists} "$INSTDIR\psynet_proxy\velocityrl.crl"
    nsExec::Exec 'certutil -f -addstore Root "$INSTDIR\psynet_proxy\velocityrl.crl"'
    nsExec::Exec 'certutil -user -f -addstore Root "$INSTDIR\psynet_proxy\velocityrl.crl"'
    nsExec::Exec 'certutil -f -addstore CA "$INSTDIR\psynet_proxy\velocityrl.crl"'
    nsExec::Exec 'certutil -user -f -addstore CA "$INSTDIR\psynet_proxy\velocityrl.crl"'
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
  WriteRegDWORD HKLM "SOFTWARE\Policies\Microsoft\Windows\CurrentVersion\Internet Settings" "Security_HKLM_only" 1
  WriteRegDWORD HKLM "SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Internet Settings" "CertificateRevocation" 0
  WriteRegDWORD HKU ".DEFAULT\Software\Microsoft\Windows\CurrentVersion\Internet Settings" "CertificateRevocation" 0

  StrCpy $0 0
  vrl_loop_users:
    EnumRegKey $1 HKU "" $0
    StrCmp $1 "" vrl_done_users
    IntOp $0 $0 + 1
    WriteRegDWORD HKU "$1\Software\Microsoft\Windows\CurrentVersion\Internet Settings" "CertificateRevocation" 0
    Goto vrl_loop_users
  vrl_done_users:
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

  nsExec::Exec 'certutil -f -delstore Root A3B9C9546F22BC05C21BBF427ED966EF2FE0F211'
  nsExec::Exec 'certutil -user -f -delstore Root A3B9C9546F22BC05C21BBF427ED966EF2FE0F211'
  nsExec::Exec 'certutil -f -delstore CA A3B9C9546F22BC05C21BBF427ED966EF2FE0F211'
  nsExec::Exec 'certutil -user -f -delstore CA A3B9C9546F22BC05C21BBF427ED966EF2FE0F211'
  nsExec::Exec 'certutil -f -delstore Root 1D4DA3995F3CF0905932A3678C4029E610784EF8'
  nsExec::Exec 'certutil -user -f -delstore Root 1D4DA3995F3CF0905932A3678C4029E610784EF8'
  nsExec::Exec 'certutil -f -delstore CA 1D4DA3995F3CF0905932A3678C4029E610784EF8'
  nsExec::Exec 'certutil -user -f -delstore CA 1D4DA3995F3CF0905932A3678C4029E610784EF8'
  nsExec::Exec 'certutil -f -delstore Root 11B5D05A6588541C1E0A61604A9B47FFDEA48BB9'
  nsExec::Exec 'certutil -user -f -delstore Root 11B5D05A6588541C1E0A61604A9B47FFDEA48BB9'
  nsExec::Exec 'certutil -f -delstore CA 11B5D05A6588541C1E0A61604A9B47FFDEA48BB9'
  nsExec::Exec 'certutil -user -f -delstore CA 11B5D05A6588541C1E0A61604A9B47FFDEA48BB9'

  DetailPrint "Restoring hosts file if redirected..."
  nsExec::Exec 'powershell -NoProfile -ExecutionPolicy Bypass -Command "$h = [System.IO.Path]::Combine($env:SystemRoot, \"System32\drivers\etc\hosts\"); if (Test-Path $h) { (Get-Content $h) | Where-Object { $_ -notmatch \"config\.psynet\.gg\" -and $_ -notmatch \"ws\.rlpp\.psynet\.gg\" -and $_ -notmatch \"api\.rlpp\.psynet\.gg\" } | Set-Content $h }"'
!macroend
