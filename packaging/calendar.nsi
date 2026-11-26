Unicode True
Name "Calendar"
OutFile "..\Calendar-Windows-x64.exe"
InstallDir "$LOCALAPPDATA\Calendar"
RequestExecutionLevel user
Icon "..\assets\app-icon\calendar.ico"

Page directory
Page instfiles
UninstPage uninstConfirm
UninstPage instfiles

Section
  SetOutPath "$INSTDIR"
  File /oname=Calendar.exe "..\target\release\calendar-gpui.exe"
  File /oname=Mulish-OFL.txt "..\assets\fonts\OFL.txt"
  WriteUninstaller "$INSTDIR\Uninstall.exe"
  CreateShortcut "$SMPROGRAMS\Calendar.lnk" "$INSTDIR\Calendar.exe"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\CalendarGPUI" "DisplayName" "Calendar"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\CalendarGPUI" "UninstallString" '"$INSTDIR\Uninstall.exe"'
SectionEnd

Section "Uninstall"
  Delete "$SMPROGRAMS\Calendar.lnk"
  Delete "$INSTDIR\Calendar.exe"
  Delete "$INSTDIR\Mulish-OFL.txt"
  Delete "$INSTDIR\Uninstall.exe"
  RMDir "$INSTDIR"
  DeleteRegKey HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\CalendarGPUI"
SectionEnd
