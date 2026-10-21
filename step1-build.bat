@echo off
cd /d "%~dp0"
set MICMIX_NOPAUSE=1
call build.bat > build.log 2>&1
powershell -NoProfile -ExecutionPolicy Bypass -Command "$r='HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\MMDevices\Audio\Capture'; Get-ChildItem $r | ForEach-Object { $s=(Get-ItemProperty -LiteralPath $_.PSPath).DeviceState; $p=Get-ItemProperty -LiteralPath (Join-Path $_.PSPath 'Properties') -ErrorAction SilentlyContinue; '{0} | state={1} | {2} | {3}' -f $_.PSChildName,$s,$p.'{a45c254e-df1c-4efd-8020-67d146a850e0},2',$p.'{b3f8fa53-0004-438e-9003-51a46e139bfc},6' }" > mics.txt 2>&1
echo done > step1.done
