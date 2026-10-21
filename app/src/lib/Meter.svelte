<script>
  // Индикатор уровня: 0..1 линейно, показываем в дБ (−48..0), чтобы тихий звук тоже было видно.
  let { level = 0, muted = false, vertical = false } = $props();
  const pct = $derived(level <= 0.0001 ? 0 : Math.max(0, Math.min(100, ((20 * Math.log10(level) + 48) / 48) * 100)));
  const hot = $derived(level > 0.85);
</script>

<div
  class="relative overflow-hidden rounded-full bg-line {vertical ? 'h-full w-1.5' : 'h-1.5 w-full'}"
  title="{Math.round(pct)}%"
>
  <div
    class="absolute rounded-full transition-[width,height] duration-75 ease-out
      {vertical ? 'bottom-0 left-0 w-full' : 'left-0 top-0 h-full'}
      {muted ? 'bg-muted/40' : hot ? 'bg-gradient-to-r from-amber-400 to-rose-500' : 'bg-gradient-to-r from-emerald-400 to-cyan-400'}"
    style={vertical ? `height:${pct}%` : `width:${pct}%`}
  ></div>
</div>
