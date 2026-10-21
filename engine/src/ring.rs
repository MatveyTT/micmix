//! Запись звука в общую память, откуда его читает APO.
//!
//! Раскладка должна совпадать с `src/MicMixShared.h`. Секцию создаёт APO
//! (только у служб есть право на объекты `Global\`), движок её открывает.
//! Секция появляется, когда какая-нибудь программа начинает запись с микрофона,
//! поэтому `Ring::open` может вернуть `None` — тогда нужно попробовать позже.

use std::sync::atomic::{AtomicI64, AtomicU32, Ordering};

use windows::core::w;
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::System::Memory::{
    MapViewOfFile, OpenFileMappingW, UnmapViewOfFile, FILE_MAP_READ, FILE_MAP_WRITE,
    MEMORY_MAPPED_VIEW_ADDRESS,
};

const MAGIC: u32 = 0x5849_4D4D; // "MMIX"
const VERSION: u32 = 1;
pub const CHANNELS: usize = 2;
const CAPACITY_FRAMES: usize = 1 << 14;
const DATA_OFFSET: usize = 64;
const RING_BYTES: usize = DATA_OFFSET + CAPACITY_FRAMES * CHANNELS * std::mem::size_of::<f32>();

#[repr(C)]
struct Header {
    magic: u32,
    version: u32,
    capacity_frames: u32,
    channels: u32,
    sample_rate: AtomicU32, // 0 — движок не подключён
    reserved: u32,
    write_frame: AtomicI64, // сколько кадров записано за всё время
}

const _: () = assert!(std::mem::offset_of!(Header, write_frame) == 24);
const _: () = assert!(std::mem::size_of::<Header>() <= DATA_OFFSET);

pub struct Ring {
    mapping: HANDLE,
    view: MEMORY_MAPPED_VIEW_ADDRESS,
    write_frame: i64,
}

// Указатель на общую память используется только из одного потока движка.
unsafe impl Send for Ring {}

impl Ring {
    /// Открывает секцию APO. `None` — APO сейчас не запущен (никто не пишет с микрофона).
    pub fn open(sample_rate: u32) -> Option<Ring> {
        unsafe {
            let mapping = OpenFileMappingW(
                (FILE_MAP_READ | FILE_MAP_WRITE).0,
                false,
                w!("Global\\MicMixApo.Ring.v1"),
            )
            .ok()?;
            let view = MapViewOfFile(mapping, FILE_MAP_READ | FILE_MAP_WRITE, 0, 0, RING_BYTES);
            if view.Value.is_null() {
                let _ = CloseHandle(mapping);
                return None;
            }
            let mut ring = Ring { mapping, view, write_frame: 0 };
            let h = ring.header();
            if h.magic != MAGIC
                || h.version != VERSION
                || h.capacity_frames as usize != CAPACITY_FRAMES
                || h.channels as usize != CHANNELS
            {
                eprintln!("[!] Общая память другой версии — пересобери и переустанови APO.");
                return None; // Drop закроет секцию
            }
            let written = h.write_frame.load(Ordering::Acquire);
            h.sample_rate.store(sample_rate, Ordering::Release);
            ring.write_frame = written;
            Some(ring)
        }
    }

    fn header(&self) -> &Header {
        unsafe { &*(self.view.Value as *const Header) }
    }

    /// Дописывает стерео-кадры (L, R, L, R, ...) и сообщает APO, что они готовы.
    pub fn write(&mut self, interleaved: &[f32]) {
        let frames = interleaved.len() / CHANNELS;
        let data = unsafe { (self.view.Value as *mut u8).add(DATA_OFFSET) as *mut f32 };
        for i in 0..frames {
            let slot = ((self.write_frame + i as i64) as usize) & (CAPACITY_FRAMES - 1);
            unsafe {
                *data.add(slot * CHANNELS) = interleaved[i * CHANNELS];
                *data.add(slot * CHANNELS + 1) = interleaved[i * CHANNELS + 1];
            }
        }
        self.write_frame += frames as i64;
        self.header().write_frame.store(self.write_frame, Ordering::Release);
    }
}

impl Drop for Ring {
    fn drop(&mut self) {
        unsafe {
            if self.header().magic == MAGIC {
                self.header().sample_rate.store(0, Ordering::Release);
            }
            let _ = UnmapViewOfFile(self.view);
            let _ = CloseHandle(self.mapping);
        }
    }
}
