<script>
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { getCurrentWebview } from "@tauri-apps/api/webview";
  import { onMount } from "svelte";
  import AddSourceDialog from "./lib/AddSourceDialog.svelte";
  import HotkeyInput from "./lib/HotkeyInput.svelte";
  import Icon from "./lib/Icon.svelte";
  import Meter from "./lib/Meter.svelte";
  import MicPanel from "./lib/MicPanel.svelte";
  import SoundsPanel from "./lib/SoundsPanel.svelte";
  import SourceCard from "./lib/SourceCard.svelte";
  import Volume from "./lib/Volume.svelte";

  const TONE = "micmix:tone";
  const SOUND_EXT = ["mp3", "wav", "ogg", "flac", "m4a", "aac", "mp4", "mka", "mkv", "webm"];

  let status = $state(null);
  let mics = $state(null); // ответ list_mics: { mics, system, tab_error }
  let hotkeys = $state(null); // ответ get_hotkeys: { master, apps, stop, errors }
  let tab = $state("sources");
  let adding = $state(false);
  let masterGain = $state(0.7);
  let draggingMaster = $state(false);
  let dropping = $state(false); // над окном тащат файлы
  let toasts = $state([]); // сообщения об ошибках

  const installed = $derived(mics?.mics.filter((m) => m.apo_installed) ?? []);
  const sources = $derived(status?.sources.filter((s) => s.exe !== TONE) ?? []);
  const toneOn = $derived(status?.sources.some((s) => s.exe === TONE) ?? false);
  const added = $derived(new Set(status?.sources.map((s) => s.exe) ?? []));
  const playingSounds = $derived(status?.sounds.filter((s) => s.progress !== null).length ?? 0);

  const banner = $derived.by(() => {
    if (!status || !mics) return { kind: "load", text: "Загрузка…" };
    if (installed.length === 0) return { kind: "warn", text: "MicMix не установлен на микрофон" };
    if (status.master_muted) return { kind: "off", text: "Звук в микрофон выключен" };
    if (!status.apo_connected) return { kind: "idle", text: "Ждёт: микрофон сейчас никто не записывает" };
    return { kind: "live", text: `В эфире · ${installed.map((m) => m.name).join(", ")} · ${status.sample_rate / 1000} кГц` };
  });

  function toast(text) {
    const id = Math.random();
    toasts = [...toasts, { id, text }];
    setTimeout(() => (toasts = toasts.filter((t) => t.id !== id)), 6000);
  }

  async function poll() {
    try {
      status = await invoke("get_status");
      if (!draggingMaster) masterGain = status.master_gain;
    } catch (e) {
      console.error(e);
    }
  }

  const loadHotkeys = () => invoke("get_hotkeys").then((h) => (hotkeys = h));

  // Назначить клавишу; при ошибке (занята, повтор) — исключение для HotkeyInput.
  async function setHotkey(target, hotkey) {
    try {
      await invoke("set_hotkey", { target, hotkey });
    } finally {
      await Promise.all([loadHotkeys(), poll()]);
    }
  }

  async function addSounds(paths) {
    const errors = await invoke("add_sounds", { paths });
    errors.forEach(toast);
    tab = "sounds";
    poll();
  }

  onMount(() => {
    poll();
    loadHotkeys();
    invoke("list_mics").then((m) => {
      mics = m;
      if (m.mics.every((x) => !x.apo_installed)) tab = "mic";
    });
    const t = setInterval(poll, 80);
    const unlisten = [
      listen("state-changed", () => {
        poll();
        loadHotkeys();
      }),
      listen("sound-error", (e) => toast(String(e.payload))),
      // Файлы, перетащенные в окно, становятся звуками-кнопками.
      getCurrentWebview().onDragDropEvent((e) => {
        const p = e.payload;
        if (p.type === "enter" || p.type === "over") dropping = true;
        else if (p.type === "leave") dropping = false;
        else if (p.type === "drop") {
          dropping = false;
          const files = p.paths.filter((f) => SOUND_EXT.includes(f.split(".").pop().toLowerCase()));
          if (files.length) addSounds(files);
          else toast("Это не звуковые файлы: подходят mp3, wav, ogg, flac, m4a");
        }
      }),
    ];
    return () => {
      clearInterval(t);
      unlisten.forEach((u) => u.then((f) => f()));
    };
  });

  const call = (cmd, args) => invoke(cmd, args).then(poll);
</script>

<div class="flex h-full flex-col">
  <!-- Шапка: логотип, статус, общая громкость -->
  <header class="flex items-center gap-5 border-b border-line bg-panel/60 px-6 py-4">
    <div class="flex min-w-0 items-center gap-3">
      <div class="grid h-10 w-10 shrink-0 place-items-center rounded-xl bg-gradient-to-br from-accent to-accent-2 shadow-lg shadow-accent/20">
        <Icon name="mic" size={20} class="text-white" />
      </div>
      <div class="min-w-0">
        <div class="text-lg font-semibold leading-tight">MicMix</div>
        <div class="flex items-center gap-1.5 text-xs">
          <span
            class="h-2 w-2 shrink-0 rounded-full
              {banner.kind === 'live' ? 'animate-pulse bg-emerald-400' : banner.kind === 'warn' ? 'bg-amber-400' : banner.kind === 'off' ? 'bg-rose-400' : 'bg-muted'}"
          ></span>
          <span class="max-w-72 truncate text-muted" title={banner.text}>{banner.text}</span>
        </div>
      </div>
    </div>

    <div class="ml-auto flex items-center gap-3">
      <button
        class="flex shrink-0 items-center gap-2 whitespace-nowrap rounded-xl border px-3 py-2 text-sm transition
          {toneOn ? 'border-accent bg-accent/15 text-white' : 'border-line text-muted hover:text-white'}"
        title="Подать в микрофон тестовый бип 880 Гц"
        onclick={() => call("set_test_tone", { on: !toneOn })}
      >
        <Icon name="wave" size={16} /> Тестовый бип
      </button>

      <div class="flex w-80 items-center gap-3 rounded-2xl border border-line bg-panel-2 px-3 py-2">
        <button
          class="grid h-9 w-9 shrink-0 place-items-center rounded-xl transition
            {status?.master_muted ? 'bg-rose-500/20 text-rose-400' : 'bg-gradient-to-br from-accent to-accent-2 text-white'}"
          title={`${status?.master_muted ? "Включить" : "Выключить"} весь звук в микрофоне${hotkeys?.master ? ` (${hotkeys.master})` : ""}`}
          onclick={() => call("set_master_muted", { muted: !status?.master_muted })}
        >
          <Icon name={status?.master_muted ? "mic-off" : "mic"} size={17} />
        </button>
        <div class="flex flex-1 flex-col gap-1.5">
          <Volume
            value={masterGain}
            disabled={status?.master_muted}
            oninput={(v) => {
              draggingMaster = true;
              masterGain = v;
              invoke("set_master_gain", { gain: v, save: false });
            }}
            onchange={(v) => {
              draggingMaster = false;
              call("set_master_gain", { gain: v, save: true });
            }}
          />
          <div class="flex items-center gap-2 pr-14">
            <Meter level={status?.master_level ?? 0} muted={status?.master_muted} />
          </div>
        </div>
      </div>
    </div>
  </header>

  <!-- Вкладки -->
  <nav class="flex items-center gap-1 px-6 pt-3">
    {#each [["sources", "Приложения", "apps"], ["sounds", "Звуки", "music"], ["mic", "Микрофон", "mic"]] as [id, label, icon]}
      <button
        class="flex items-center gap-2 rounded-xl px-4 py-2 text-sm transition
          {tab === id ? 'bg-panel-2 text-white' : 'text-muted hover:text-white'}"
        onclick={() => (tab = id)}
      >
        <Icon name={icon} size={16} />
        {label}
        {#if id === "mic" && mics && installed.length === 0}
          <span class="h-2 w-2 rounded-full bg-amber-400"></span>
        {/if}
        {#if id === "sounds" && playingSounds > 0}
          <span class="h-2 w-2 animate-pulse rounded-full bg-accent"></span>
        {/if}
      </button>
    {/each}
    {#if status && status.limiter > 0.05}
      <div class="ml-auto flex items-center gap-1.5 text-xs text-amber-300" title="Слишком громко: лимитер приглушает звук, чтобы не было хрипа">
        <Icon name="shield" size={14} /> лимитер −{Math.round(-20 * Math.log10(1 - status.limiter))} дБ
      </div>
    {/if}
  </nav>

  <main class="flex-1 overflow-y-auto px-6 py-5">
    {#if tab === "sources"}
      {#if installed.length === 0 && mics}
        <button
          class="mb-4 flex w-full items-center gap-3 rounded-2xl border border-amber-500/30 bg-amber-500/10 p-4 text-left text-sm hover:bg-amber-500/15"
          onclick={() => (tab = "mic")}
        >
          <Icon name="alert" size={18} class="shrink-0 text-amber-400" />
          Сначала установи MicMix на микрофон — вкладка «Микрофон».
        </button>
      {/if}

      {#if status}
        <!-- Все приложения разом -->
        <div class="mb-4 flex items-center gap-3 rounded-2xl border border-line bg-panel px-4 py-2.5">
          <button
            class="flex items-center gap-2 rounded-xl px-3 py-1.5 text-sm transition
              {status.apps_muted ? 'bg-rose-500/15 text-rose-300 hover:bg-rose-500/25' : 'bg-panel-2 text-white hover:bg-accent/20'}"
            onclick={() => call("set_apps_muted", { muted: !status.apps_muted })}
          >
            <Icon name={status.apps_muted ? "volume-x" : "volume"} size={16} />
            {status.apps_muted ? "Все приложения выключены" : "Все приложения включены"}
          </button>
          <span class="text-xs text-muted">вкл/выкл все:</span>
          <HotkeyInput value={hotkeys?.apps} error={hotkeys?.errors["apps"]} onset={(v) => setHotkey("apps", v)} />
        </div>
      {/if}

      <div class="grid grid-cols-[repeat(auto-fill,minmax(280px,1fr))] gap-4">
        {#each sources as source (source.exe)}
          <SourceCard
            {source}
            hotkeyError={hotkeys?.errors[`source:${source.exe}`]}
            ongain={(exe, gain, save) => invoke("set_gain", { exe, gain, save })}
            onmute={(exe, muted) => call("set_muted", { exe, muted })}
            onremove={(exe) => call("remove_source", { exe }).then(loadHotkeys)}
            onhotkey={(exe, v) => setHotkey(`source:${exe}`, v)}
          />
        {/each}

        <button
          class="flex min-h-40 flex-col items-center justify-center gap-2 rounded-2xl border border-dashed border-line text-muted transition hover:border-accent/60 hover:bg-panel hover:text-white"
          onclick={() => (adding = true)}
        >
          <div class="grid h-11 w-11 place-items-center rounded-xl bg-panel-2"><Icon name="plus" size={20} /></div>
          <span class="text-sm">Добавить приложение</span>
        </button>
      </div>

      {#if sources.length === 0}
        <p class="mt-6 text-center text-sm text-muted">
          Добавь браузер, плеер или игру — их звук пойдёт в микрофон вместе с твоим голосом.
        </p>
      {/if}
    {:else if tab === "sounds"}
      {#if status}
        <SoundsPanel {status} {hotkeys} onhotkey={setHotkey} onadded={addSounds} refresh={poll} />
      {/if}
    {:else}
      <MicPanel onchanged={(m) => (mics = m)} />
    {/if}
  </main>

  <footer class="flex items-center justify-between gap-4 border-t border-line px-6 py-2 text-xs text-muted">
    <span class="flex min-w-0 items-center gap-2">
      <span class="shrink-0">Весь звук в микрофон вкл/выкл:</span>
      <HotkeyInput value={hotkeys?.master || null} error={hotkeys?.errors["master"]} onset={(v) => setHotkey("master", v)} />
    </span>
    <span class="min-w-0 truncate" title="Крестик сворачивает MicMix в трей — звук продолжает идти.">
      Крестик сворачивает MicMix в трей — звук продолжает идти.
    </span>
  </footer>
</div>

{#if dropping}
  <div class="pointer-events-none fixed inset-3 z-30 grid place-items-center rounded-3xl border-2 border-dashed border-accent bg-accent/10 backdrop-blur-sm">
    <div class="flex flex-col items-center gap-2 text-white">
      <Icon name="music" size={36} />
      <div class="text-lg font-medium">Отпусти — звуки добавятся в «Звуки»</div>
    </div>
  </div>
{/if}

{#if toasts.length}
  <div class="fixed bottom-12 right-4 z-40 flex max-w-sm flex-col gap-2">
    {#each toasts as t (t.id)}
      <div class="rounded-xl border border-rose-500/30 bg-panel px-4 py-3 text-sm shadow-2xl">{t.text}</div>
    {/each}
  </div>
{/if}

{#if adding}
  <AddSourceDialog
    {added}
    tabError={mics?.tab_error}
    onadd={(exe, name) => {
      call("add_source", { exe, name });
      adding = false;
    }}
    onclose={() => (adding = false)}
  />
{/if}
