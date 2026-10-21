<script>
  // Ползунок громкости 0..200 %. onchange вызывается при отпускании и при прокрутке колёсиком.
  let { value = 1, disabled = false, oninput, onchange } = $props();
  let pct = $derived(Math.round(value * 100));

  function input(e) {
    oninput?.(Number(e.currentTarget.value) / 100);
  }
  function wheel(e) {
    if (disabled) return;
    e.preventDefault();
    const next = Math.max(0, Math.min(200, pct + (e.deltaY < 0 ? 5 : -5)));
    oninput?.(next / 100);
    onchange?.(next / 100);
  }
</script>

<div class="flex items-center gap-3">
  <input
    class="vol"
    type="range"
    min="0"
    max="200"
    step="1"
    value={pct}
    {disabled}
    style="--fill:{pct / 2}%"
    oninput={input}
    onchange={(e) => onchange?.(Number(e.currentTarget.value) / 100)}
    onwheel={wheel}
  />
  <span class="w-11 shrink-0 text-right text-xs tabular-nums text-muted">{pct}%</span>
</div>
