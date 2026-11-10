; Probe only this installation's executables. OPEN_EXISTING never creates or
; truncates a file; no bytes are written. Windows denies write access to a
; running executable, so an active MCP client or desktop remains untouched.
!macro EnogramRequireExecutableAvailable executablePath productName closeInstruction
  !define enogramProbeId ${__COUNTER__}
  Push $0
  IfFileExists "${executablePath}" 0 enogramProbeDone_${enogramProbeId}
  System::Call 'kernel32::CreateFileW(w "${executablePath}", i 0x40000000, i 7, p 0, i 3, i 0x80, p 0) p .r0'
  ${If} $0 == -1
    DetailPrint "${productName} is in use or its executable is not writable. ${closeInstruction}"
    IfSilent +2 0
      MessageBox MB_OK|MB_ICONEXCLAMATION "${productName} is in use or its executable is not writable.$\r$\n$\r$\n${closeInstruction}$\r$\n$\r$\nIf it is already closed, check write access to:$\r$\n${executablePath}"
    Pop $0
    SetErrorLevel 2
    Abort
  ${EndIf}
  System::Call 'kernel32::CloseHandle(p r0)'
  enogramProbeDone_${enogramProbeId}:
  Pop $0
  !undef enogramProbeId
!macroend

; Tauri includes utils.nsh before this hook. Replace its force-close behavior,
; including silent installs, so even the later desktop check never kills work.
!ifmacrondef CheckIfAppIsRunning
  !error "Enogram requires the Tauri NSIS process-check macro before installer hooks. Review the updated Tauri template."
!endif
!macroundef CheckIfAppIsRunning
!macro CheckIfAppIsRunning executablePath productName
  !insertmacro EnogramRequireExecutableAvailable "${executablePath}" "${productName}" "Close Enogram and retry setup."
!macroend

!macro EnogramRequireInstallationAvailable
  !insertmacro EnogramRequireExecutableAvailable "$INSTDIR\enogram-mcp.exe" "Enogram MCP" "Disconnect the Enogram MCP server in your MCP clients and retry setup."
  !insertmacro EnogramRequireExecutableAvailable "$INSTDIR\${MAINBINARYNAME}.exe" "Enogram" "Close Enogram and retry setup."
!macroend

!macro NSIS_HOOK_PREINSTALL
  !insertmacro EnogramRequireInstallationAvailable
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  !insertmacro EnogramRequireInstallationAvailable
!macroend
