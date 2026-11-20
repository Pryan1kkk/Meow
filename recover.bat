@echo off
rem Emergency recovery: restores the built-in Windows Task Manager.
rem Double-click this file. It asks for administrator rights by itself.
net session >nul 2>&1
if %errorlevel% neq 0 (
    powershell -NoProfile -Command "Start-Process -FilePath '%~f0' -Verb RunAs"
    exit /b
)
reg delete "HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion\Image File Execution Options\taskmgr.exe" /f
echo.
echo Done. Ctrl+Shift+Esc opens the standard Task Manager again.
pause
