# MicMix — контекст проекта для Claude Code

Отвечай пользователю по-русски. Он студент, это его первый проект для GitHub.

## Что строим

Приложение для Windows, которое транслирует звук выбранной вкладки браузера, всего браузера
или выбранных приложений **прямо в микрофон** (в стиле Soundpad, только источник — приложения,
а не файлы), с современным красивым интерфейсом. Собеседники в Discord и играх слышат этот звук
вместе с голосом. В Discord и играх при этом ничего перенастраивать не нужно.

Полный план и решения по стеку описаны в `docs/PLAN.md`.

## Архитектура (решено)

```
[звук приложений по PID] --WASAPI process loopback--+
[вкладка браузера] --расширение MV3 -> WebSocket----+--> движок (Rust) --общая память--> APO-DLL в audiodg.exe
                                                                                         (подмешивает в сигнал микрофона)
запасной режим без APO: движок -> виртуальный кабель (VB-CABLE / VAC Lite) -> игра слушает кабель
```

- **APO** (Audio Processing Object): C++ COM-DLL, которую Windows загружает в `audiodg.exe`
  как эффект SFX на конкретном устройстве записи. Так подмешивает звук и Soundpad.
- Неподписанный APO загружается только при `HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Audio`
  `DisableProtectedAudioDG = 1` (DWORD) и после перезагрузки. На Windows 11 25H2 (26200) это уже
  не работает: там нужна подпись.
- Стек приложения (этапы 1+): **Tauri 2 + Rust** (крейт `wasapi` — именно он, `cpal` не умеет
  захват отдельного процесса), фронт Svelte 5 или React + Tailwind 4. Перед добавлением
  зависимостей проверяй их актуальные версии (пользователь просил так делать всегда).

## Машина пользователя (ноутбук)

- Windows 11 23H2, сборка 22631. `DisableProtectedAudioDG = 1` выставлен, и Process Explorer
  показал, что `audiodg.exe` работает **без защиты** (Protection пустой) — проверено.
- Установлена **Visual Studio Insiders** (не 2026 stable), поэтому `vswhere` нужен с `-prerelease`.
- Микрофоны: встроенный (аудио Senary/Realtek — уточни по списку) и гарнитура HyperX.
  Первые тесты — **только на встроенном**, чтобы HyperX оставался рабочим.

## Текущий статус (этапы 1–2: приложение)

Структура: cargo-workspace в корне (`Cargo.toml`, общий `target\`), члены `engine` и `app/src-tauri`.

- `src/MicMixApo.cpp` — APO: читает звук из общей памяти и подмешивает в микрофон (сам не пищит).
  CLSID `{04206D4E-6467-414B-B052-6AA2220324A5}` (тот же в `install-apo.ps1` и `engine/src/mics.rs`).
- Общая память `Global\MicMixApo.Ring.v1` (раскладка — `src/MicMixShared.h` = `engine/src/ring.rs`):
  кольцо float32 стерео на 16384 кадра, движок пишет и двигает `writeFrame`, каждый поток APO читает
  со своей позиции (запас 20 мс, задержка срезается после 80 мс). Секцию создаёт **APO** (у службы есть
  право на `Global\`), движок только открывает и раз в секунду пробует, пока её нет. APO пишет в
  `micSampleRate` частоту микрофона, движок захватывает приложения сразу в ней (WASAPI autoconvert) —
  ресемплер не нужен. Если частоты всё же разные, APO не подмешивает (пишет в `UnlockForProcess`).
- `engine/` — ядро `micmix_core` (`wasapi 0.26`, `windows 0.62`, `png`, `serde`):
  `engine.rs` — микшер (поток захвата на приложение + поток микшера раз в 5 мс, громкость/mute на источник,
  общая громкость, лимитер −1 дБFS, уровни, тестовый бип = источник `micmix:tone`, перезапуск захвата
  при перезапуске приложения); `apps.rs` — приложения со звуковыми сессиями, иконки (ExtractIconEx → PNG),
  выбор процесса для захвата (процесс с сессией → самый верхний предок с тем же именем);
  `mics.rs` — микрофоны и стоит ли APO (реестр); `config.rs` — `%APPDATA%\MicMix\config.json`.
  Консоль: `target\release\micmix-engine.exe apps|mics|tone|capture <exe>...`.
- `app/` — Tauri 2.12 + Svelte 5 + Vite 8 + Tailwind 4. `app/src-tauri/src/main.rs`: команды, трей
  (открыть / mute / выход), крестик прячет в трей, глобальная клавиша mute (по умолчанию `Ctrl+Alt+M`; старая `Ctrl+Shift+M` конфликтует с Discord и при занятости
  автоматически меняется; сменить — `set_hotkey`, в интерфейсе `Hotkey.svelte`), одна копия — мьютекс `Local\MicMix.App`
  (два движка портили бы общую память),
  `apo_install` запускает `install-apo.ps1 -Quiet -EndpointId {guid} [-Uninstall]` через UAC.
  Интерфейс: `app/src/App.svelte` + `app/src/lib/*`. Иконка рисуется `app/make-icon.ps1`.
  Запуск: `cd app; npm run tauri dev`, сборка установщика: `npm run tauri build` (NSIS, ресурсы —
  `install-apo.ps1` и `build\MicMixApo.dll`, поэтому сначала `build.bat`).
- Rust у пользователя: 1.99.1 из .msi, подключён в rustup как тулчейн `msi` (по умолчанию). Сеть медленная
  (~100 КБ/с), rustup-загрузки зависали — новые тулчейны/компоненты лучше не качать без нужды.
  `cargo fetch` без `--target x86_64-pc-windows-msvc` качает и линуксовые крейты — не надо.
- `build.bat` собирает `build\MicMixApo.dll` и `build\captest.exe` (MSVC, /MT).
- `install-apo.ps1` — универсальный установщик на любой микрофон: ставит наш CLSID первым
  в `PKEY_CompositeFX_StreamEffectClsid` (`{d04e05a6-594b-4fb6-a80d-01af5eed7d1d},13`),
  при пустом legacy SFX (`...,5`) пишет туда тоже, включает улучшения
  (`{1da5d803-d492-4edd-8c23-e0c0ffee7f0e},5` = 0), исходные значения сохраняет в
  `C:\ProgramData\MicMixApo\<endpoint-id>.json`, а `-Uninstall` восстанавливает их.
- **Этап 0 пройден:** бип работал на встроенном «Набор микрофонов» (Intel Smart Sound, 2 ch / 48 кГц float),
  проверено 2026-10-22. Причина прежнего «бипа нет»: `audiodg` создаёт APO через **COM-агрегацию**
  (`CreateInstance(pUnkOuter != NULL, IID_IUnknown)`), а фабрика возвращала `CLASS_E_NOAGGREGATION`.
  Теперь у объекта есть неделегирующий `IUnknown` (`m_inner`), остальные интерфейсы делегируют outer.
- Журнал `C:\ProgramData\MicMixApo\apo.log` пишет COM-вызовы (`DllGetClassObject`, `CreateInstance`,
  промахи `QueryInterface`: запросы IAudioSystemEffects2/3 и т. п. без ответа — это нормально),
  формат, LockForProcess, первый APOProcess.
- `build\captest.exe "<часть имени микрофона>" [default|raw|comms]` (собирается в `build.bat`) пишет
  3 с через WASAPI и ищет 880 Гц: проверка без пользователя (запусти рядом `micmix-engine tone`).
  В RAW бипа нет (эффекты обходятся), это ожидаемо.

## Как работать с установщиком из Claude Code

Claude Code работает без прав администратора, а установщику они нужны. Запускай его так
(пользователь подтвердит окно UAC), потом читай логи:

```powershell
Start-Process powershell -Verb RunAs -Wait -ArgumentList '-NoProfile -ExecutionPolicy Bypass -File "<путь>\install-apo.ps1" -Device "<часть имени микрофона>"'
Get-Content .\logs\install-last.log   # весь вывод установщика (Start-Transcript)
Get-Content .\logs\apo.log            # копия журнала DLL на момент запуска
Get-Content C:\ProgramData\MicMixApo\apo.log   # живой журнал DLL (читается без админа)
```

- Без `-Device` скрипт спрашивает номер через `Read-Host`. В неинтерактивном запуске
  **всегда передавай `-Device`**. Список микрофонов можно получить без админа:
  `Get-ChildItem HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\MMDevices\Audio\Capture`
  (`DeviceState` = 1 — активен, имя в `Properties` → `{a45c254e-df1c-4efd-8020-67d146a850e0},2`,
  адаптер в `{b3f8fa53-0004-438e-9003-51a46e139bfc},6`).
- `-Status` показывает FX-значения микрофона и хвост журнала DLL. `-Uninstall -Device ...` снимает APO.
- Установка и удаление перезапускают службу `Audiosrv`: на пару секунд пропадает звук.
- Для проверки бипа запускай `build\captest.exe` (режимы default и comms должны дать «БИП ЕСТЬ»).
  Итоговую проверку на слух делает пользователь в «Звукозаписи» Windows.

## Порядок диагностики «бипа нет»

1. Журнал пустой → DLL вообще не загружается. Проверь `-Status`: COM-регистрацию, путь к DLL,
   FX-значения. Попробуй поставить CLSID в MFX (`...,14` composite / `...,6` legacy) вместо SFX.
   Проверь, нет ли у устройства `PKEY_SFX_ProcessingModes_Supported_For_Streaming`
   (`{d3993a3f-99c2-4402-b5ec-a92a0367664b},5`) без режима DEFAULT.
2. Есть `IsInputFormatSupported ... hr=0x887D0003` → предлагают не float32; поддержи формат или
   верни предложенный float-формат.
3. Есть `APOProcess running`, но бипа нет → программа записи открывает микрофон в RAW-режиме
   и обходит эффекты, либо наш эффект стоит не в том режиме. Проверь в другой программе.

## Правила

- Код в аудиопотоке (`APOProcess`) должен быть real-time safe: без выделения памяти,
  блокировок и файлового ввода-вывода (единственное исключение — одна строка журнала при первом вызове).
- Ничего не меняй в реестре вне ключей конкретного микрофона без бэкапа и согласия пользователя.
- `.ps1` сохраняй в UTF-8 **с BOM** (Windows PowerShell 5.1 иначе портит кириллицу), `.bat` — с CRLF.
