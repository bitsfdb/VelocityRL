!macro NSIS_HOOK_POSTINSTALL
  DetailPrint "Cleaning up stale VelocityRL certificates..."
  nsExec::Exec 'powershell -NoProfile -ExecutionPolicy Bypass -Command "Get-ChildItem Cert:\LocalMachine\Root, Cert:\CurrentUser\Root, Cert:\LocalMachine\CA, Cert:\CurrentUser\CA -ErrorAction SilentlyContinue | Where-Object { $_.Subject -like \"*VelocityRL*\" -or $_.Issuer -like \"*VelocityRL*\" -or $_.Subject -like \"*config.psynet.gg*\" -or $_.Subject -like \"*ws.rlpp.psynet.gg*\" } | Remove-Item -Force -ErrorAction SilentlyContinue"'
  ; Old CA thumbprints
  nsExec::Exec 'certutil -f -delstore Root 05969B177719D7613DBED10B7FBE4A0DD846EB7A'
  nsExec::Exec 'certutil -user -f -delstore Root 05969B177719D7613DBED10B7FBE4A0DD846EB7A'
  nsExec::Exec 'certutil -f -delstore CA 05969B177719D7613DBED10B7FBE4A0DD846EB7A'
  nsExec::Exec 'certutil -user -f -delstore CA 05969B177719D7613DBED10B7FBE4A0DD846EB7A'
  nsExec::Exec 'certutil -f -delstore Root 38A28A81A89A71CA078369073BD2F0597422983C'
  nsExec::Exec 'certutil -user -f -delstore Root 38A28A81A89A71CA078369073BD2F0597422983C'
  ; Old leaf cert thumbprints (3AF... = config, E3BD... = ws)
  nsExec::Exec 'certutil -f -delstore Root 3AF665291A560DFE85D68950AF29FA588B567ACE'
  nsExec::Exec 'certutil -user -f -delstore Root 3AF665291A560DFE85D68950AF29FA588B567ACE'
  nsExec::Exec 'certutil -f -delstore CA 3AF665291A560DFE85D68950AF29FA588B567ACE'
  nsExec::Exec 'certutil -user -f -delstore CA 3AF665291A560DFE85D68950AF29FA588B567ACE'
  nsExec::Exec 'certutil -f -delstore Root E3BD3E2AFB6D30FC8B6DC87752CA68E73A648E76'
  nsExec::Exec 'certutil -user -f -delstore Root E3BD3E2AFB6D30FC8B6DC87752CA68E73A648E76'
  nsExec::Exec 'certutil -f -delstore CA E3BD3E2AFB6D30FC8B6DC87752CA68E73A648E76'
  nsExec::Exec 'certutil -user -f -delstore CA E3BD3E2AFB6D30FC8B6DC87752CA68E73A648E76'
  ; Previous build leaf thumbprints (CFFF... = config, 2901... = ws)
  nsExec::Exec 'certutil -f -delstore Root CFFF312D754F62344E30E11D128CDB1F35CF8FC8'
  nsExec::Exec 'certutil -user -f -delstore Root CFFF312D754F62344E30E11D128CDB1F35CF8FC8'
  nsExec::Exec 'certutil -f -delstore CA CFFF312D754F62344E30E11D128CDB1F35CF8FC8'
  nsExec::Exec 'certutil -user -f -delstore CA CFFF312D754F62344E30E11D128CDB1F35CF8FC8'
  nsExec::Exec 'certutil -f -delstore Root 290193877074751336AECEE8554F0D065F8F11CE'
  nsExec::Exec 'certutil -user -f -delstore Root 290193877074751336AECEE8554F0D065F8F11CE'
  nsExec::Exec 'certutil -f -delstore CA 290193877074751336AECEE8554F0D065F8F11CE'
  nsExec::Exec 'certutil -user -f -delstore CA 290193877074751336AECEE8554F0D065F8F11CE'
  ; Previous CRL hashes
  nsExec::Exec 'certutil -f -delstore Root 9DB9369DF51127837DC086DBA047B8DBB4A626D3'
  nsExec::Exec 'certutil -user -f -delstore Root 9DB9369DF51127837DC086DBA047B8DBB4A626D3'
  nsExec::Exec 'certutil -f -delstore CA 9DB9369DF51127837DC086DBA047B8DBB4A626D3'
  nsExec::Exec 'certutil -user -f -delstore CA 9DB9369DF51127837DC086DBA047B8DBB4A626D3'
  ; ws leaf from current build (0A53...) - was erroneously installed in a previous build, clean up
  nsExec::Exec 'certutil -f -delstore Root 0A5340B975F5AD3E8B7B9382C9CF8B37BAAF7EC8'
  nsExec::Exec 'certutil -user -f -delstore Root 0A5340B975F5AD3E8B7B9382C9CF8B37BAAF7EC8'
  nsExec::Exec 'certutil -f -delstore CA 0A5340B975F5AD3E8B7B9382C9CF8B37BAAF7EC8'
  nsExec::Exec 'certutil -user -f -delstore CA 0A5340B975F5AD3E8B7B9382C9CF8B37BAAF7EC8'

  DetailPrint "Installing VelocityRL CA and CRL to Windows Certificate Stores..."
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

  ; config.psynet.gg leaf is MITM-intercepted so Schannel checks its revocation - install it.
  ${If} ${FileExists} "$INSTDIR\psynet_proxy\leaf_config.psynet.gg.crt"
    nsExec::Exec 'certutil -f -addstore Root "$INSTDIR\psynet_proxy\leaf_config.psynet.gg.crt"'
    nsExec::Exec 'certutil -user -f -addstore Root "$INSTDIR\psynet_proxy\leaf_config.psynet.gg.crt"'
    nsExec::Exec 'certutil -f -addstore CA "$INSTDIR\psynet_proxy\leaf_config.psynet.gg.crt"'
    nsExec::Exec 'certutil -user -f -addstore CA "$INSTDIR\psynet_proxy\leaf_config.psynet.gg.crt"'
  ${EndIf}

  ; ws.rlpp.psynet.gg is cert-pinned and never redirected via hosts file - do NOT install its
  ; leaf cert into Windows stores. Schannel never checks our fake ws cert for revocation.

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
  ; All known CA thumbprints
  nsExec::Exec 'certutil -f -delstore Root 05969B177719D7613DBED10B7FBE4A0DD846EB7A'
  nsExec::Exec 'certutil -user -f -delstore Root 05969B177719D7613DBED10B7FBE4A0DD846EB7A'
  nsExec::Exec 'certutil -f -delstore CA 05969B177719D7613DBED10B7FBE4A0DD846EB7A'
  nsExec::Exec 'certutil -user -f -delstore CA 05969B177719D7613DBED10B7FBE4A0DD846EB7A'
  nsExec::Exec 'certutil -f -delstore Root 38A28A81A89A71CA078369073BD2F0597422983C'
  nsExec::Exec 'certutil -user -f -delstore Root 38A28A81A89A71CA078369073BD2F0597422983C'
  ; Old leaf thumbprints
  nsExec::Exec 'certutil -f -delstore Root 3AF665291A560DFE85D68950AF29FA588B567ACE'
  nsExec::Exec 'certutil -user -f -delstore Root 3AF665291A560DFE85D68950AF29FA588B567ACE'
  nsExec::Exec 'certutil -f -delstore CA 3AF665291A560DFE85D68950AF29FA588B567ACE'
  nsExec::Exec 'certutil -user -f -delstore CA 3AF665291A560DFE85D68950AF29FA588B567ACE'
  nsExec::Exec 'certutil -f -delstore Root E3BD3E2AFB6D30FC8B6DC87752CA68E73A648E76'
  nsExec::Exec 'certutil -user -f -delstore Root E3BD3E2AFB6D30FC8B6DC87752CA68E73A648E76'
  nsExec::Exec 'certutil -f -delstore CA E3BD3E2AFB6D30FC8B6DC87752CA68E73A648E76'
  nsExec::Exec 'certutil -user -f -delstore CA E3BD3E2AFB6D30FC8B6DC87752CA68E73A648E76'
  ; Previous build leaf thumbprints
  nsExec::Exec 'certutil -f -delstore Root CFFF312D754F62344E30E11D128CDB1F35CF8FC8'
  nsExec::Exec 'certutil -user -f -delstore Root CFFF312D754F62344E30E11D128CDB1F35CF8FC8'
  nsExec::Exec 'certutil -f -delstore CA CFFF312D754F62344E30E11D128CDB1F35CF8FC8'
  nsExec::Exec 'certutil -user -f -delstore CA CFFF312D754F62344E30E11D128CDB1F35CF8FC8'
  nsExec::Exec 'certutil -f -delstore Root 290193877074751336AECEE8554F0D065F8F11CE'
  nsExec::Exec 'certutil -user -f -delstore Root 290193877074751336AECEE8554F0D065F8F11CE'
  nsExec::Exec 'certutil -f -delstore CA 290193877074751336AECEE8554F0D065F8F11CE'
  nsExec::Exec 'certutil -user -f -delstore CA 290193877074751336AECEE8554F0D065F8F11CE'
  ; Current build leaf thumbprints (FFB0... = config, 0A53... = ws)
  nsExec::Exec 'certutil -f -delstore Root FFB086A4E5D17B08ABD81BD496874BFCA7928D23'
  nsExec::Exec 'certutil -user -f -delstore Root FFB086A4E5D17B08ABD81BD496874BFCA7928D23'
  nsExec::Exec 'certutil -f -delstore CA FFB086A4E5D17B08ABD81BD496874BFCA7928D23'
  nsExec::Exec 'certutil -user -f -delstore CA FFB086A4E5D17B08ABD81BD496874BFCA7928D23'
  nsExec::Exec 'certutil -f -delstore Root 0A5340B975F5AD3E8B7B9382C9CF8B37BAAF7EC8'
  nsExec::Exec 'certutil -user -f -delstore Root 0A5340B975F5AD3E8B7B9382C9CF8B37BAAF7EC8'
  nsExec::Exec 'certutil -f -delstore CA 0A5340B975F5AD3E8B7B9382C9CF8B37BAAF7EC8'
  nsExec::Exec 'certutil -user -f -delstore CA 0A5340B975F5AD3E8B7B9382C9CF8B37BAAF7EC8'
  ; All known CRL hashes
  nsExec::Exec 'certutil -f -delstore Root 9DB9369DF51127837DC086DBA047B8DBB4A626D3'
  nsExec::Exec 'certutil -user -f -delstore Root 9DB9369DF51127837DC086DBA047B8DBB4A626D3'
  nsExec::Exec 'certutil -f -delstore CA 9DB9369DF51127837DC086DBA047B8DBB4A626D3'
  nsExec::Exec 'certutil -user -f -delstore CA 9DB9369DF51127837DC086DBA047B8DBB4A626D3'
  nsExec::Exec 'certutil -f -delstore Root C6FF88B7B1038B88B89FCDC92422C6397860CFAD'
  nsExec::Exec 'certutil -user -f -delstore Root C6FF88B7B1038B88B89FCDC92422C6397860CFAD'
  nsExec::Exec 'certutil -f -delstore CA C6FF88B7B1038B88B89FCDC92422C6397860CFAD'
  nsExec::Exec 'certutil -user -f -delstore CA C6FF88B7B1038B88B89FCDC92422C6397860CFAD'
  ; Older CRL hashes
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

  DetailPrint "Removing VelocityRL data directories..."
  RMDir /r "$APPDATA\VelocityRL"
  RMDir /r "$LOCALAPPDATA\com.velocityrl.app"
  RMDir /r "$LOCALAPPDATA\Programs\VelocityRL"

  DetailPrint "Removing installation directory..."
  RMDir /r "$INSTDIR"
!macroend
