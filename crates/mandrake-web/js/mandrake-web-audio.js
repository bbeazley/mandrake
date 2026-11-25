// WebAudio output for mandrake-web.
//
// The game (mandrake-client's wasm build) owns the mixer (mandrake_audio::SharedAudio) and renders interleaved stereo
// f32 frames with `mix_audio`. This module pumps those frames into an `AudioContext`:
//   * AudioWorklet (preferred): the worklet only plays a ring of frames the main thread posts.
//   * ScriptProcessorNode (fallback): the callback calls `mix_audio` directly.
// Browsers keep an AudioContext suspended until a user gesture, so `M2WebAudio.attach` hooks the
// first pointer/key/touch event and resumes the context. No audio is generated here; every
// sample comes from the game's mixer.
//
// Usage (scripts/web/client.js):
//   const audio = await M2WebAudio.attach(game, { workletUrl: "./mandrake-audio-worklet.js" });

const BLOCK_FRAMES = 128;
const TARGET_AHEAD_FRAMES = 2048; // ~46 ms at 44.1 kHz of audio queued in the worklet

export class M2WebAudio {
  constructor(client, context, mode) {
    this.client = client;
    this.context = context;
    this.mode = mode; // "worklet" | "script-processor"
    this.node = null;
    this.timer = null;
    this.queuedFrames = 0;
    this.removeUnlock = null;
    this.unlocked = context.state === "running";
  }

  /** Create the context, route the mixer into it and install the user-gesture unlock. */
  static async attach(client, options = {}) {
    const Ctx = window.AudioContext || window.webkitAudioContext;
    if (!Ctx) throw new Error("WebAudio is not supported in this browser");
    const context = new Ctx({ latencyHint: "interactive" });
    client.set_audio_sample_rate(context.sampleRate);

    let audio;
    if (context.audioWorklet && options.workletUrl) {
      try {
        await context.audioWorklet.addModule(options.workletUrl);
        audio = new M2WebAudio(client, context, "worklet");
        audio.startWorklet();
      } catch (e) {
        console.warn("AudioWorklet unavailable, using ScriptProcessor:", e);
      }
    }
    if (!audio) {
      audio = new M2WebAudio(client, context, "script-processor");
      audio.startScriptProcessor();
    }
    audio.installUnlock(options.unlockTarget || window);
    return audio;
  }

  startWorklet() {
    const node = new AudioWorkletNode(this.context, "m2-mixer-output", {
      numberOfInputs: 0,
      numberOfOutputs: 1,
      outputChannelCount: [2],
    });
    node.port.onmessage = (ev) => {
      // The worklet reports how many frames it has consumed.
      this.queuedFrames = Math.max(0, this.queuedFrames - ev.data.consumed);
    };
    node.connect(this.context.destination);
    this.node = node;
    // Top the worklet's ring up from the main thread (where the WASM mixer lives).
    this.timer = setInterval(() => this.pump(), 10);
  }

  pump() {
    if (this.context.state !== "running") return;
    while (this.queuedFrames < TARGET_AHEAD_FRAMES) {
      const block = new Float32Array(BLOCK_FRAMES * 2);
      this.client.mix_audio(block);
      this.node.port.postMessage(block, [block.buffer]);
      this.queuedFrames += BLOCK_FRAMES;
    }
  }

  startScriptProcessor() {
    const node = this.context.createScriptProcessor(1024, 0, 2);
    let mix = new Float32Array(1024 * 2);
    node.onaudioprocess = (ev) => {
      const out = ev.outputBuffer;
      const frames = out.length;
      if (mix.length < frames * 2) mix = new Float32Array(frames * 2);
      const view = mix.subarray(0, frames * 2);
      this.client.mix_audio(view);
      const l = out.getChannelData(0);
      const r = out.getChannelData(1);
      for (let i = 0; i < frames; i++) {
        l[i] = view[2 * i];
        r[i] = view[2 * i + 1];
      }
    };
    node.connect(this.context.destination);
    this.node = node;
  }

  /** Resume the (autoplay-suspended) context on the first user gesture. */
  installUnlock(target) {
    const events = ["pointerdown", "keydown", "touchend", "click"];
    const remove = () => {
      for (const ev of events) target.removeEventListener(ev, unlock, true);
      this.removeUnlock = null;
    };
    const unlock = async () => {
      try {
        await this.context.resume();
      } catch (e) {
        console.warn("AudioContext resume failed:", e);
        return;
      }
      if (this.context.state === "running") {
        this.unlocked = true;
        remove();
      }
    };
    this.removeUnlock = remove;
    for (const ev of events) target.addEventListener(ev, unlock, true);
    if (this.unlocked) unlock();
  }

  close() {
    if (this.removeUnlock) this.removeUnlock();
    if (this.timer) clearInterval(this.timer);
    if (this.node) this.node.disconnect();
    return this.context.close();
  }
}
