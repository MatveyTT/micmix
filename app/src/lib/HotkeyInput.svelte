<script>
  // Кнопка с горячей клавишей: щелчок — записать новое сочетание, × — убрать.
  // onset(значение | null) должен вернуть промис; ошибка (клавиша занята) показывается рядом.
  import Icon from "./Icon.svelte";

  let { value = null, error = null, onset, compact = false, placeholder = "Клавиша", clearable = true } = $props();
  let recording = $state(false);
  let failure = $state("");
  const shown = $derived(failure || error);

  // KeyboardEvent → строка для Tauri: «Ctrl+Alt+M», «Ctrl+Shift+F9», «F7», «Ctrl+Numpad1».
  function combo(e) {
    const c = e.code;
    const key = c.startsWith("Key")
      ? c.slice(3)
      : c.startsWith("Digit")
        ? c.slice(5)
        : /^F\d{1,2}$/.test(c) || /^Numpad\d$/.test(c)
          ? c
          : null;
    if (!key) return null;
    const mods = [e.ctrlKey && "Ctrl", e.altKey && "Alt", e.shiftKey && "Shift"].filter(Boolean);
    // Одиночная буква или цифра без модификаторов мешала бы печатать; F-клавиши и цифры
    // на цифровом блоке можно одни — как в Soundpad.
    if (mods.length === 0 && !/^F\d|^Numpad/.test(key)) return null;
    return [...mods, key].join("+");
  }

  async function apply(v) {
    try {
      await onset?.(v);
      failure = "";
    } catch (err) {
      failure = String(err);
    }
  }

  function keydown(e) {
    if (!recording) return;
    e.preventDefault();
    e.stopPropagation();
    if (e.key === "Escape") {
      recording = false;
      return;
    }
    if (e.code === "F12") {
      // Windows резервирует F12 для отладчика: регистрация проходит, но нажатия не приходят.
      recording = false;
      failure = "F12 Windows не отдаёт программам — выбери другую клавишу";
      return;
    }
    const v = combo(e);
    if (!v) return; // ждём полноценное сочетание
    recording = false;
    apply(v);
  }
</script>

<svelte:window onkeydown={keydown} />

<span class="inline-flex min-w-0 items-center gap-1 whitespace-nowrap">
  <button
    class="inline-flex items-center gap-1.5 rounded-md border px-2 py-0.5 text-xs transition
      {recording
        ? 'border-accent bg-accent/15 text-white'
        : shown
          ? 'border-amber-500/50 text-amber-200 hover:border-amber-400'
          : value
            ? 'border-line bg-panel-2 text-white hover:border-accent/60'
            : 'border-dashed border-line text-muted hover:border-accent/60 hover:text-white'}"
    title={recording ? "Нажми сочетание, Esc — отмена" : shown ? shown : "Щёлкни и нажми сочетание клавиш"}
    onclick={() => (recording = !recording)}
  >
    <Icon name="keyboard" size={12} />
    {#if recording}
      Нажми…
    {:else}
      {value ?? placeholder}
    {/if}
  </button>
  {#if value && clearable && !recording}
    <button class="rounded p-0.5 text-muted hover:text-white" title="Убрать клавишу" onclick={() => apply(null)}>
      <Icon name="x" size={12} />
    </button>
  {/if}
  {#if shown && !compact && !recording}
    <span class="truncate text-xs text-amber-300">{shown}</span>
  {/if}
</span>
