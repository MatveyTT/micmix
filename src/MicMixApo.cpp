// MicMixApo — APO (Audio Processing Object) для микрофона.
//
// Windows загружает DLL в аудиодвижок (audiodg.exe) как эффект микрофона.
// APO читает звук приложений, который движок MicMix пишет в общую память
// (см. MicMixShared.h), и подмешивает его в сигнал микрофона.
//
// Сборка: build.bat (нужна Visual Studio с C++ и Windows SDK).

#define WIN32_LEAN_AND_MEAN
#include <windows.h>
#include <objbase.h>
#include <mmreg.h>
#include <audioenginebaseapo.h>
#include <audiomediatype.h>
#include <sddl.h>

#include <atomic>
#include <cstdarg>
#include <cstdio>
#include <cstring>
#include <new>

#include "MicMixShared.h"

// ---------- Диагностический журнал: C:\ProgramData\MicMixApo\apo.log ----------
// Пишется только при загрузке и настройке, не в аудиопотоке (кроме одной строки
// при первом вызове APOProcess), так что на работу звука не влияет.
static void Log(const char* fmt, ...)
{
    char msg[512];
    SYSTEMTIME st; GetLocalTime(&st);
    int n = std::snprintf(msg, sizeof(msg), "%02u:%02u:%02u.%03u pid=%lu  ",
                          st.wHour, st.wMinute, st.wSecond, st.wMilliseconds, GetCurrentProcessId());
    if (n < 0) return;
    va_list ap; va_start(ap, fmt);
    int m = std::vsnprintf(msg + n, sizeof(msg) - n - 2, fmt, ap);
    va_end(ap);
    if (m < 0) return;
    size_t len = std::strlen(msg);
    msg[len++] = '\r'; msg[len++] = '\n';

    wchar_t path[MAX_PATH];
    if (!ExpandEnvironmentStringsW(L"%ProgramData%\\MicMixApo\\apo.log", path, MAX_PATH)) return;
    HANDLE h = CreateFileW(path, FILE_APPEND_DATA, FILE_SHARE_READ | FILE_SHARE_WRITE, nullptr,
                           OPEN_ALWAYS, FILE_ATTRIBUTE_NORMAL, nullptr);
    if (h == INVALID_HANDLE_VALUE) return;
    DWORD written = 0;
    WriteFile(h, msg, static_cast<DWORD>(len), &written, nullptr);
    CloseHandle(h);
}

static void LogFormat(const char* what, IAudioMediaType* type, HRESULT hr);

// GUID в виде {xxxxxxxx-...} для журнала.
static const char* GuidStr(REFGUID g, char (&buf)[40])
{
    std::snprintf(buf, sizeof(buf), "{%08lX-%04X-%04X-%02X%02X-%02X%02X%02X%02X%02X%02X}",
                  g.Data1, g.Data2, g.Data3, g.Data4[0], g.Data4[1], g.Data4[2],
                  g.Data4[3], g.Data4[4], g.Data4[5], g.Data4[6], g.Data4[7]);
    return buf;
}

// {04206D4E-6467-414B-B052-6AA2220324A5} — должен совпадать с $Clsid в install-apo.ps1
static const CLSID CLSID_MicMixApo =
    { 0x04206d4e, 0x6467, 0x414b, { 0xb0, 0x52, 0x6a, 0xa2, 0x22, 0x03, 0x24, 0xa5 } };

// KSDATAFORMAT_SUBTYPE_IEEE_FLOAT (объявлен здесь, чтобы не тянуть ksmedia/ksguid.lib)
static const GUID kSubtypeFloat =
    { 0x00000003, 0x0000, 0x0010, { 0x80, 0x00, 0x00, 0xaa, 0x00, 0x38, 0x9b, 0x71 } };

static std::atomic<long> g_objects{0};

// ---------- Общая память с движком ----------
// Создаёт (или открывает уже созданную) секцию с кольцевым буфером.
// Права: службы и администраторы — всё, вошедшие в систему пользователи — чтение и запись,
// чтобы движок, запущенный без прав администратора, мог туда писать. Плюс «песочные» процессы
// (ALL APPLICATION PACKAGES и ALL RESTRICTED APPLICATION PACKAGES): в новых Windows audiodg
// может работать в такой песочнице, и без этих записей секция была бы ему недоступна.
static micmix::RingHeader* OpenRing(HANDLE* outMapping)
{
    *outMapping = nullptr;
    SECURITY_ATTRIBUTES sa{ sizeof(sa), nullptr, FALSE };
    if (!ConvertStringSecurityDescriptorToSecurityDescriptorW(
            L"D:(A;;GA;;;SY)(A;;GA;;;LS)(A;;GA;;;BA)(A;;GRGW;;;IU)(A;;GA;;;AC)(A;;GA;;;S-1-15-2-2)",
            SDDL_REVISION_1, &sa.lpSecurityDescriptor, nullptr))
    {
        Log("Ring: bad security descriptor, err=%lu", GetLastError());
        return nullptr;
    }
    HANDLE h = CreateFileMappingW(INVALID_HANDLE_VALUE, &sa, PAGE_READWRITE, 0,
                                  static_cast<DWORD>(micmix::kRingBytes), micmix::kRingName);
    const DWORD err = GetLastError();
    LocalFree(sa.lpSecurityDescriptor);
    if (!h)
    {
        Log("Ring: CreateFileMapping failed, err=%lu", err);
        return nullptr;
    }
    auto* hdr = static_cast<micmix::RingHeader*>(
        MapViewOfFile(h, FILE_MAP_READ | FILE_MAP_WRITE, 0, 0, micmix::kRingBytes));
    if (!hdr)
    {
        Log("Ring: MapViewOfFile failed, err=%lu", GetLastError());
        CloseHandle(h);
        return nullptr;
    }
    if (err != ERROR_ALREADY_EXISTS)
    {
        // Новая секция заполнена нулями — пишем заголовок.
        hdr->version = micmix::kVersion;
        hdr->capacityFrames = micmix::kCapacityFrames;
        hdr->channels = micmix::kChannels;
        std::atomic_thread_fence(std::memory_order_release);
        hdr->magic = micmix::kMagic;
    }
    if (hdr->magic != micmix::kMagic || hdr->version != micmix::kVersion ||
        hdr->capacityFrames != micmix::kCapacityFrames || hdr->channels != micmix::kChannels)
    {
        Log("Ring: header mismatch (magic=0x%08X version=%u)", hdr->magic, hdr->version);
        UnmapViewOfFile(hdr);
        CloseHandle(h);
        return nullptr;
    }
    Log("Ring: %s %ls", err == ERROR_ALREADY_EXISTS ? "opened" : "created", micmix::kRingName);
    *outMapping = h;
    return hdr;
}
static std::atomic<long> g_serverLocks{0};

// Аудиодвижок Windows внутри APO работает с float32 — его и принимаем.
static bool IsFloat32(IAudioMediaType* type)
{
    if (!type) return false;
    UNCOMPRESSEDAUDIOFORMAT f{};
    if (FAILED(type->GetUncompressedAudioFormat(&f))) return false;
    return f.guidFormatType == kSubtypeFloat &&
           f.dwBytesPerSampleContainer == 4 &&
           f.dwValidBitsPerSample == 32 &&
           f.dwSamplesPerFrame >= 1;
}

static void LogFormat(const char* what, IAudioMediaType* type, HRESULT hr)
{
    UNCOMPRESSEDAUDIOFORMAT f{};
    if (type && SUCCEEDED(type->GetUncompressedAudioFormat(&f)))
        Log("%s: %s, %lu ch, %lu bit (container %lu B), %.0f Hz -> hr=0x%08lX",
            what, f.guidFormatType == kSubtypeFloat ? "float" : "PCM/other",
            f.dwSamplesPerFrame, f.dwValidBitsPerSample, f.dwBytesPerSampleContainer,
            f.fFramesPerSecond, static_cast<unsigned long>(hr));
    else
        Log("%s: format unreadable -> hr=0x%08lX", what, static_cast<unsigned long>(hr));
}

class MicMixApo final : public IAudioProcessingObject,
                        public IAudioProcessingObjectRT,
                        public IAudioProcessingObjectConfiguration,
                        public IAudioSystemEffects
{
public:
    // Аудиодвижок создаёт APO через COM-агрегацию: передаёт свой IUnknown (outer),
    // и все вызовы IUnknown на наших интерфейсах должны уходить к нему. Собственный
    // («неделегирующий») IUnknown объекта — m_inner, его движок получает из CreateInstance.
    explicit MicMixApo(IUnknown* outer)
    {
        m_inner.self = this;
        m_outer = outer ? outer : &m_inner;
        ++g_objects;
        Log("APO object created (%s)", outer ? "aggregated" : "standalone");
    }

    IUnknown* Inner() { return &m_inner; }

    // ---------- IUnknown (делегирует внешнему объекту) ----------
    STDMETHODIMP QueryInterface(REFIID riid, void** ppv) override { return m_outer->QueryInterface(riid, ppv); }
    STDMETHODIMP_(ULONG) AddRef() override { return m_outer->AddRef(); }
    STDMETHODIMP_(ULONG) Release() override { return m_outer->Release(); }

private:
    // Неделегирующий IUnknown: настоящие QueryInterface и счётчик ссылок.
    struct InnerUnknown final : IUnknown
    {
        MicMixApo* self = nullptr;
        STDMETHODIMP QueryInterface(REFIID riid, void** ppv) override { return self->InnerQueryInterface(riid, ppv); }
        STDMETHODIMP_(ULONG) AddRef() override { return static_cast<ULONG>(++self->m_refs); }
        STDMETHODIMP_(ULONG) Release() override
        {
            const long r = --self->m_refs;
            if (r == 0) delete self;
            return static_cast<ULONG>(r);
        }
    };

    HRESULT InnerQueryInterface(REFIID riid, void** ppv)
    {
        if (!ppv) return E_POINTER;
        *ppv = nullptr;
        if (riid == __uuidof(IUnknown))
        {
            *ppv = &m_inner;
            m_inner.AddRef();
            return S_OK;
        }
        if (riid == __uuidof(IAudioProcessingObject))
            *ppv = static_cast<IAudioProcessingObject*>(this);
        else if (riid == __uuidof(IAudioProcessingObjectRT))
            *ppv = static_cast<IAudioProcessingObjectRT*>(this);
        else if (riid == __uuidof(IAudioProcessingObjectConfiguration))
            *ppv = static_cast<IAudioProcessingObjectConfiguration*>(this);
        else if (riid == __uuidof(IAudioSystemEffects))
            *ppv = static_cast<IAudioSystemEffects*>(this);
        else
        {
            char g[40];
            Log("QueryInterface: no %s", GuidStr(riid, g));
            return E_NOINTERFACE;
        }
        static_cast<IUnknown*>(*ppv)->AddRef();   // через outer, как требует агрегация
        return S_OK;
    }

public:
    // ---------- IAudioProcessingObject ----------
    STDMETHODIMP Reset() override
    {
        m_primed = false;
        return S_OK;
    }

    STDMETHODIMP GetLatency(HNSTIME* pTime) override
    {
        if (!pTime) return E_POINTER;
        *pTime = 0;
        return S_OK;
    }

    STDMETHODIMP GetRegistrationProperties(APO_REG_PROPERTIES** ppRegProps) override
    {
        if (!ppRegProps) return E_POINTER;
        auto* p = static_cast<APO_REG_PROPERTIES*>(CoTaskMemAlloc(sizeof(APO_REG_PROPERTIES)));
        if (!p) return E_OUTOFMEMORY;
        ZeroMemory(p, sizeof(*p));
        p->clsid = CLSID_MicMixApo;
        p->Flags = static_cast<APO_FLAG>(APO_FLAG_INPLACE | APO_FLAG_DEFAULT);
        wcscpy_s(p->szFriendlyName, L"MicMix APO");
        wcscpy_s(p->szCopyrightInfo, L"MicMix (open source)");
        p->u32MajorVersion = 0;
        p->u32MinorVersion = 1;
        p->u32MinInputConnections = 1;
        p->u32MaxInputConnections = 1;
        p->u32MinOutputConnections = 1;
        p->u32MaxOutputConnections = 1;
        p->u32MaxInstances = 0xFFFFFFFF;
        p->u32NumAPOInterfaces = 1;
        p->iidAPOInterfaceList[0] = __uuidof(IAudioProcessingObject);
        *ppRegProps = p;
        return S_OK;
    }

    STDMETHODIMP Initialize(UINT32 cbDataSize, BYTE* /*pbyData*/) override
    {
        // pbyData — APOInitSystemEffects(2): данные об эндпоинте. Пока не нужны.
        Log("Initialize (cbDataSize=%u)", cbDataSize);
        return S_OK;
    }

    STDMETHODIMP IsInputFormatSupported(IAudioMediaType* /*pOpposite*/,
                                        IAudioMediaType* pRequested,
                                        IAudioMediaType** ppSupported) override
    {
        HRESULT hr = CheckFormat(pRequested, ppSupported);
        LogFormat("IsInputFormatSupported", pRequested, hr);
        return hr;
    }

    STDMETHODIMP IsOutputFormatSupported(IAudioMediaType* /*pOpposite*/,
                                         IAudioMediaType* pRequested,
                                         IAudioMediaType** ppSupported) override
    {
        HRESULT hr = CheckFormat(pRequested, ppSupported);
        LogFormat("IsOutputFormatSupported", pRequested, hr);
        return hr;
    }

    STDMETHODIMP GetInputChannelCount(UINT32* pu32ChannelCount) override
    {
        if (!pu32ChannelCount) return E_POINTER;
        *pu32ChannelCount = m_channels;
        return S_OK;
    }

    // ---------- IAudioProcessingObjectConfiguration ----------
    STDMETHODIMP LockForProcess(UINT32 numIn, APO_CONNECTION_DESCRIPTOR** ppIn,
                                UINT32 numOut, APO_CONNECTION_DESCRIPTOR** ppOut) override
    {
        if (m_locked) return APOERR_APO_LOCKED;
        if (numIn != 1 || numOut != 1 || !ppIn || !ppOut || !ppIn[0] || !ppOut[0])
        {
            Log("LockForProcess: bad connections in=%u out=%u", numIn, numOut);
            return APOERR_NUM_CONNECTIONS_INVALID;
        }
        if (!IsFloat32(ppIn[0]->pFormat) || !IsFloat32(ppOut[0]->pFormat))
        {
            LogFormat("LockForProcess REJECTED input", ppIn[0]->pFormat, APOERR_FORMAT_NOT_SUPPORTED);
            return APOERR_FORMAT_NOT_SUPPORTED;
        }

        UNCOMPRESSEDAUDIOFORMAT f{};
        HRESULT hr = ppIn[0]->pFormat->GetUncompressedAudioFormat(&f);
        if (FAILED(hr)) return hr;

        m_channels = f.dwSamplesPerFrame;
        m_sampleRate = f.fFramesPerSecond > 0 ? static_cast<UINT32>(f.fFramesPerSecond) : 48000;
        if (!m_ring) m_ring = OpenRing(&m_mapping);
        if (m_ring) m_ring->micSampleRate.store(m_sampleRate, std::memory_order_release);
        Reset();
        m_locked = true;
        m_loggedProcess = false;
        m_rateMismatch = false;
        m_mixedFrames = 0;
        m_underruns = 0;
        Log("LockForProcess OK: %u ch, %u Hz, max %u frames", m_channels, m_sampleRate,
            ppIn[0]->u32MaxFrameCount);
        return S_OK;
    }

    STDMETHODIMP UnlockForProcess() override
    {
        if (!m_locked) return APOERR_ALREADY_UNLOCKED;
        m_locked = false;
        Log("UnlockForProcess: mixed %llu frames, underruns %lu%s", m_mixedFrames, m_underruns,
            m_rateMismatch ? ", engine sample rate differs from mic (not mixed)" : "");
        return S_OK;
    }

    // ---------- IAudioProcessingObjectRT ----------
    // Вызывается в реальном времени: никаких выделений памяти, блокировок и логов.
    STDMETHODIMP_(void) APOProcess(UINT32 numIn, APO_CONNECTION_PROPERTY** ppIn,
                                   UINT32 numOut, APO_CONNECTION_PROPERTY** ppOut) override
    {
        if (numIn < 1 || numOut < 1) return;
        APO_CONNECTION_PROPERTY* in = ppIn[0];
        APO_CONNECTION_PROPERTY* out = ppOut[0];

        if (!m_loggedProcess)
        {
            m_loggedProcess = true;   // одна строка на запуск потока — допустимо
            Log("APOProcess running: %u frames, flags=%d, in-place=%d",
                in->u32ValidFrameCount, static_cast<int>(in->u32BufferFlags),
                in->pBuffer == out->pBuffer ? 1 : 0);
        }
        const UINT32 frames = in->u32ValidFrameCount;
        const UINT32 ch = m_channels;
        auto* src = reinterpret_cast<const float*>(in->pBuffer);
        auto* dst = reinterpret_cast<float*>(out->pBuffer);

        if (in->u32BufferFlags != BUFFER_VALID)
            std::memset(dst, 0, sizeof(float) * frames * ch);   // тишина / нет данных
        else if (dst != src)
            std::memcpy(dst, src, sizeof(float) * frames * ch); // обычно буфер общий (in-place)

        MixFromRing(dst, frames, ch);

        out->u32ValidFrameCount = frames;
        out->u32BufferFlags = BUFFER_VALID;
    }

    // Подмешивает в dst звук из общей памяти. Без блокировок и выделений памяти.
    void MixFromRing(float* dst, UINT32 frames, UINT32 ch)
    {
        micmix::RingHeader* r = m_ring;
        if (!r || ch == 0) return;
        const uint32_t rate = r->sampleRate.load(std::memory_order_acquire);
        if (rate == 0) { m_primed = false; return; }              // движок не подключён
        if (rate != m_sampleRate) { m_rateMismatch = true; m_primed = false; return; }

        const int64_t written = r->writeFrame.load(std::memory_order_acquire);
        int64_t avail = written - m_readFrame;
        if (avail < 0 || avail > static_cast<int64_t>(micmix::kCapacityFrames / 2))
        {
            // Потеряли синхронизацию (движок перезапустился или мы отстали) — догоняем.
            m_readFrame = written;
            avail = 0;
            m_primed = false;
        }
        if (!m_primed)
        {
            // Копим небольшой запас, чтобы неровная запись движка не давала щелчков.
            if (avail < kPrimeFrames) return;
            m_primed = true;
        }
        if (avail > kMaxLagFrames)
        {
            // Задержка выросла (часы движка и микрофона немного расходятся) — срезаем лишнее.
            m_readFrame = written - kPrimeFrames;
            avail = kPrimeFrames;
        }

        const UINT32 n = avail < frames ? static_cast<UINT32>(avail) : frames;
        const float* data = reinterpret_cast<const float*>(
            reinterpret_cast<const BYTE*>(r) + micmix::kDataOffset);
        constexpr uint32_t mask = micmix::kCapacityFrames - 1;
        for (UINT32 i = 0; i < n; ++i)
        {
            const float* in = data + static_cast<size_t>((m_readFrame + i) & mask) * micmix::kChannels;
            float* frame = dst + static_cast<size_t>(i) * ch;
            if (ch == 1)
                frame[0] = Clamp(frame[0] + 0.5f * (in[0] + in[1]));
            else
                for (UINT32 c = 0; c < ch; ++c)
                    frame[c] = Clamp(frame[c] + in[c & 1]);
        }
        m_readFrame += n;
        m_mixedFrames += n;
        if (n < frames) { m_primed = false; ++m_underruns; }   // движок не успел — ждём новый запас
    }

    static float Clamp(float v) { return v > 1.0f ? 1.0f : (v < -1.0f ? -1.0f : v); }

    STDMETHODIMP_(UINT32) CalcInputFrames(UINT32 outFrames) override { return outFrames; }
    STDMETHODIMP_(UINT32) CalcOutputFrames(UINT32 inFrames) override { return inFrames; }

private:
    ~MicMixApo()
    {
        if (m_ring) UnmapViewOfFile(m_ring);
        if (m_mapping) CloseHandle(m_mapping);
        --g_objects;
    }

    HRESULT CheckFormat(IAudioMediaType* pRequested, IAudioMediaType** ppSupported)
    {
        if (!ppSupported) return E_POINTER;
        *ppSupported = nullptr;
        if (!pRequested) return E_POINTER;
        if (!IsFloat32(pRequested)) return APOERR_FORMAT_NOT_SUPPORTED;
        pRequested->AddRef();
        *ppSupported = pRequested;
        return S_OK;
    }

    static constexpr int64_t kPrimeFrames = 960;      // 20 мс при 48 кГц: запас перед стартом
    static constexpr int64_t kMaxLagFrames = 4 * 960; // 80 мс: больше — срезаем задержку

    std::atomic<long> m_refs{1};
    InnerUnknown m_inner;
    IUnknown* m_outer = nullptr;
    bool m_locked = false;
    UINT32 m_channels = 1;
    UINT32 m_sampleRate = 48000;
    bool m_loggedProcess = false;
    HANDLE m_mapping = nullptr;
    micmix::RingHeader* m_ring = nullptr;
    int64_t m_readFrame = 0;
    bool m_primed = false;
    bool m_rateMismatch = false;
    unsigned long long m_mixedFrames = 0;
    unsigned long m_underruns = 0;
};

// ---------- Фабрика классов и экспорт COM ----------
class ClassFactory final : public IClassFactory
{
public:
    STDMETHODIMP QueryInterface(REFIID riid, void** ppv) override
    {
        if (!ppv) return E_POINTER;
        if (riid == __uuidof(IUnknown) || riid == __uuidof(IClassFactory))
        {
            *ppv = static_cast<IClassFactory*>(this);
            AddRef();
            return S_OK;
        }
        *ppv = nullptr;
        char g[40];
        Log("ClassFactory::QueryInterface: no %s", GuidStr(riid, g));
        return E_NOINTERFACE;
    }
    STDMETHODIMP_(ULONG) AddRef() override { return 2; }   // статический объект
    STDMETHODIMP_(ULONG) Release() override { return 1; }

    STDMETHODIMP CreateInstance(IUnknown* outer, REFIID riid, void** ppv) override
    {
        if (!ppv) return E_POINTER;
        *ppv = nullptr;
        char g[40];
        Log("CreateInstance(%s, outer=%s)", GuidStr(riid, g), outer ? "yes" : "no");
        // При агрегации можно запросить только IUnknown (правило COM).
        if (outer && riid != __uuidof(IUnknown)) return CLASS_E_NOAGGREGATION;
        auto* apo = new (std::nothrow) MicMixApo(outer);
        if (!apo) return E_OUTOFMEMORY;
        HRESULT hr = apo->Inner()->QueryInterface(riid, ppv);
        apo->Inner()->Release();
        Log("CreateInstance -> hr=0x%08lX", static_cast<unsigned long>(hr));
        return hr;
    }
    STDMETHODIMP LockServer(BOOL lock) override
    {
        if (lock) ++g_serverLocks; else --g_serverLocks;
        return S_OK;
    }
};

static ClassFactory g_factory;

STDAPI DllGetClassObject(REFCLSID rclsid, REFIID riid, LPVOID* ppv)
{
    if (!ppv) return E_POINTER;
    *ppv = nullptr;
    char c[40], i[40];
    Log("DllGetClassObject(%s, %s)", GuidStr(rclsid, c), GuidStr(riid, i));
    if (rclsid != CLSID_MicMixApo)
    {
        Log("DllGetClassObject: unknown CLSID");
        return CLASS_E_CLASSNOTAVAILABLE;
    }
    return g_factory.QueryInterface(riid, ppv);
}

STDAPI DllCanUnloadNow()
{
    const bool can = g_objects == 0 && g_serverLocks == 0;
    if (can) Log("DllCanUnloadNow -> S_OK");
    return can ? S_OK : S_FALSE;
}

BOOL WINAPI DllMain(HINSTANCE hinst, DWORD reason, LPVOID)
{
    if (reason == DLL_PROCESS_ATTACH)
    {
        DisableThreadLibraryCalls(hinst);
        wchar_t exe[MAX_PATH] = L"?";
        GetModuleFileNameW(nullptr, exe, MAX_PATH);
        Log("DLL loaded into %ls", exe);
    }
    else if (reason == DLL_PROCESS_DETACH)
    {
        Log("DLL unloaded");
    }
    return TRUE;
}
