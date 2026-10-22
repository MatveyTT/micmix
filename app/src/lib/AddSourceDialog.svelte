<script>
  import { invoke } from "@tauri-apps/api/core";
  import { onMount } from "svelte";
  import Icon from "./Icon.svelte";

  // added: Set имён exe, которые уже в микшере.
  let { added, tabError = null, onadd, onclose } = $props();
  let extensionHint = $state("");

  async function openExtension() {
    try {
      const dir = await invoke("open_extension_folder");
      extensionHint = `Папка открыта: ${dir}. В браузере: chrome://extensions → «Режим разработчика» → «Загрузить распакованное» → эта папка.`;
    } catch (e) {
      extensionHint = String(e);
    }
  }
  let apps = $state([]);
  let loading = $state(true);
  let manual = $state("");

  async function refresh() {
    loading = true;
    try {
      apps = await invoke("list_apps");
    } finally {
      loading = false;
    }
  }

  function addManual() {
    const exe = manual.trim().toLowerCase();
    if (!exe) return;
    const full = exe.endsWith(".exe") ? exe : `${exe}.exe`;
    onadd?.(full, full.replace(/\.exe$/, ""));
    manual = "";
  }

  onMount(() => {
    refresh();
    // Список сам обновляется: приложение могло только что начать играть.
    const t = setInterval(refresh, 3000);
    return () => clearInterval(t);
  });
</script>

<svelte:window onkeydown={(e) => e.key === "Escape" && onclose?.()} />

<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
<div class="fixed inset-0 z-20 grid place-items-center bg-black/60 p-6 backdrop-blur-sm" onclick={onclose}>
  <div
    class="flex max-h-full w-full max-w-lg flex-col overflow-hidden rounded-2xl border border-line bg-panel shadow-2xl"
    onclick={(e) => e.stopPropagation()}
  >
    <div class="flex items-center justify-between border-b border-line px-5 py-4">
      <div>
        <div class="font-semibold">Добавить источник</div>
        <div class="text-xs text-muted">Приложения, которые сейчас выводят звук</div>
      </div>
      <div class="flex gap-1">
        <button class="rounded-lg p-2 text-muted hover:bg-panel-2 hover:text-white" title="Обновить" onclick={refresh}>
          <Icon name="refresh" size={16} class={loading ? "animate-spin" : ""} />
        </button>
        <button class="rounded-lg p-2 text-muted hover:bg-panel-2 hover:text-white" title="Закрыть" onclick={onclose}>
          <Icon name="x" size={16} />
        </button>
      </div>
    </div>

    <div class="min-h-40 flex-1 overflow-y-auto p-2">
      <!-- Отдельная вкладка браузера — через расширение MicMix -->
      <div class="mb-1 rounded-xl border border-line/60 bg-panel-2/40 p-1">
        <button
          class="flex w-full items-center gap-3 rounded-lg px-2 py-2 text-left transition hover:bg-panel-2 disabled:opacity-50"
          disabled={added.has("micmix:tab") || !!tabError}
          onclick={() => onadd?.("micmix:tab", "Вкладка браузера")}
        >
          <div class="grid h-9 w-9 shrink-0 place-items-center rounded-lg bg-gradient-to-br from-accent to-accent-2 text-white">
            <Icon name="globe" size={18} />
          </div>
          <div class="min-w-0 flex-1">
            <div class="text-sm font-medium">Вкладка браузера</div>
            <div class="truncate text-xs text-muted">
              {tabError ? `недоступно: ${tabError}` : "одна вкладка, а не весь браузер — нужно расширение MicMix"}
            </div>
          </div>
          {#if added.has("micmix:tab")}<Icon name="check" size={16} class="text-emerald-400" />{/if}
        </button>
        <button class="flex items-center gap-1.5 px-2 pb-1 text-xs text-accent hover:underline" onclick={openExtension}>
          <Icon name="folder" size={12} /> Установить расширение в браузер
        </button>
        {#if extensionHint}
          <div class="px-2 pb-1 text-xs text-muted">{extensionHint}</div>
        {/if}
      </div>

      {#if apps.length === 0 && !loading}
        <div class="px-4 py-10 text-center text-sm text-muted">
          Ничего не играет. Включи музыку или видео — приложение появится здесь.
        </div>
      {/if}
      {#each apps as app (app.exe)}
        {@const isAdded = added.has(app.exe)}
        <button
          class="flex w-full items-center gap-3 rounded-xl px-3 py-2.5 text-left transition hover:bg-panel-2 disabled:opacity-50"
          disabled={isAdded}
          onclick={() => onadd?.(app.exe, app.name)}
        >
          <div class="grid h-9 w-9 shrink-0 place-items-center">
            {#if app.icon}
              <img src={app.icon} alt="" class="h-8 w-8" />
            {:else}
              <Icon name="app" size={20} class="text-muted" />
            {/if}
          </div>
          <div class="min-w-0 flex-1">
            <div class="truncate text-sm font-medium">{app.name}</div>
            <div class="truncate text-xs text-muted">{app.exe}</div>
          </div>
          {#if isAdded}
            <Icon name="check" size={16} class="text-emerald-400" />
          {:else if app.playing}
            <span class="rounded-full bg-emerald-400/15 px-2 py-0.5 text-[11px] text-emerald-300">играет</span>
          {/if}
        </button>
      {/each}
    </div>

    <form
      class="flex gap-2 border-t border-line p-3"
      onsubmit={(e) => {
        e.preventDefault();
        addManual();
      }}
    >
      <input
        class="flex-1 rounded-xl border border-line bg-panel-2 px-3 py-2 text-sm outline-none placeholder:text-muted focus:border-accent"
        placeholder="Или впиши имя программы: spotify.exe"
        bind:value={manual}
      />
      <button class="rounded-xl bg-accent px-4 text-sm font-medium hover:brightness-110" type="submit">Добавить</button>
    </form>
  </div>
</div>
