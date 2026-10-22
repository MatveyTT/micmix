// Аудио-поток: собирает стерео-сэмплы вкладки пачками по ~20 мс и отдаёт их
// offscreen-документу, который шлёт их в MicMix.
class MicMixTap extends AudioWorkletProcessor {
  constructor() {
    super();
    this.chunk = new Float32Array(960 * 2); // 20 мс при 48 кГц, L/R вперемешку
    this.pos = 0;
  }

  process(inputs) {
    const input = inputs[0];
    if (!input || input.length === 0) return true;
    const left = input[0];
    const right = input[1] ?? input[0]; // моно-вкладка → в оба канала
    for (let i = 0; i < left.length; i++) {
      this.chunk[this.pos++] = left[i];
      this.chunk[this.pos++] = right[i];
      if (this.pos === this.chunk.length) {
        this.port.postMessage(this.chunk.buffer, [this.chunk.buffer]);
        this.chunk = new Float32Array(960 * 2);
        this.pos = 0;
      }
    }
    return true;
  }
}

registerProcessor("micmix-tap", MicMixTap);
