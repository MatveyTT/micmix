// Захват звука вкладки и отправка в MicMix по WebSocket (ws://127.0.0.1:47823).
// Формат: сначала текст {"rate":48000}, потом двоичные кадры float32 стерео (L, R, ...).

const WS_URL = "ws://127.0.0.1:47823";
const RATE = 48000;

let ctx = null;
let stream = null;
let ws = null;
let stopping = false;

function report(type, message) {
  chrome.runtime.sendMessage({ target: "background", type, message });
}

async function start(streamId) {
  await stop(false);
  stopping = false;
  stream = await navigator.mediaDevices.getUserMedia({
    audio: { mandatory: { chromeMediaSource: "tab", chromeMediaSourceId: streamId } },
  });
  ctx = new AudioContext({ sampleRate: RATE });
  await ctx.audioWorklet.addModule("worklet.js");
  const source = ctx.createMediaStreamSource(stream);
  // Захват вкладки глушит её звук — возвращаем его в колонки, чтобы ты тоже слышал.
  source.connect(ctx.destination);
  const tap = new AudioWorkletNode(ctx, "micmix-tap", { numberOfInputs: 1, numberOfOutputs: 0 });
  source.connect(tap);
  tap.port.onmessage = (e) => {
    if (ws && ws.readyState === WebSocket.OPEN && ws.bufferedAmount < 1 << 20) ws.send(e.data);
  };
  connect();
}

// Подключение к приложению; если MicMix не запущен — пробуем раз в 2 секунды.
function connect() {
  if (stopping) return;
  ws = new WebSocket(WS_URL);
  ws.binaryType = "arraybuffer";
  ws.onopen = () => {
    ws.send(JSON.stringify({ rate: ctx.sampleRate }));
    report("connected");
  };
  ws.onclose = () => {
    if (stopping) return;
    report("error", "приложение MicMix не запущено — жду");
    setTimeout(connect, 2000);
  };
}

async function stop(notify = true) {
  stopping = true;
  if (ws) ws.close();
  ws = null;
  if (stream) stream.getTracks().forEach((t) => t.stop());
  stream = null;
  if (ctx) await ctx.close();
  ctx = null;
  if (notify) report("stopped");
}

chrome.runtime.onMessage.addListener((msg) => {
  if (msg.target !== "offscreen") return;
  if (msg.type === "start") start(msg.streamId).catch((e) => report("error", e.message));
  if (msg.type === "stop") stop();
});
