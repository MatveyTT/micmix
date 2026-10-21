<script>
  import Icon from "./Icon.svelte";
  import Meter from "./Meter.svelte";
  import Volume from "./Volume.svelte";

  // source: { exe, name, icon, gain, muted, level, running }
  let { source, ongain, onmute, onremove } = $props();
  let gain = $state(1); // сразу заменит эффект ниже
  let dragging = $state(false);
  // Пока пользователь двигает ползунок, не перетираем его значение опросом.
  $effect(() => {
    if (!dragging) gain = source.gain;
  });
</script>

<div
  class="group relative flex flex-col gap-4 rounded-2xl border border-line bg-panel p-4 transition
    hover:border-accent/40 {source.running ? '' : 'opacity-70'}"
>
  <div class="flex items-center gap-3">
    <div class="grid h-11 w-11 shrink-0 place-items-center overflow-hidden rounded-xl bg-panel-2">
      {#if source.icon}
        <img src={source.icon} alt="" class="h-8 w-8" />
      {:else}
        <Icon name={source.exe === "micmix:tone" ? "wave" : "app"} size={22} class="text-muted" />
      {/if}
    </div>
    <div class="min-w-0 flex-1">
      <div class="truncate font-medium">{source.name}</div>
      <div class="flex items-center gap-1.5 text-xs text-muted">
        <span class="h-1.5 w-1.5 rounded-full {source.running ? 'bg-emerald-400' : 'bg-amber-400'}"></span>
        {source.running ? (source.exe === "micmix:tone" ? "880 Гц, раз в секунду" : source.exe) : "не запущено — жду"}
      </div>
    </div>
    <button
      class="rounded-lg p-1.5 text-muted opacity-0 transition hover:bg-panel-2 hover:text-white group-hover:opacity-100"
      title="Убрать источник"
      onclick={() => onremove?.(source.exe)}
    >
      <Icon name="x" size={16} />
    </button>
  </div>

  <Meter level={source.level} muted={source.muted} />

  <div class="flex items-center gap-2">
    <button
      class="grid h-9 w-9 shrink-0 place-items-center rounded-xl transition
        {source.muted ? 'bg-rose-500/15 text-rose-400 hover:bg-rose-500/25' : 'bg-panel-2 text-muted hover:text-white'}"
      title={source.muted ? "Включить звук" : "Выключить звук"}
      onclick={() => onmute?.(source.exe, !source.muted)}
    >
      <Icon name={source.muted ? "volume-x" : "volume"} size={17} />
    </button>
    <div class="flex-1">
      <Volume
        value={gain}
        disabled={source.muted}
        oninput={(v) => {
          dragging = true;
          gain = v;
          ongain?.(source.exe, v, false);
        }}
        onchange={(v) => {
          dragging = false;
          ongain?.(source.exe, v, true);
        }}
      />
    </div>
  </div>
</div>
