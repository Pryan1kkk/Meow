# Устанавливает TaskMgrRS и подменяет им встроенный диспетчер задач (Ctrl+Shift+Esc, Ctrl+Alt+Del -> Диспетчер задач).
# Запуск: ПКМ по PowerShell -> «От имени администратора», затем:  .\install.ps1
# Подмена включается ТОЛЬКО после успешной самопроверки (--selftest) установленной копии.
#Requires -RunAsAdministrator

$ErrorActionPreference = 'Stop'

$src = Join-Path $PSScriptRoot 'target\release\TaskMgrRS.exe'
if (-not (Test-Path $src)) {
    throw "Не найден $src. Сначала выполни: cargo build --release"
}

$dstDir = Join-Path $env:ProgramFiles 'TaskMgrRS'
$dst    = Join-Path $dstDir 'TaskMgrRS.exe'
$key    = 'HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion\Image File Execution Options\taskmgr.exe'
$log    = Join-Path $env:APPDATA 'TaskMgrRS\crash.log'

# Если подмена уже стоит, снимаем её на время проверки, чтобы не остаться без диспетчера при неудаче
if (Test-Path $key) { Remove-Item -Path $key -Recurse -Force }

New-Item -ItemType Directory -Force -Path $dstDir | Out-Null
Copy-Item $src $dst -Force

Write-Host "Самопроверка..." -NoNewline
$p = Start-Process -FilePath $dst -ArgumentList '--selftest' -WindowStyle Hidden -PassThru
if (-not $p.WaitForExit(60000)) {
    try { $p.Kill() } catch {}
    $failed = "самопроверка не завершилась за 60 секунд"
} elseif ($p.ExitCode -ne 0) {
    $failed = "самопроверка вернула код $($p.ExitCode)"
}
if ($failed) {
    Write-Host " ОШИБКА" -ForegroundColor Red
    Write-Host "Подмена НЕ установлена, стандартный диспетчер задач остался на месте: $failed" -ForegroundColor Red
    if (Test-Path $log) {
        Write-Host "--- последние строки $log ---"
        Get-Content $log -Tail 15 -Encoding UTF8
    }
    exit 1
}
Write-Host " ок" -ForegroundColor Green

# Перенаправляем запуск taskmgr.exe на наш exe
New-Item -Path $key -Force | Out-Null
New-ItemProperty -Path $key -Name 'Debugger' -Value "`"$dst`"" -PropertyType String -Force | Out-Null

Write-Host "Готово. Теперь Ctrl+Shift+Esc открывает TaskMgrRS." -ForegroundColor Green
Write-Host "Откат: .\uninstall.ps1 или recover.bat (от администратора)."
