// AudioWorkletProcessor for mandrake-web. Plays interleaved stereo f32 blocks that
// the main thread renders with the WASM mixer (`mix_audio`) and posts here. It
// generates no audio itself: with an empty ring it outputs silence.
class M2MixerOutput extends AudioWorkletProcessor {
  constructor() {
    super();
    this.blocks = [];
    this.offset = 0; // frames already read from blocks[0]
    this.port.onmessage = (ev) => this.blocks.push(ev.data);
  }

  process(_inputs, outputs) {
    const out = outputs[0];
    const left = out[0];
    const right = out[1] || out[0];
    let consumed = 0;
    for (let i = 0; i < left.length; i++) {
      const block = this.blocks[0];
      if (!block) {
        left[i] = 0;
        right[i] = 0;
        continue;
      }
      left[i] = block[2 * this.offset];
      right[i] = block[2 * this.offset + 1];
      this.offset++;
      consumed++;
      if (this.offset * 2 >= block.length) {
        this.blocks.shift();
        this.offset = 0;
      }
    }
    if (consumed > 0) this.port.postMessage({ consumed });
    return true;
  }
}

registerProcessor("m2-mixer-output", M2MixerOutput);
