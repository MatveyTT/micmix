<script>
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { onMount } from "svelte";
  import AddSourceDialog from "./lib/AddSourceDialog.svelte";
  import Icon from "./lib/Icon.svelte";
  import Meter from "./lib/Meter.svelte";
  import MicPanel from "./lib/MicPanel.svelte";
  import SourceCard from "./lib/SourceCard.svelte";
  import Volume from "./lib/Volume.svelte";

  const TONE = "micmix:tone";

  let status = $state(null);
  let mics = $state(null); // ответ list_mics: { mics, system, hotkey }
  let tab = $state("sources");
  let adding = $state(false);
  let masterGain = $state(0.7);
  let draggingMaster = $state(false);

  const installed = $derived(mics?.mics.filter((m) => m.apo_installed) ?? []);
  const sources = $derived(status?.sources.filter((s) => s.exe !== TONE) ?? []);
  const toneOn = $derived(status?.sources.some((s) => s.exe === TONE) ?? false);
  const added = $derived(new Set(status?.sources.map((s) => s.exe) ?? []));

  const banner = $derived.by(() => {
    if (!status || !mics) return { kind: "load", text: "Загрузка…" };
    if (installed.length === 0) return { kind: "warn", text: "MicMix не установлен на микрофон" };
    if (status.master_muted) return { kind: "off", text: "Звук в микрофон выключен" };
    if (!status.apo_connected) return { kind: "idle", text: "Ждёт: микрофон сейчас никто не записывает" };
    return { kind: "live", text: `В эфире · ${installed.map((m) => m.name).join(", ")} · ${status.sample_rate / 1000} кГц` };
  });

  async function poll() {
    try {
      status = await invoke("get_status");
      if (!draggingMaster) masterGain = status.master_gain;
    } catch (e) {
      console.error(e);
    }
  }

  onMount(() => {
    poll();
    invoke("list_mics").then((m) => {
      mics = m;
      if (m.mics.every((x) => !x.apo_installed)) tab = "mic";
    });
    const t = setInterval(poll, 80);
    const un = listen("master-muted", poll);
    return () => {
      clearInterval(t);
      un.then((f) => f());
    };
  });

  const call = (cmd, args) => invoke(cmd, args).then(poll);
</script>

<div class="flex h-full flex-col">
  <!-- Шапка: логотип, статус, общая громкость -->
  <header class="flex items-center gap-5 border-b border-line bg-panel/60 px-6 py-4">
    <div class="flex items-center gap-3">
      <div class="grid h-10 w-10 place-items-center rounded-xl bg-gradient-to-br from-accent to-accent-2 shadow-lg shadow-accent/20">
        <Icon name="mic" size={20} class="text-white" />
      </div>
      <div>
        <div class="text-lg font-semibold leading-tight">MicMix</div>
        <div class="flex items-center gap-1.5 text-xs">
          <span
            class="h-2 w-2 rounded-full
              {banner.kind === 'live' ? 'animate-pulse bg-emerald-400' : banner.kind === 'warn' ? 'bg-amber-400' : banner.kind === 'off' ? 'bg-rose-400' : 'bg-muted'}"
          ></span>
          <span class="max-w-80 truncate text-muted" title={banner.text}>{banner.text}</span>
        </div>
      </div>
    </div>

    <div class="ml-auto flex items-center gap-3">
      <button
        class="flex items-center gap-2 rounded-xl border px-3 py-2 text-sm transition
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
          title={`${status?.master_muted ? "Включить" : "Выключить"} звук в микрофоне${mics ? ` (${mics.hotkey})` : ""}`}
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
  <nav class="flex gap-1 px-6 pt-3">
    {#each [["sources", "Источники", "sliders"], ["mic", "Микрофон", "mic"]] as [id, label, icon]}
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
      </button>
    {/each}
    {#if status && status.limiter > 0.05}
      <div class="ml-auto flex items-center gap-1.5 self-center text-xs text-amber-300" title="Слишком громко: лимитер приглушает звук, чтобы не было хрипа">
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

      <div class="grid grid-cols-[repeat(auto-fill,minmax(280px,1fr))] gap-4">
        {#each sources as source (source.exe)}
          <SourceCard
            {source}
            ongain={(exe, gain, save) => invoke("set_gain", { exe, gain, save })}
            onmute={(exe, muted) => call("set_muted", { exe, muted })}
            onremove={(exe) => call("remove_source", { exe })}
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
    {:else}
      <MicPanel onchanged={(m) => (mics = m)} />
    {/if}
  </main>

  <footer class="flex items-center gap-2 border-t border-line px-6 py-2 text-xs text-muted">
    <Icon name="keyboard" size={14} />
    {mics?.hotkey ?? "Ctrl+Shift+M"} — выключить/включить звук в микрофоне. Крестик сворачивает MicMix в трей, звук продолжает идти.
  </footer>
</div>

{#if adding}
  <AddSourceDialog
    {added}
    onadd={(exe, name) => {
      call("add_source", { exe, name });
      adding = false;
    }}
    onclose={() => (adding = false)}
  />
{/if}
