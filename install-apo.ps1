#Requires -RunAsAdministrator
<#
.SYNOPSIS
    Ставит или снимает MicMix APO на любой микрофон Windows.

.DESCRIPTION
    Скрипт показывает список активных микрофонов, ставит APO на выбранный,
    сохраняет исходные настройки и умеет вернуть всё как было.

.EXAMPLE
    .\install-apo.ps1                    # выбрать микрофон из списка и поставить
.EXAMPLE
    .\install-apo.ps1 -Device "HyperX"   # поставить на микрофон, в имени которого есть "HyperX"
.EXAMPLE
    .\install-apo.ps1 -Uninstall         # снять с выбранного микрофона и восстановить настройки
.EXAMPLE
    .\install-apo.ps1 -Status            # показать, где стоит APO и загружен ли он
#>
param(
    [string]$Device,
    [switch]$Uninstall,
    [switch]$Status,
    [string]$DllPath = (Join-Path $PSScriptRoot 'build\MicMixApo.dll'),
    # GUID микрофона из MMDevices\Audio\Capture — точный выбор (так вызывает приложение MicMix).
    [string]$EndpointId,
    # Без вопросов в консоли: защиту аудиодвижка снимает сам, код возврата 0/1 (для приложения).
    [switch]$Quiet
)

$ErrorActionPreference = 'Stop'

# --- Константы -------------------------------------------------------------
$Clsid      = '{04206D4E-6467-414B-B052-6AA2220324A5}'   # совпадает с CLSID в MicMixApo.cpp
$DllName    = 'MicMixApo.dll'
$InstallDir = Join-Path $env:ProgramFiles 'MicMixApo'
$DataDir    = Join-Path $env:ProgramData  'MicMixApo'
$CapRoot    = 'SOFTWARE\Microsoft\Windows\CurrentVersion\MMDevices\Audio\Capture'

$PK_SFX          = '{d04e05a6-594b-4fb6-a80d-01af5eed7d1d},5'   # PKEY_FX_StreamEffectClsid
$PK_CompositeSFX = '{d04e05a6-594b-4fb6-a80d-01af5eed7d1d},13'  # PKEY_CompositeFX_StreamEffectClsid
$PK_Association  = '{d04e05a6-594b-4fb6-a80d-01af5eed7d1d},0'   # PKEY_FX_Association: к какому узлу применять
$AnyNode         = '{00000000-0000-0000-0000-000000000000}'     # KSNODETYPE_ANY
$PK_SfxModes     = '{d3993a3f-99c2-4402-b5ec-a92a0367664b},5'   # PKEY_SFX_ProcessingModes_Supported_For_Streaming
$ModeDefault     = '{C18E2F7E-933D-4965-B7D1-1EEF228D2AF3}'     # AUDIO_SIGNALPROCESSINGMODE_DEFAULT
$PK_DisableSysFx = '{1da5d803-d492-4edd-8c23-e0c0ffee7f0e},5'   # «Отключить все улучшения»
$PK_FriendlyName = '{a45c254e-df1c-4efd-8020-67d146a850e0},2'   # «Микрофон»
$PK_AdapterName  = '{b3f8fa53-0004-438e-9003-51a46e139bfc},6'   # «Realtek(R) Audio», «HyperX ...»

$HKLM = [Microsoft.Win32.Registry]::LocalMachine

# --- Права на ключи аудиоустройств -----------------------------------------
# Ключи MMDevices часто принадлежат системе, и даже администратор может только читать.
# Тогда временно включаем привилегию владельца и выдаём Администраторам полный доступ.
if (-not ([System.Management.Automation.PSTypeName]'MicMixTokenPriv').Type) {
    Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class MicMixTokenPriv {
    [StructLayout(LayoutKind.Sequential, Pack = 4)]
    struct TokPriv { public int Count; public long Luid; public int Attr; }
    [DllImport("advapi32.dll", SetLastError = true)]
    static extern bool OpenProcessToken(IntPtr h, int access, out IntPtr token);
    [DllImport("advapi32.dll", SetLastError = true)]
    static extern bool LookupPrivilegeValue(string sys, string name, out long luid);
    [DllImport("advapi32.dll", SetLastError = true)]
    static extern bool AdjustTokenPrivileges(IntPtr token, bool disableAll, ref TokPriv state, int len, IntPtr prev, IntPtr retLen);
    [DllImport("kernel32.dll")]
    static extern IntPtr GetCurrentProcess();
    public static bool Enable(string name) {
        IntPtr token;
        if (!OpenProcessToken(GetCurrentProcess(), 0x28, out token)) return false;  // ADJUST_PRIVILEGES | QUERY
        TokPriv tp = new TokPriv(); tp.Count = 1; tp.Attr = 2;                      // SE_PRIVILEGE_ENABLED
        if (!LookupPrivilegeValue(null, name, out tp.Luid)) return false;
        return AdjustTokenPrivileges(token, false, ref tp, 0, IntPtr.Zero, IntPtr.Zero);
    }
}
'@
}

function Grant-AdminAccess([string]$SubPath) {
    Write-Host "  Выдаю Администраторам доступ к ключу ...\$(Split-Path $SubPath -Leaf)" -ForegroundColor DarkGray
    [void][MicMixTokenPriv]::Enable('SeTakeOwnershipPrivilege')
    [void][MicMixTokenPriv]::Enable('SeRestorePrivilege')
    $admins = New-Object System.Security.Principal.SecurityIdentifier('S-1-5-32-544')
    $rw = [Microsoft.Win32.RegistryKeyPermissionCheck]::ReadWriteSubTree

    $k = $HKLM.OpenSubKey($SubPath, $rw, [System.Security.AccessControl.RegistryRights]::TakeOwnership)
    $acl = $k.GetAccessControl([System.Security.AccessControl.AccessControlSections]::None)
    $acl.SetOwner($admins); $k.SetAccessControl($acl); $k.Close()

    $k = $HKLM.OpenSubKey($SubPath, $rw, [System.Security.AccessControl.RegistryRights]::ChangePermissions)
    $acl = $k.GetAccessControl()
    $rule = New-Object System.Security.AccessControl.RegistryAccessRule(
        $admins, 'FullControl', 'ContainerInherit', 'None', 'Allow')
    $acl.AddAccessRule($rule); $k.SetAccessControl($acl); $k.Close()
}

# Открывает ключ на запись (при нехватке прав — выдаёт их). $null, если ключа нет.
function Open-Writable([string]$SubPath) {
    try {
        $probe = $HKLM.OpenSubKey($SubPath, $false)
        if ($null -eq $probe) { return $null }
        $probe.Close()
        return $HKLM.OpenSubKey($SubPath, $true)
    }
    catch {
        Grant-AdminAccess $SubPath
        return $HKLM.OpenSubKey($SubPath, $true)
    }
}

function Open-OrCreateWritable([string]$ParentPath, [string]$Name) {
    $k = Open-Writable "$ParentPath\$Name"
    if ($k) { return $k }
    $parent = Open-Writable $ParentPath
    if (-not $parent) { throw "Не найден ключ $ParentPath" }
    try { return $parent.CreateSubKey($Name) } finally { $parent.Close() }
}

# --- Сохранение и восстановление значений -----------------------------------
function Get-Entry($Key, [string]$Name) {
    if ($Key -and ($Key.GetValueNames() -contains $Name)) {
        [pscustomobject]@{
            Name    = $Name
            Existed = $true
            Kind    = [string]$Key.GetValueKind($Name)
            Data    = $Key.GetValue($Name, $null, 'DoNotExpandEnvironmentNames')
        }
    } else {
        [pscustomobject]@{ Name = $Name; Existed = $false; Kind = $null; Data = $null }
    }
}

function Restore-Entry($Key, $Entry) {
    if (-not $Entry.Existed) { $Key.DeleteValue($Entry.Name, $false); return }
    $kind = [Microsoft.Win32.RegistryValueKind]$Entry.Kind
    # Без switch: его вывод разворачивает массивы, и список из одного элемента стал бы строкой.
    if     ($kind -eq 'MultiString') { [string[]]$data = @($Entry.Data) }
    elseif ($kind -eq 'Binary')      { [byte[]]$data   = @($Entry.Data) }
    elseif ($kind -eq 'DWord')       { $data = [int]$Entry.Data }
    elseif ($kind -eq 'QWord')       { $data = [long]$Entry.Data }
    else                             { $data = [string]$Entry.Data }
    $Key.SetValue($Entry.Name, $data, $kind)
}

# --- Микрофоны ---------------------------------------------------------------
function Get-CaptureEndpoints {
    $root = $HKLM.OpenSubKey($CapRoot)
    foreach ($id in $root.GetSubKeyNames()) {
        $ep = $HKLM.OpenSubKey("$CapRoot\$id")
        $state = $ep.GetValue('DeviceState')
        $props = $HKLM.OpenSubKey("$CapRoot\$id\Properties")
        $fx    = $HKLM.OpenSubKey("$CapRoot\$id\FxProperties")
        $sfx = @()
        if ($fx) { $sfx = @($fx.GetValue($PK_CompositeSFX)) + @($fx.GetValue($PK_SFX)) | Where-Object { $_ } }
        [pscustomobject]@{
            Id        = $id
            Active    = ($state -eq 1)
            Name      = if ($props) { $props.GetValue($PK_FriendlyName) } else { '?' }
            Adapter   = if ($props) { $props.GetValue($PK_AdapterName) } else { '?' }
            Installed = ($sfx -contains $Clsid)
            HasBackup = Test-Path (Join-Path $DataDir "$id.json")
        }
    }
}

function Select-Endpoint($List, [string]$Filter, [string]$Prompt) {
    $List = @($List)
    if ($EndpointId) {
        $hit = @($List | Where-Object { $_.Id -eq $EndpointId })
        if ($hit.Count -eq 0) { throw "Не нашёл микрофон $EndpointId." }
        return $hit[0]
    }
    if ($Filter) {
        $List = @($List | Where-Object { "$($_.Name) $($_.Adapter)" -like "*$Filter*" })
        if ($List.Count -eq 0) { throw "Не нашёл микрофон, в имени которого есть «$Filter»." }
        if ($List.Count -eq 1) { return $List[0] }
    }
    if ($List.Count -eq 0) { throw 'Подходящих микрофонов не найдено.' }
    if ($Quiet) { throw 'Подходит несколько микрофонов — укажи -EndpointId.' }
    Write-Host ''
    for ($i = 0; $i -lt $List.Count; $i++) {
        $mark = if ($List[$i].Installed) { '  [APO стоит]' } else { '' }
        Write-Host ('  [{0}] {1} — {2}{3}' -f ($i + 1), $List[$i].Name, $List[$i].Adapter, $mark)
    }
    $n = 0
    if (-not [int]::TryParse((Read-Host "`n$Prompt"), [ref]$n) -or $n -lt 1 -or $n -gt $List.Count) {
        throw 'Неверный номер.'
    }
    return $List[$n - 1]
}

# --- Служба звука и проверка защиты -----------------------------------------
function Stop-Audio  { Write-Host 'Останавливаю службу звука...'; Stop-Service Audiosrv -Force; Start-Sleep -Milliseconds 800 }
function Start-Audio { Write-Host 'Запускаю службу звука...';     Start-Service Audiosrv }

function Test-ProtectedAudioDG {
    $v = (Get-ItemProperty 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Audio' `
            -Name DisableProtectedAudioDG -ErrorAction SilentlyContinue).DisableProtectedAudioDG
    if ($v -eq 1) { return }
    Write-Warning 'Защита аудиодвижка включена: Windows не загрузит неподписанную DLL.'
    if ($Quiet -or (Read-Host 'Отключить защиту (DisableProtectedAudioDG = 1)? Потребуется перезагрузка. [y/n]') -eq 'y') {
        # Только создать ключ, если его нет: New-Item -Force на реестре стирает ключ целиком —
        # вместе с системными настройками звука, которые там лежат.
        $audioKey = 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Audio'
        if (-not (Test-Path $audioKey)) { New-Item $audioKey | Out-Null }
        New-ItemProperty $audioKey -Name DisableProtectedAudioDG -PropertyType DWord -Value 1 -Force | Out-Null
        Write-Host 'Готово. После установки перезагрузи компьютер.' -ForegroundColor Yellow
    }
}

function Show-Status {
    $v = (Get-ItemProperty 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Audio' `
            -Name DisableProtectedAudioDG -ErrorAction SilentlyContinue).DisableProtectedAudioDG
    Write-Host "`nDisableProtectedAudioDG = $v" -ForegroundColor Cyan
    $dll = Join-Path $InstallDir $DllName
    Write-Host ("DLL в Program Files: {0}" -f $(if (Test-Path $dll) { 'есть' } else { 'НЕТ' }))
    $com = $HKLM.OpenSubKey("SOFTWARE\Classes\CLSID\$Clsid\InprocServer32")
    Write-Host ("COM-регистрация: {0}" -f $(if ($com) { $com.GetValue('') } else { 'НЕТ' }))

    Write-Host "`nМикрофоны:" -ForegroundColor Cyan
    Get-CaptureEndpoints | Where-Object Active | ForEach-Object {
        $s = if ($_.Installed) { 'APO стоит' } else { '—' }
        Write-Host ('  {0} — {1}: {2}' -f $_.Name, $_.Adapter, $s)
        if ($_.Installed) {
            $fx = $HKLM.OpenSubKey("$CapRoot\$($_.Id)\FxProperties")
            $pr = $HKLM.OpenSubKey("$CapRoot\$($_.Id)\Properties")
            foreach ($name in $fx.GetValueNames()) {
                $val = $fx.GetValue($name)
                Write-Host ('      FX {0} = {1}' -f $name, (@($val) -join '; ')) -ForegroundColor DarkGray
            }
            Write-Host ('      DisableSysFx = {0}' -f $pr.GetValue($PK_DisableSysFx)) -ForegroundColor DarkGray
        }
    }

    Write-Host "`nЖурнал DLL ($DataDir\apo.log):" -ForegroundColor Cyan
    $log = Join-Path $DataDir 'apo.log'
    if (Test-Path $log) { Get-Content $log -Tail 40 | ForEach-Object { Write-Host "  $_" } }
    else { Write-Host '  пусто — Windows ни разу не загрузила DLL' -ForegroundColor Yellow }
    Write-Host "`nАудиодвижок (audiodg.exe):" -ForegroundColor Cyan
    $p = Get-Process audiodg -ErrorAction SilentlyContinue
    if (-not $p) { Write-Host '  не запущен — включи любой звук или запись и повтори'; return }
    try {
        $loaded = $p.Modules | Where-Object ModuleName -eq $DllName
        if ($loaded) { Write-Host "  $DllName загружена ✔" -ForegroundColor Green }
        else { Write-Host "  $DllName не загружена (открой запись с микрофона, например «Звукозапись»)" }
    } catch {
        Write-Host '  нет доступа к процессу — защита аудиодвижка, скорее всего, включена' -ForegroundColor Yellow
    }
}

# --- Регистрация COM ---------------------------------------------------------
function Register-Com([string]$Path) {
    $k = $HKLM.CreateSubKey("SOFTWARE\Classes\CLSID\$Clsid")
    $k.SetValue('', 'MicMix APO'); $k.Close()
    $k = $HKLM.CreateSubKey("SOFTWARE\Classes\CLSID\$Clsid\InprocServer32")
    $k.SetValue('', $Path); $k.SetValue('ThreadingModel', 'Both'); $k.Close()

    $k = $HKLM.CreateSubKey("SOFTWARE\Classes\AudioEngine\AudioProcessingObjects\$Clsid")
    $k.SetValue('FriendlyName', 'MicMix APO')
    $k.SetValue('Copyright', 'MicMix (open source)')
    foreach ($pair in @(@('MajorVersion', 0), @('MinorVersion', 1), @('Flags', 0xF),
                        @('MinInputConnections', 1), @('MaxInputConnections', 1),
                        @('MinOutputConnections', 1), @('MaxOutputConnections', 1),
                        @('MaxInstances', -1), @('NumAPOInterfaces', 1))) {
        $k.SetValue($pair[0], [int]$pair[1], 'DWord')
    }
    $k.SetValue('APOInterface0', '{FD7F2B29-24D0-4B5C-B177-592C39F9CA10}')  # IID_IAudioProcessingObject
    $k.Close()
}

function Unregister-Com {
    $HKLM.DeleteSubKeyTree("SOFTWARE\Classes\CLSID\$Clsid", $false)
    $HKLM.DeleteSubKeyTree("SOFTWARE\Classes\AudioEngine\AudioProcessingObjects\$Clsid", $false)
}

# --- Установка ---------------------------------------------------------------
function Install-Apo {
    if (-not (Test-Path $DllPath)) { throw "Не найден $DllPath. Сначала запусти build.bat." }
    Test-ProtectedAudioDG

    $ep = Select-Endpoint (Get-CaptureEndpoints | Where-Object Active) $Device 'На какой микрофон поставить APO? Номер'
    Write-Host "`nСтавлю на: $($ep.Name) — $($ep.Adapter)" -ForegroundColor Cyan
    $epPath = "$CapRoot\$($ep.Id)"

    New-Item -ItemType Directory -Force $InstallDir, $DataDir | Out-Null
    # audiodg.exe работает от LOCAL SERVICE — разрешаем ему писать журнал в $DataDir.
    & icacls $DataDir /grant '*S-1-5-19:(OI)(CI)M' | Out-Null
    Remove-Item (Join-Path $DataDir 'apo.log') -ErrorAction SilentlyContinue
    $target = Join-Path $InstallDir $DllName

    Stop-Audio
    try {
        Copy-Item $DllPath $target -Force
        Register-Com $target

        $fx    = Open-OrCreateWritable $epPath 'FxProperties'
        $props = Open-Writable "$epPath\Properties"

        # Исходные значения сохраняем один раз — повторная установка их не перезапишет.
        $backup = Join-Path $DataDir "$($ep.Id).json"
        if (-not (Test-Path $backup)) {
            @{
                Device = "$($ep.Name) — $($ep.Adapter)"
                Fx     = @((Get-Entry $fx $PK_SFX), (Get-Entry $fx $PK_CompositeSFX), (Get-Entry $fx $PK_Association), (Get-Entry $fx $PK_SfxModes))
                Props  = @((Get-Entry $props $PK_DisableSysFx))
            } | ConvertTo-Json -Depth 5 | Set-Content -Encoding UTF8 $backup
        } else {
            # Копия от старой версии установщика без новых ключей — дописываем их исходные значения.
            $saved = Get-Content -Raw -Encoding UTF8 $backup | ConvertFrom-Json
            $changed = $false
            foreach ($name in $PK_Association, $PK_SfxModes) {
                if (-not (@($saved.Fx) | Where-Object { $_.Name -eq $name })) {
                    $saved.Fx = @($saved.Fx) + (Get-Entry $fx $name)
                    $changed = $true
                }
            }
            if ($changed) { $saved | ConvertTo-Json -Depth 5 | Set-Content -Encoding UTF8 $backup }
        }

        # У микрофонов без своих эффектов (USB-микрофоны на стандартном драйвере) нет ни привязки
        # эффектов к узлу устройства, ни списка режимов — без них Windows молча пропускает APO.
        # Прописываем «любой узел» и режим DEFAULT, как это делает Equalizer APO.
        if (-not $fx.GetValue($PK_Association)) { $fx.SetValue($PK_Association, $AnyNode, 'String') }
        if (-not $fx.GetValue($PK_SfxModes)) { $fx.SetValue($PK_SfxModes, [string[]]@($ModeDefault), 'MultiString') }

        # Ставим наш APO первым в цепочку, эффекты драйвера (если были) оставляем после него.
        $chain = @($fx.GetValue($PK_CompositeSFX)) | Where-Object { $_ }
        if (-not $chain) { $chain = @($fx.GetValue($PK_SFX)) | Where-Object { $_ } }
        $chain = @($Clsid) + @($chain | Where-Object { $_ -ne $Clsid })
        $fx.SetValue($PK_CompositeSFX, [string[]]$chain, 'MultiString')
        if (-not $fx.GetValue($PK_SFX)) { $fx.SetValue($PK_SFX, $Clsid, 'String') }
        $fx.Close()

        # Включаем «улучшения звука» для этого микрофона, иначе Windows пропустит APO.
        if ($props) { $props.SetValue($PK_DisableSysFx, 0, 'DWord'); $props.Close() }
    }
    finally { Start-Audio }

    Write-Host "`nГотово." -ForegroundColor Green
    Write-Host 'Звук приложений пойдёт в этот микрофон, пока запущен MicMix.'
    Write-Host 'Проверка: «Тестовый бип» в MicMix и запись в «Звукозаписи». Состояние: .\install-apo.ps1 -Status'
}

# --- Удаление ----------------------------------------------------------------
function Uninstall-Apo {
    $list = Get-CaptureEndpoints | Where-Object { $_.Installed -or $_.HasBackup }
    $ep = Select-Endpoint $list $Device 'С какого микрофона снять APO? Номер'
    Write-Host "`nСнимаю с: $($ep.Name) — $($ep.Adapter)" -ForegroundColor Cyan
    $epPath = "$CapRoot\$($ep.Id)"
    $backup = Join-Path $DataDir "$($ep.Id).json"

    Stop-Audio
    try {
        $fx    = Open-Writable "$epPath\FxProperties"
        $props = Open-Writable "$epPath\Properties"
        if (Test-Path $backup) {
            $saved = Get-Content -Raw -Encoding UTF8 $backup | ConvertFrom-Json
            if ($fx)    { foreach ($e in @($saved.Fx))    { Restore-Entry $fx $e } }
            if ($props) { foreach ($e in @($saved.Props)) { Restore-Entry $props $e } }
            Remove-Item $backup -Force
        } elseif ($fx) {
            # Резервной копии нет — просто убираем себя из цепочки.
            $chain = @($fx.GetValue($PK_CompositeSFX)) | Where-Object { $_ -and $_ -ne $Clsid }
            if ($chain) { $fx.SetValue($PK_CompositeSFX, [string[]]$chain, 'MultiString') }
            else { $fx.DeleteValue($PK_CompositeSFX, $false) }
            if ($fx.GetValue($PK_SFX) -eq $Clsid) { $fx.DeleteValue($PK_SFX, $false) }
        }
        if ($fx) { $fx.Close() }
        if ($props) { $props.Close() }

        # Если APO больше нигде не стоит — убираем регистрацию и файлы.
        if (-not (Get-CaptureEndpoints | Where-Object Installed)) {
            Unregister-Com
            Remove-Item $InstallDir -Recurse -Force -ErrorAction SilentlyContinue
            Write-Host 'APO больше нигде не стоит — регистрация и файлы удалены.'
        }
    }
    finally { Start-Audio }
    Write-Host "`nГотово, настройки микрофона восстановлены." -ForegroundColor Green
}

# --- Точка входа -------------------------------------------------------------
# Весь вывод дублируется в logs\install-last.log: так результат можно прочитать,
# даже если скрипт запускали в отдельном окне администратора (например, из Claude Code).
$LogDir = Join-Path $PSScriptRoot 'logs'
New-Item -ItemType Directory -Force $LogDir | Out-Null
try { Start-Transcript -Path (Join-Path $LogDir 'install-last.log') -Force | Out-Null } catch { }

$exitCode = 0
try {
    if ($Status)        { Show-Status }
    elseif ($Uninstall) { Uninstall-Apo }
    else                { Install-Apo }
} catch {
    Write-Host "`nОшибка: $($_.Exception.Message)" -ForegroundColor Red
    $exitCode = 1
} finally {
    $apoLog = Join-Path $DataDir 'apo.log'
    if (Test-Path $apoLog) { Copy-Item $apoLog (Join-Path $LogDir 'apo.log') -Force -ErrorAction SilentlyContinue }
    try { Stop-Transcript | Out-Null } catch { }
}
# Без exit: при запуске через install.bat окно должно остаться открытым (-NoExit).
$global:LASTEXITCODE = $exitCode
if ($Quiet) { exit $exitCode }
