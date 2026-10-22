<script>
  // Линейные иконки в стиле Lucide (24×24, обводка currentColor).
  let { name, size = 18, class: cls = "" } = $props();
  const paths = {
    mic: "M12 2a3 3 0 0 0-3 3v7a3 3 0 0 0 6 0V5a3 3 0 0 0-3-3Z|M19 10v2a7 7 0 0 1-14 0v-2|M12 19v3",
    "mic-off": "M2 2l20 20|M18.9 13.9A7 7 0 0 0 19 12v-2|M5 10v2a7 7 0 0 0 12 5|M15 9.3V5a3 3 0 0 0-5.7-1.3|M9 9v3a3 3 0 0 0 5.1 2.1|M12 19v3",
    volume: "M11 5 6 9H2v6h4l5 4V5Z|M15.5 8.5a5 5 0 0 1 0 7|M19 5a10 10 0 0 1 0 14",
    "volume-x": "M11 5 6 9H2v6h4l5 4V5Z|M22 9l-6 6|M16 9l6 6",
    plus: "M12 5v14|M5 12h14",
    x: "M18 6 6 18|M6 6l12 12",
    refresh: "M3 12a9 9 0 0 1 15-6.7L21 8|M21 3v5h-5|M21 12a9 9 0 0 1-15 6.7L3 16|M3 21v-5h5",
    check: "M20 6 9 17l-5-5",
    wave: "M2 12h2|M6 8v8|M10 4v16|M14 7v10|M18 10v4|M22 12h0",
    sliders: "M4 21v-7|M4 10V3|M12 21v-9|M12 8V3|M20 21v-5|M20 12V3|M2 14h4|M10 8h4|M18 16h4",
    shield: "M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10Z",
    alert: "M12 9v4|M12 17h.01|M10.3 3.9 1.8 18a2 2 0 0 0 1.7 3h17a2 2 0 0 0 1.7-3L13.7 3.9a2 2 0 0 0-3.4 0Z",
    app: "M4 4h16v16H4z|M4 9h16",
    keyboard: "M2 6h20v12H2z|M6 10h.01|M10 10h.01|M14 10h.01|M18 10h.01|M7 14h10",
    play: "M6 4l14 8-14 8V4Z",
    stop: "M6 6h12v12H6z",
    headphones: "M3 18v-6a9 9 0 0 1 18 0v6|M21 19a2 2 0 0 1-2 2h-1v-6h3v4Z|M3 19a2 2 0 0 0 2 2h1v-6H3v4Z",
    music: "M9 18V5l12-2v13|M9 18a3 3 0 1 1-6 0 3 3 0 0 1 6 0Z|M21 16a3 3 0 1 1-6 0 3 3 0 0 1 6 0Z",
    folder: "M3 6a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v10a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V6Z",
    globe: "M12 2a10 10 0 1 0 0 20 10 10 0 0 0 0-20Z|M2 12h20|M12 2a15 15 0 0 1 0 20|M12 2a15 15 0 0 0 0 20",
    apps: "M3 3h7v7H3z|M14 3h7v7h-7z|M3 14h7v7H3z|M14 14h7v7h-7z",
  };
</script>

<svg
  class={cls}
  width={size}
  height={size}
  viewBox="0 0 24 24"
  fill="none"
  stroke="currentColor"
  stroke-width="2"
  stroke-linecap="round"
  stroke-linejoin="round"
  aria-hidden="true"
>
  {#each (paths[name] ?? "").split("|") as d}
    <path {d} />
  {/each}
</svg>
