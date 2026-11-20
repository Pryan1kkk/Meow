# Возвращает встроенный диспетчер задач Windows.
#Requires -RunAsAdministrator

$key = 'HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion\Image File Execution Options\taskmgr.exe'
if (Test-Path $key) {
    Remove-Item -Path $key -Recurse -Force
    Write-Host "Подмена снята: снова открывается стандартный диспетчер задач." -ForegroundColor Green
} else {
    Write-Host "Подмена не была установлена."
}

$dstDir = Join-Path $env:ProgramFiles 'TaskMgrRS'
if (Test-Path $dstDir) {
    Remove-Item $dstDir -Recurse -Force -ErrorAction SilentlyContinue
}
