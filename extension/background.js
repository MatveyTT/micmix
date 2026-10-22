// MicMix: щелчок по значку включает/выключает трансляцию звука вкладки в микрофон.
// Сам звук захватывает offscreen-документ (у service worker нет Web Audio).

let capturedTab = null; // id вкладки, которая сейчас транслируется

async function ensureOffscreen() {
  if (await chrome.offscreen.hasDocument()) return;
  await chrome.offscreen.createDocument({
    url: "offscreen.html",
    reasons: ["USER_MEDIA"],
    justification: "Захват звука вкладки для передачи в приложение MicMix",
  });
}

function badge(tabId, text, color, title) {
  chrome.action.setBadgeText({ tabId, text });
  if (color) chrome.action.setBadgeBackgroundColor({ tabId, color });
  chrome.action.setTitle({ tabId, title });
}

async function stop() {
  if (capturedTab !== null) {
    badge(capturedTab, "", null, "MicMix: транслировать вкладку в микрофон");
  }
  capturedTab = null;
  if (await chrome.offscreen.hasDocument()) {
    chrome.runtime.sendMessage({ target: "offscreen", type: "stop" });
  }
}

chrome.action.onClicked.addListener(async (tab) => {
  if (capturedTab === tab.id) {
    await stop();
    return;
  }
  await stop(); // одна вкладка за раз
  try {
    const streamId = await chrome.tabCapture.getMediaStreamId({ targetTabId: tab.id });
    await ensureOffscreen();
    capturedTab = tab.id;
    badge(tab.id, "…", "#8b5cf6", "MicMix: подключаюсь к приложению…");
    chrome.runtime.sendMessage({ target: "offscreen", type: "start", streamId });
  } catch (e) {
    badge(tab.id, "ERR", "#e11d48", `MicMix: ${e.message}`);
  }
});

// Состояние из offscreen-документа.
chrome.runtime.onMessage.addListener((msg) => {
  if (msg.target !== "background" || capturedTab === null) return;
  if (msg.type === "connected") {
    badge(capturedTab, "ON", "#10b981", "MicMix: вкладка идёт в микрофон. Щёлкни, чтобы остановить");
  } else if (msg.type === "error") {
    badge(capturedTab, "ERR", "#e11d48", `MicMix: ${msg.message}`);
  } else if (msg.type === "stopped") {
    badge(capturedTab, "", null, "MicMix: транслировать вкладку в микрофон");
    capturedTab = null;
  }
});

// Вкладку закрыли — останавливаемся.
chrome.tabs.onRemoved.addListener((tabId) => {
  if (tabId === capturedTab) stop();
});
