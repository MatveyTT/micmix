// captest — проверка бипа без «Звукозаписи».
//
// Пишет 3 секунды с микрофона через WASAPI (общий режим) и ищет в сигнале
// тон 880 Гц, который подмешивает MicMix APO. Звук никуда не сохраняется.
//
//   captest.exe "<часть имени микрофона>" [default|raw|comms]
//
// default — обычный режим (эффекты включены), raw — RAW (Windows обходит эффекты),
// comms — категория «Связь», как у Discord и игр.

#define WIN32_LEAN_AND_MEAN
#include <windows.h>
#include <initguid.h>
#include <mmdeviceapi.h>
#include <audioclient.h>
#include <functiondiscoverykeys_devpkey.h>
#include <mmreg.h>
#include <ksmedia.h>

#include <fcntl.h>
#include <io.h>

#include <cmath>
#include <cstdio>
#include <cwchar>
#include <vector>

#define CHECK(expr) do { HRESULT hr_ = (expr); if (FAILED(hr_)) { \
    std::wprintf(L"[!] %hs -> 0x%08lX\n", #expr, static_cast<unsigned long>(hr_)); return 1; } } while (0)

// Мощность частоты freq в сигнале (алгоритм Гёрцеля), нормированная на длину.
static double Goertzel(const std::vector<float>& x, double freq, double rate)
{
    const double w = 2.0 * 3.14159265358979323846 * freq / rate;
    const double k = 2.0 * std::cos(w);
    double s1 = 0, s2 = 0;
    for (float v : x) { double s0 = v + k * s1 - s2; s2 = s1; s1 = s0; }
    const double p = s1 * s1 + s2 * s2 - k * s1 * s2;
    return p / (static_cast<double>(x.size()) * x.size());
}

int wmain(int argc, wchar_t** argv)
{
    _setmode(_fileno(stdout), _O_U8TEXT);   // кириллица в консоли и при перенаправлении
    const wchar_t* filter = argc > 1 ? argv[1] : L"";
    const wchar_t* mode = argc > 2 ? argv[2] : L"default";

    CHECK(CoInitializeEx(nullptr, COINIT_MULTITHREADED));
    IMMDeviceEnumerator* en = nullptr;
    CHECK(CoCreateInstance(__uuidof(MMDeviceEnumerator), nullptr, CLSCTX_ALL,
                           __uuidof(IMMDeviceEnumerator), reinterpret_cast<void**>(&en)));
    IMMDeviceCollection* list = nullptr;
    CHECK(en->EnumAudioEndpoints(eCapture, DEVICE_STATE_ACTIVE, &list));
    UINT count = 0;
    CHECK(list->GetCount(&count));

    IMMDevice* dev = nullptr;
    for (UINT i = 0; i < count && !dev; ++i)
    {
        IMMDevice* d = nullptr;
        if (FAILED(list->Item(i, &d))) continue;
        IPropertyStore* ps = nullptr;
        PROPVARIANT name; PropVariantInit(&name);
        if (SUCCEEDED(d->OpenPropertyStore(STGM_READ, &ps)) &&
            SUCCEEDED(ps->GetValue(PKEY_Device_FriendlyName, &name)) && name.vt == VT_LPWSTR &&
            std::wcsstr(name.pwszVal, filter))
        {
            std::wprintf(L"Микрофон: %ls\n", name.pwszVal);
            dev = d; d = nullptr;
        }
        PropVariantClear(&name);
        if (ps) ps->Release();
        if (d) d->Release();
    }
    if (!dev) { std::wprintf(L"[!] Не нашёл микрофон «%ls»\n", filter); return 1; }

    IAudioClient2* client = nullptr;
    CHECK(dev->Activate(__uuidof(IAudioClient2), CLSCTX_ALL, nullptr, reinterpret_cast<void**>(&client)));

    AudioClientProperties props{};
    props.cbSize = sizeof(props);
    props.eCategory = AudioCategory_Other;
    if (!std::wcscmp(mode, L"raw"))   props.Options = AUDCLNT_STREAMOPTIONS_RAW;
    if (!std::wcscmp(mode, L"comms")) props.eCategory = AudioCategory_Communications;
    CHECK(client->SetClientProperties(&props));
    std::wprintf(L"Режим: %ls\n", mode);

    WAVEFORMATEX* wf = nullptr;
    CHECK(client->GetMixFormat(&wf));
    bool isFloat = wf->wFormatTag == WAVE_FORMAT_IEEE_FLOAT;
    if (wf->wFormatTag == WAVE_FORMAT_EXTENSIBLE)
        isFloat = reinterpret_cast<WAVEFORMATEXTENSIBLE*>(wf)->SubFormat == KSDATAFORMAT_SUBTYPE_IEEE_FLOAT;
    std::wprintf(L"Формат: %u ch, %lu Hz, %u bit, %ls\n", wf->nChannels, wf->nSamplesPerSec,
                 wf->wBitsPerSample, isFloat ? L"float" : L"PCM");
    if (!isFloat || wf->wBitsPerSample != 32) { std::wprintf(L"[!] ожидался float32\n"); return 1; }

    CHECK(client->Initialize(AUDCLNT_SHAREMODE_SHARED, 0, 10000000, 0, wf, nullptr));
    IAudioCaptureClient* cap = nullptr;
    CHECK(client->GetService(__uuidof(IAudioCaptureClient), reinterpret_cast<void**>(&cap)));

    const UINT32 rate = wf->nSamplesPerSec, ch = wf->nChannels;
    std::vector<float> mono;
    mono.reserve(rate * 4);
    UINT32 silentPackets = 0, packets = 0;

    CHECK(client->Start());
    while (mono.size() < rate * 3)
    {
        Sleep(10);
        UINT32 n = 0;
        while (SUCCEEDED(cap->GetNextPacketSize(&n)) && n)
        {
            BYTE* data = nullptr; UINT32 frames = 0; DWORD flags = 0;
            CHECK(cap->GetBuffer(&data, &frames, &flags, nullptr, nullptr));
            ++packets;
            const float* f = reinterpret_cast<const float*>(data);
            const bool silent = (flags & AUDCLNT_BUFFERFLAGS_SILENT) != 0;
            if (silent) ++silentPackets;
            for (UINT32 i = 0; i < frames; ++i)
                mono.push_back(silent ? 0.0f : f[static_cast<size_t>(i) * ch]);
            cap->ReleaseBuffer(frames);
        }
    }
    client->Stop();

    double sum = 0, peak = 0;
    for (float v : mono) { sum += double(v) * v; peak = (std::max)(peak, double(std::fabs(v))); }
    const double rms = std::sqrt(sum / mono.size());
    const double tone = Goertzel(mono, 880.0, rate);
    const double ref = (Goertzel(mono, 700.0, rate) + Goertzel(mono, 1100.0, rate)) / 2 + 1e-20;
    // Бип 0.08 * sin, 30% времени → амплитуда на 880 Гц около 0.012.
    const double amp880 = 2.0 * std::sqrt(tone);

    std::wprintf(L"Пакетов: %u (тишина: %u), сэмплов: %zu\n", packets, silentPackets, mono.size());
    std::wprintf(L"RMS %.5f, пик %.4f\n", rms, peak);
    std::wprintf(L"880 Гц: амплитуда %.5f, в %.0f раз выше соседних частот\n", amp880, tone / ref);
    const bool beep = amp880 > 0.003 && tone / ref > 20;
    std::wprintf(beep ? L"БИП ЕСТЬ\n" : L"бипа нет\n");
    return beep ? 0 : 2;
}
