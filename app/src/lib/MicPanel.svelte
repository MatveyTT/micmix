<script>
  import { invoke } from "@tauri-apps/api/core";
  import { onMount } from "svelte";
  import Icon from "./Icon.svelte";

  let { onchanged } = $props();
  let info = $state(null);
  let busy = $state(""); // endpoint, над которым идёт установка
  let message = $state(null); // { ok, text }

  async function refresh() {
    info = await invoke("list_mics");
    onchanged?.(info);
  }

  async function apply(mic, uninstall) {
    busy = mic.endpoint;
    message = null;
    try {
      await invoke("apo_install", { endpoint: mic.endpoint, uninstall });
      message = {
        ok: true,
        text: uninstall
          ? `MicMix снят с «${mic.name}», настройки микрофона восстановлены.`
          : `MicMix установлен на «${mic.name}».` +
            " Проверь кнопкой «Тестовый бип»." +
            (info?.system.unsigned_apo_allowed ? "" : " Если бипа в микрофоне нет — перезагрузи компьютер."),
      };
    } catch (e) {
      message = { ok: false, text: String(e) };
    } finally {
      busy = "";
      await refresh();
    }
  }

  onMount(refresh);
</script>

<div class="mx-auto flex max-w-3xl flex-col gap-5">
  <div>
    <h2 class="text-lg font-semibold">Микрофон</h2>
    <p class="mt-1 text-sm text-muted">
      MicMix встраивается в микрофон как звуковой эффект Windows. Discord, игры и любые другие программы
      слышат звук приложений вместе с голосом — в них ничего настраивать не нужно.
    </p>
  </div>

  {#if info && !info.system.unsigned_apo_allowed}
    <div class="flex gap-3 rounded-2xl border border-amber-500/30 bg-amber-500/10 p-4 text-sm">
      <Icon name="alert" size={18} class="mt-0.5 shrink-0 text-amber-400" />
      <div>
        При установке MicMix разрешит Windows загружать неподписанные звуковые эффекты
        (<code class="text-amber-200">DisableProtectedAudioDG</code>). Обычно эффект начинает работать сразу;
        если после установки звука нет — перезагрузи компьютер.
      </div>
    </div>
  {/if}

  <div class="flex flex-col gap-2">
    {#each info?.mics ?? [] as mic (mic.endpoint)}
      <div class="flex items-center gap-4 rounded-2xl border border-line bg-panel p-4">
        <div
          class="grid h-10 w-10 shrink-0 place-items-center rounded-xl
            {mic.apo_installed ? 'bg-gradient-to-br from-accent to-accent-2 text-white' : 'bg-panel-2 text-muted'}"
        >
          <Icon name="mic" size={18} />
        </div>
        <div class="min-w-0 flex-1">
          <div class="flex items-center gap-2">
            <span class="truncate font-medium">{mic.name}</span>
            {#if mic.is_default}
              <span class="rounded-full bg-panel-2 px-2 py-0.5 text-[11px] text-muted">по умолчанию</span>
            {/if}
          </div>
          <div class="truncate text-xs text-muted">{mic.adapter}</div>
        </div>
        {#if mic.apo_installed}
          <span class="flex items-center gap-1 text-xs text-emerald-300"><Icon name="check" size={14} /> MicMix стоит</span>
          <button
            class="rounded-xl border border-line px-3 py-2 text-sm text-muted hover:border-rose-500/50 hover:text-rose-300 disabled:opacity-50"
            disabled={!!busy}
            onclick={() => apply(mic, true)}
          >
            {busy === mic.endpoint ? "Снимаю…" : "Снять"}
          </button>
        {:else}
          <button
            class="rounded-xl bg-gradient-to-r from-accent to-accent-2 px-4 py-2 text-sm font-medium text-white hover:brightness-110 disabled:opacity-50"
            disabled={!!busy}
            onclick={() => apply(mic, false)}
          >
            {busy === mic.endpoint ? "Устанавливаю…" : "Установить"}
          </button>
        {/if}
      </div>
    {/each}
  </div>

  {#if busy}
    <div class="text-sm text-muted">
      Windows спросит разрешение администратора. Служба звука перезапустится — на пару секунд звук пропадёт.
    </div>
  {/if}
  {#if message}
    <div
      class="whitespace-pre-line rounded-2xl border p-4 text-sm
        {message.ok ? 'border-emerald-500/30 bg-emerald-500/10' : 'border-rose-500/30 bg-rose-500/10'}"
    >
      {message.text}
    </div>
  {/if}

  <div class="rounded-2xl border border-line bg-panel p-4 text-sm text-muted">
    <div class="mb-2 font-medium text-white">Если собеседники не слышат звук</div>
    <ul class="list-disc space-y-1 pl-5">
      <li>В Discord и игре должен быть выбран тот микрофон, на который установлен MicMix.</li>
      <li>
        В Discord выключи «Обход системной обработки звука» (Голос и видео) — иначе он берёт микрофон
        в обход MicMix.
      </li>
      <li>Отключи шумоподавление (Krisp и т. п.) — оно вырезает музыку как шум.</li>
      <li>Программы, которые открывают микрофон в «сыром» режиме (RAW), обходят эффекты Windows.</li>
      <li>Проверь кнопкой «Тестовый бип» и записью в «Звукозаписи» Windows.</li>
    </ul>
  </div>
</div>
