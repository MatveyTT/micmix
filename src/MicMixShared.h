// Общая память между движком MicMix (пишет звук) и APO (подмешивает его в микрофон).
//
// Раскладка должна совпадать с engine/src/ring.rs.
//
// Секцию создаёт APO: он работает в audiodg.exe от LOCAL SERVICE, и только у служб
// есть право создавать объекты в пространстве имён Global\. Движок её открывает.
// Звук хранится кольцом: кадр N лежит в ячейке N % kCapacityFrames.
// Движок дописывает кадры и потом увеличивает writeFrame; каждый поток APO
// читает со своей позиции, поэтому читателей может быть сколько угодно.
#pragma once

#include <atomic>
#include <cstddef>
#include <cstdint>

namespace micmix
{
constexpr wchar_t  kRingName[]     = L"Global\\MicMixApo.Ring.v1";
constexpr uint32_t kMagic          = 0x58494D4D;    // "MMIX"
constexpr uint32_t kVersion        = 1;
constexpr uint32_t kChannels       = 2;             // всегда стерео float32
constexpr uint32_t kCapacityFrames = 1u << 14;      // 16384 кадров ≈ 341 мс при 48 кГц
constexpr size_t   kDataOffset     = 64;

struct RingHeader
{
    uint32_t magic;
    uint32_t version;
    uint32_t capacityFrames;
    uint32_t channels;
    std::atomic<uint32_t> sampleRate;   // ставит движок; 0 — движок не подключён
    std::atomic<uint32_t> micSampleRate; // ставит APO: частота микрофона, в ней движок и пишет
    std::atomic<int64_t> writeFrame;    // сколько кадров записано за всё время
};
static_assert(sizeof(RingHeader) <= kDataOffset, "header too big");
static_assert(offsetof(RingHeader, writeFrame) == 24, "layout must match ring.rs");

constexpr size_t kRingBytes = kDataOffset + size_t(kCapacityFrames) * kChannels * sizeof(float);
}
