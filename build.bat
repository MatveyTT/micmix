@echo off
setlocal
cd /d "%~dp0"

set "VSWHERE=%ProgramFiles(x86)%\Microsoft Visual Studio\Installer\vswhere.exe"
if not exist "%VSWHERE%" (
    echo [!] Visual Studio not found. Install Visual Studio with "Desktop development with C++".
    goto :fail
)

rem -prerelease: also finds Visual Studio Insiders / Preview
set "VSDIR="
for /f "usebackq tokens=*" %%i in (`"%VSWHERE%" -latest -prerelease -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath`) do set "VSDIR=%%i"
if not defined VSDIR (
    echo [!] Visual Studio found, but C++ tools are missing.
    echo     Open Visual Studio Installer - Modify - check "Desktop development with C++".
    goto :fail
)
echo Using: %VSDIR%

call "%VSDIR%\VC\Auxiliary\Build\vcvars64.bat" >nul
if errorlevel 1 (
    echo [!] vcvars64.bat failed.
    goto :fail
)

if not exist build mkdir build
cl /nologo /LD /EHsc /O2 /W4 /std:c++17 /utf-8 /MT /DUNICODE /D_UNICODE ^
   src\MicMixApo.cpp /Fobuild\ /Fe:build\MicMixApo.dll ^
   /link /DEF:src\MicMixApo.def ole32.lib
if errorlevel 1 (
    echo [!] Build failed - see errors above.
    goto :fail
)
rem Test tool: records 3 s from a mic and looks for the 880 Hz beep
cl /nologo /EHsc /O2 /W4 /std:c++17 /utf-8 /MT /DUNICODE /D_UNICODE ^
   tools\captest.cpp /Fobuild\ /Fe:build\captest.exe /link ole32.lib
if errorlevel 1 (
    echo [!] captest build failed - see errors above.
    goto :fail
)
echo.
echo [OK] build\MicMixApo.dll is ready. Next: install.bat
if not defined MICMIX_NOPAUSE pause
exit /b 0

:fail
echo.
if not defined MICMIX_NOPAUSE pause
exit /b 1
