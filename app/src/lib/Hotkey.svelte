<script>
  import { invoke } from "@tauri-apps/api/core";
  import Icon from "./Icon.svelte";

  // hotkey — текущая клавиша, error — почему она не работает; onchanged — перечитать настройки.
  let { hotkey, error, onchanged } = $props();
  let recording = $state(false);
  let failure = $state("");

  // KeyboardEvent → строка для Tauri: «Ctrl+Alt+M», «Ctrl+Shift+F9».
  function combo(e) {
    const key = e.code.startsWith("Key")
      ? e.code.slice(3)
      : e.code.startsWith("Digit")
        ? e.code.slice(5)
        : /^F\d{1,2}$/.test(e.code)
          ? e.code
          : null;
    if (!key) return null;
    const mods = [e.ctrlKey && "Ctrl", e.altKey && "Alt", e.shiftKey && "Shift"].filter(Boolean);
    // Одиночная буква без модификаторов мешала бы печатать.
    if (mods.length === 0 && !key.startsWith("F")) return null;
    return [...mods, key].join("+");
  }

  async function keydown(e) {
    if (!recording) return;
    e.preventDefault();
    if (e.key === "Escape") {
      recording = false;
      return;
    }
    const value = combo(e);
    if (!value) return; // ждём полноценное сочетание
    recording = false;
    try {
      await invoke("set_hotkey", { hotkey: value });
      failure = "";
    } catch (err) {
      failure = String(err);
    }
    onchanged?.();
  }
</script>

<svelte:window onkeydown={keydown} />

<div class="flex shrink-0 items-center gap-2 whitespace-nowrap">
  <Icon name="keyboard" size={14} />
  {#if recording}
    <span class="rounded-md border border-accent bg-accent/15 px-2 py-0.5 text-white">Нажми сочетание… (Esc — отмена)</span>
  {:else}
    <kbd class="rounded-md border border-line bg-panel-2 px-2 py-0.5 font-sans text-white">{hotkey}</kbd>
    <span>— звук в микрофон вкл/выкл</span>
    <button class="rounded-md px-1.5 py-0.5 text-accent hover:bg-panel-2" onclick={() => (recording = true)}>Изменить</button>
  {/if}
  {#if !recording && (failure || error)}
    <span class="text-amber-300">· {failure || error}</span>
  {/if}
</div>
