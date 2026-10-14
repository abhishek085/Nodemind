// Voice capture: Web Audio → 16 kHz mono s16le → base64 chunks → `append_audio`.
//
// The AudioWorklet does the resampling off the main thread; the main thread
// only base64-encodes finished chunks and ships them to Rust, where they are
// buffered until `finish_capture` runs the pipeline.

/** AudioWorklet source — injected as a blob module (kept as a string so the
 *  bundler never tries to resolve it). */
const WORKLET_SOURCE = `
class PCM16Processor extends AudioWorkletProcessor {
  constructor() {
    super();
    this._ratio = sampleRate / 16000;
    this._acc = 0;
    this._frac = 0;
  }
  process(inputs) {
    const ch = inputs[0] && inputs[0][0];
    if (!ch || ch.length === 0) return true;
    const out = new Int16Array(Math.ceil(ch.length / this._ratio) + 8);
    let oi = 0;
    for (let i = 0; i < ch.length; i++) {
      this._acc += ch[i];
      this._frac += 1;
      if (this._frac >= this._ratio) {
        const v = Math.max(-1, Math.min(1, this._acc / this._ratio));
        out[oi++] = Math.round(v * 32767);
        this._acc = 0;
        this._frac -= this._ratio;
      }
    }
    const buf = new ArrayBuffer(oi * 2);
    new Int16Array(buf).set(out.subarray(0, oi));
    this.port.postMessage(buf, [buf]);
    return true;
  }
}
registerProcessor("pcm16", PCM16Processor);
`;

export interface RecorderCallbacks {
  /** Called for each resampled PCM chunk, base64-encoded. */
  onChunk: (b64: string) => void;
  /** Called once when the user (or the time cap) ends the session. */
  onEnded: (elapsedSecs: number) => void;
  /** Mic permission / capture failure. */
  onError: (message: string) => void;
}

export class Recorder {
  private ctx: AudioContext | null = null;
  private stream: MediaStream | null = null;
  private node: AudioWorkletNode | null = null;
  private timer: number | null = null;
  private startedAt = 0;
  private maxSeconds: number;
  private cb: RecorderCallbacks;

  constructor(maxSeconds: number, cb: RecorderCallbacks) {
    this.maxSeconds = maxSeconds;
    this.cb = cb;
  }

  get running(): boolean {
    return this.node !== null;
  }

  async start(): Promise<void> {
    try {
      this.stream = await navigator.mediaDevices.getUserMedia({
        audio: {
          channelCount: 1,
          echoCancellation: true,
          noiseSuppression: true,
        },
      });
    } catch (e) {
      this.cb.onError(
        e instanceof DOMException && e.name === "NotAllowedError"
          ? "Microphone access denied. Allow Nodemind in System Settings → Privacy → Microphone."
          : `Could not open the microphone: ${String(e)}`,
      );
      return;
    }

    try {
      this.ctx = new AudioContext();
      await this.ctx.audioWorklet.addModule(URL.createObjectURL(new Blob([WORKLET_SOURCE], { type: "application/javascript" })));
      const source = this.ctx.createMediaStreamSource(this.stream);
      this.node = new AudioWorkletNode(this.ctx, "pcm16");
      this.node.port.onmessage = (e: MessageEvent<ArrayBuffer>) => {
        const b64 = bufferToBase64(e.data);
        if (b64.length > 0) this.cb.onChunk(b64);
      };
      source.connect(this.node);
      // Route to a zero-gain sink: keeps the worklet scheduled, no monitor loop.
      const sink = this.ctx.createGain();
      sink.gain.value = 0;
      this.node.connect(sink);
      sink.connect(this.ctx.destination);
      this.startedAt = Date.now();
      this.timer = window.setInterval(() => this.tick(), 1000);
    } catch (e) {
      this.teardown();
      this.cb.onError(`Audio engine failed: ${String(e)}`);
    }
  }

  private tick(): void {
    const elapsed = (Date.now() - this.startedAt) / 1000;
    if (elapsed >= this.maxSeconds) {
      this.stop();
    }
  }

  /** Stop capture and report elapsed seconds. */
  stop(): void {
    if (!this.running) return;
    const elapsed = Math.max(1, Math.round((Date.now() - this.startedAt) / 1000));
    this.teardown();
    this.cb.onEnded(elapsed);
  }

  private teardown(): void {
    if (this.timer !== null) {
      window.clearInterval(this.timer);
      this.timer = null;
    }
    try {
      this.node?.disconnect();
    } catch {
      // already disconnected
    }
    this.node = null;
    for (const t of this.stream?.getTracks() ?? []) t.stop();
    this.stream = null;
    this.ctx?.close().catch(() => undefined);
    this.ctx = null;
  }
}

/** base64-encode a PCM buffer without blowing the call stack. */
function bufferToBase64(buf: ArrayBuffer): string {
  const bytes = new Uint8Array(buf);
  let s = "";
  const CHUNK = 0x8000;
  for (let i = 0; i < bytes.length; i += CHUNK) {
    s += String.fromCharCode(...bytes.subarray(i, Math.min(i + CHUNK, bytes.length)));
  }
  return btoa(s);
}
