import { mount } from "svelte";
import "./app.css";
import App from "./App.svelte";

// Масштаб щипком на тачпаде приходит как колесо мыши с Ctrl, а жесты — отдельными
// событиями: гасим их, чтобы окно приложения не масштабировалось и не уезжало.
const block = (e) => e.preventDefault();
window.addEventListener("wheel", (e) => e.ctrlKey && e.preventDefault(), { passive: false });
for (const ev of ["gesturestart", "gesturechange", "gestureend"]) window.addEventListener(ev, block, { passive: false });
window.addEventListener("keydown", (e) => {
  // Ctrl + / Ctrl − / Ctrl 0 — тоже масштаб страницы.
  if (e.ctrlKey && ["+", "-", "=", "0"].includes(e.key)) e.preventDefault();
});

export default mount(App, { target: document.getElementById("app") });
