<script>
  // Звуки-кнопки, как в Soundpad: щелчок или горячая клавиша — звук идёт в микрофон.
  import { invoke } from "@tauri-apps/api/core";
  import { open } from "@tauri-apps/plugin-dialog";
  import HotkeyInput from "./HotkeyInput.svelte";
  import Icon from "./Icon.svelte";
  import Meter from "./Meter.svelte";
  import Volume from "./Volume.svelte";

  // status — ответ get_status; hotkeys — ответ get_hotkeys; onhotkey(target, value) → промис.
  let { status, hotkeys, onhotkey, onadded, refresh } = $props();

  const EXT = ["mp3", "wav", "ogg", "flac", "m4a", "aac", "mp4", "mka", "mkv", "webm"];
  let renaming = $state(null); // id звука, который переименовывают
  let newName = $state("");
  let soundsGain = $state(0.8);
  let monitorGain = $state(0.5);
  let dragging = $state(""); // какой общий ползунок двигают
  let gains = $state({}); // id → громкость, пока двигают ползунок звука

  $effect(() => {
    if (dragging !== "sounds") soundsGain = status.sounds_gain;
    if (dragging !== "monitor") monitorGain = status.monitor_gain;
  });

  async function addFiles() {
    const picked = await open({ multiple: true, filters: [{ name: "Звуки", extensions: EXT }] });
    if (picked) onadded?.(Array.isArray(picked) ? picked : [picked]);
  }

  function play(id) {
    invoke("play_sound", { id }).catch((e) => alert(String(e)));
  }

  function fmt(sec) {
    if (!sec) return "";
    const m = Math.floor(sec / 60);
    const s = Math.round(sec % 60);
    return m ? `${m}:${String(s).padStart(2, "0")}` : `${sec.toFixed(1)} с`;
  }

  async function commitRename(id) {
    const name = newName.trim();
    renaming = null;
    if (name) await invoke("rename_sound", { id, name }).catch(() => {});
    refresh?.();
  }
</script>

<div class="flex flex-col gap-4">
  <!-- Панель: добавить, стоп, громкость, прослушивание -->
  <div class="flex flex-wrap items-center gap-3 rounded-2xl border border-line bg-panel p-3">
    <button
      class="flex items-center gap-2 rounded-xl bg-gradient-to-r from-accent to-accent-2 px-4 py-2 text-sm font-medium text-white hover:brightness-110"
      onclick={addFiles}
    >
      <Icon name="plus" size={16} /> Добавить звуки
    </button>

    <div class="flex items-center gap-2">
      <button
        class="flex items-center gap-2 rounded-xl border border-line px-3 py-2 text-sm text-muted hover:border-rose-500/50 hover:text-rose-300"
        onclick={() => invoke("stop_sounds").then(refresh)}
      >
        <Icon name="stop" size={14} /> Стоп
      </button>
      <HotkeyInput value={hotkeys?.stop} error={hotkeys?.errors["stop"]} compact onset={(v) => onhotkey("stop", v)} />
    </div>

    <div class="ml-auto flex items-center gap-3">
      <div class="flex w-52 flex-col gap-1" title="Громкость звуков в микрофоне">
        <div class="flex items-center gap-2 text-xs text-muted"><Icon name="music" size={13} /> Звуки в микрофон</div>
        <Volume
          value={soundsGain}
          oninput={(v) => {
            dragging = "sounds";
            soundsGain = v;
            invoke("set_sounds_gain", { gain: v, save: false });
          }}
          onchange={(v) => {
            dragging = "";
            invoke("set_sounds_gain", { gain: v, save: true });
          }}
        />
        <div class="pr-14"><Meter level={status.sounds_level} /></div>
      </div>

      <div class="flex w-52 flex-col gap-1" title="Слышать звуки самому — в наушниках или колонках">
        <label class="flex cursor-pointer items-center gap-2 text-xs {status.monitor ? 'text-white' : 'text-muted'}">
          <input
            type="checkbox"
            class="accent-violet-500"
            checked={status.monitor}
            onchange={(e) => invoke("set_monitor", { on: e.currentTarget.checked }).then(refresh)}
          />
          <Icon name="headphones" size={13} /> Слышать самому
        </label>
        <Volume
          value={monitorGain}
          disabled={!status.monitor}
          oninput={(v) => {
            dragging = "monitor";
            monitorGain = v;
            invoke("set_monitor_gain", { gain: v, save: false });
          }}
          onchange={(v) => {
            dragging = "";
            invoke("set_monitor_gain", { gain: v, save: true });
          }}
        />
      </div>
    </div>
  </div>

  {#if status.sounds.length === 0}
    <button
      class="flex min-h-56 flex-col items-center justify-center gap-3 rounded-2xl border border-dashed border-line text-muted transition hover:border-accent/60 hover:text-white"
      onclick={addFiles}
    >
      <div class="grid h-14 w-14 place-items-center rounded-2xl bg-panel-2"><Icon name="music" size={26} /></div>
      <div class="text-sm">Перетащи сюда звуки (mp3, wav, ogg, flac) или нажми, чтобы выбрать</div>
      <div class="text-xs">Каждому звуку можно назначить горячую клавишу — как в Soundpad</div>
    </button>
  {/if}

  <div class="grid grid-cols-[repeat(auto-fill,minmax(210px,1fr))] gap-3">
    {#each status.sounds as sound (sound.id)}
      {@const playing = sound.progress !== null}
      {@const gain = gains[sound.id] ?? sound.gain}
      <div
        class="group relative flex flex-col gap-2.5 overflow-hidden rounded-2xl border bg-panel p-3 transition
          {playing ? 'border-accent/70' : 'border-line hover:border-accent/40'}"
      >
        {#if playing}
          <div class="absolute inset-x-0 top-0 h-0.5 bg-line">
            <div class="h-full bg-gradient-to-r from-accent to-accent-2" style="width:{sound.progress * 100}%"></div>
          </div>
        {/if}

        <div class="flex items-center gap-2.5">
          <button
            class="grid h-10 w-10 shrink-0 place-items-center rounded-xl transition
              {playing ? 'bg-gradient-to-br from-accent to-accent-2 text-white' : 'bg-panel-2 text-white hover:bg-accent/30'}"
            title="Проиграть в микрофон"
            onclick={() => play(sound.id)}
          >
            <Icon name="play" size={16} />
          </button>
          <div class="min-w-0 flex-1">
            {#if renaming === sound.id}
              <!-- svelte-ignore a11y_autofocus -->
              <input
                class="w-full rounded-md border border-accent bg-panel-2 px-1.5 py-0.5 text-sm outline-none"
                bind:value={newName}
                autofocus
                onkeydown={(e) => {
                  if (e.key === "Enter") commitRename(sound.id);
                  if (e.key === "Escape") renaming = null;
                }}
                onblur={() => commitRename(sound.id)}
              />
            {:else}
              <button
                class="block w-full truncate text-left text-sm font-medium"
                title="Двойной щелчок — переименовать"
                ondblclick={() => {
                  renaming = sound.id;
                  newName = sound.name;
                }}
              >
                {sound.name}
              </button>
            {/if}
            <div class="text-xs text-muted">{fmt(sound.seconds)}</div>
          </div>
          <button
            class="rounded-lg p-1 text-muted opacity-0 transition hover:bg-panel-2 hover:text-white group-hover:opacity-100"
            title="Удалить звук"
            onclick={() => invoke("remove_sound", { id: sound.id }).then(refresh)}
          >
            <Icon name="x" size={14} />
          </button>
        </div>

        <Volume
          value={gain}
          oninput={(v) => {
            gains[sound.id] = v;
            invoke("set_sound_gain", { id: sound.id, gain: v, save: false });
          }}
          onchange={(v) => {
            delete gains[sound.id];
            invoke("set_sound_gain", { id: sound.id, gain: v, save: true });
          }}
        />

        <HotkeyInput
          value={sound.hotkey}
          error={hotkeys?.errors[`sound:${sound.id}`]}
          compact
          onset={(v) => onhotkey(`sound:${sound.id}`, v)}
        />
      </div>
    {/each}
  </div>
</div>
