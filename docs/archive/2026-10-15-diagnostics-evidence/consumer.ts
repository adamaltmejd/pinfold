// Disposable host-only research adapter. No policy interpretation or replay.
// Captured bytes may contain command secrets: callers choose what to persist.
import { spawn, type ChildProcessWithoutNullStreams } from "node:child_process";

type Event = Record<string, unknown> & { event?: string };
type End = { code: number | null; signal: string | null; spawnError?: string };
type OwnerEnd = End & { outputDrain: "complete" | "deadline" };
class Tail {
  bytes = Buffer.alloc(0);
  truncated = false;
  push(chunk: Buffer) {
    const next = Buffer.concat([this.bytes, chunk]);
    this.truncated ||= next.length > 65536;
    this.bytes = next.subarray(-65536);
  }
  value() { return { text: this.bytes.toString("utf8"), truncated: this.truncated }; }
}
function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => { resolve = done; });
  return { promise, resolve };
}
async function within<T>(promise: Promise<T>, ms: number, label: string): Promise<T> {
  let timer: ReturnType<typeof setTimeout>;
  try {
    return await Promise.race([promise, new Promise<never>((_, fail) => {
      timer = setTimeout(() => fail(new Error(label)), ms);
    })]);
  } finally { clearTimeout(timer!); }
}
function ended(child: ReturnType<typeof spawn>): Promise<End> {
  return new Promise((done) => {
    let spawnError: string | undefined;
    child.once("error", (error) => { spawnError = error.message; });
    child.once("close", (code, signal) => done({ code, signal, ...(spawnError ? { spawnError } : {}) }));
  });
}
function ownerEnded(child: ChildProcessWithoutNullStreams) {
  const exit = deferred<End>(), drain = deferred<OwnerEnd>();
  let result: End | undefined, timer: ReturnType<typeof setTimeout> | undefined;
  let complete = false;
  const finish = (outputDrain: OwnerEnd["outputDrain"]) => {
    if (complete || !result) return;
    complete = true; clearTimeout(timer);
    drain.resolve({ ...result, outputDrain });
    if (outputDrain === "deadline") {
      // Descendants can retain inherited pipes after the owner died. Close
      // only our readers; never infer teardown or kill an observed process.
      child.stdout.destroy(); child.stderr.destroy(); child.stdin.destroy();
    }
  };
  const exited = (status: End) => {
    if (result) return;
    result = status; exit.resolve(status);
    timer = setTimeout(() => finish("deadline"), 1000);
  };
  child.once("exit", (code, signal) => exited({ code, signal }));
  child.once("error", (error) => {
    if (child.pid === undefined) exited({ code: null, signal: null, spawnError: error.message });
  });
  child.once("close", (code, signal) => { exited({ code, signal }); finish("complete"); });
  return { exited: exit.promise, closed: drain.promise };
}

export class BoxOwner {
  readonly events: Event[] = [];
  readonly stderr = new Tail();
  readonly protocolErrors: string[] = [];
  eventsTruncated = false;
  readonly child: ChildProcessWithoutNullStreams;
  readonly exited: Promise<End>;
  readonly closed: Promise<OwnerEnd>;
  ownerEnd?: OwnerEnd;
  readonly first = deferred<Event>();
  #stop?: Promise<End>;
  #pending = Buffer.alloc(0);
  #discardLine = false;
  #firstSeen = false;
  constructor(readonly binary: string, readonly name: string, spec: object) {
    this.child = spawn(binary, ["box", "up"], { stdio: ["pipe", "pipe", "pipe"] });
    const ending = ownerEnded(this.child);
    this.exited = ending.exited;
    this.closed = ending.closed;
    this.child.stderr.on("data", (chunk: Buffer) => this.stderr.push(chunk));
    this.child.stdout.on("data", (chunk: Buffer) => this.#lines(chunk));
    this.child.stdin.on("error", () => {});
    this.child.stdin.write(`${JSON.stringify({ ...spec, name })}\n`);
    void this.closed.then((result) => {
      this.ownerEnd = result;
      if (this.#pending.length) this.#error("unterminated lifecycle line");
      if (!this.#firstSeen) this.first.resolve({ event: "owner-closed-before-event", ...result });
    });
  }
  #error(reason: string) { if (this.protocolErrors.length < 8) this.protocolErrors.push(reason); }
  #lines(chunk: Buffer) {
    // Byte-based cap includes unterminated lines. Continue draining after overflow.
    for (const part of chunk.toString("binary").split(/(?<=\n)/)) {
      const bytes = Buffer.from(part, "binary");
      if (!this.#discardLine) {
        if (this.#pending.length + bytes.length > 65536) {
          this.#error("lifecycle line exceeded 64 KiB"); this.#discardLine = true; this.#pending = Buffer.alloc(0);
        } else this.#pending = Buffer.concat([this.#pending, bytes]);
      }
      if (bytes.at(-1) !== 10) continue;
      if (!this.#discardLine) {
        try {
          const event = JSON.parse(this.#pending.toString("utf8")) as Event;
          if (!event || typeof event !== "object") throw new Error();
          if (this.events.length === 64) { this.events.shift(); this.eventsTruncated = true; }
          this.events.push(event);
          if (!this.#firstSeen) { this.#firstSeen = true; this.first.resolve(event); }
        } catch { this.#error("invalid lifecycle JSON"); }
      }
      this.#pending = Buffer.alloc(0); this.#discardLine = false;
    }
  }
  async ready(ms = 60000) {
    try {
      const event = await within(this.first.promise, ms, "readiness deadline");
      if (event.event !== "ready") throw Object.assign(new Error("startup did not become ready"), { lifecycle: event });
      return event;
    } catch (error) {
      let cleanup: unknown;
      try { cleanup = await this.stop(); } catch (failure) { cleanup = String(failure); }
      throw Object.assign(error as Error, { evidence: this.snapshot(), cleanup });
    }
  }
  snapshot() {
    return { events: [...this.events], eventsTruncated: this.eventsTruncated,
      stderr: this.stderr.value(), protocolErrors: [...this.protocolErrors], ownerEnd: this.ownerEnd };
  }
  stop(ms = 30000): Promise<End> {
    // Only this explicit owner operation ends the lifetime. Observer cancellation does not.
    return this.#stop ??= (async () => {
      this.child.kill("SIGTERM");
      this.child.stdin.end();
      // No hard-kill: a deadline reports uncertain cleanup, never successful teardown.
      return within(this.closed, ms, "owner teardown unconfirmed; inspect this owned box");
    })();
  }
  async exec(argv: string[], sink?: (stream: "stdout" | "stderr", bytes: Buffer) => void) {
    const child = spawn(this.binary, ["box", "exec", this.name, "--", ...argv], { stdio: ["ignore", "pipe", "pipe"] });
    const stdout = new Tail(), stderr = new Tail();
    child.stdout.on("data", (bytes: Buffer) => { stdout.push(bytes); sink?.("stdout", bytes); });
    child.stderr.on("data", (bytes: Buffer) => { stderr.push(bytes); sink?.("stderr", bytes); });
    const result = await ended(child);
    // Exit 3 is intentionally not classified: guest commands can also return it.
    return { ...result, stdout: stdout.value(), stderr: stderr.value() };
  }
}

// Current box-list has binary-owned cancellation/reaping. Do not generalize
// this to killing exec wrappers: that is not a guest-command cancellation API.
export async function observeList(binary: string, signal: AbortSignal, ms = 25000) {
  const child = spawn(binary, ["box", "list", "--label", "dev.pinfold.owner"], { stdio: ["ignore", "pipe", "pipe"] });
  const stdout = new Tail(), stderr = new Tail();
  child.stdout.on("data", (chunk: Buffer) => stdout.push(chunk));
  child.stderr.on("data", (chunk: Buffer) => stderr.push(chunk));
  const close = ended(child);
  const cancel = () => { child.kill("SIGTERM"); };
  const timer = setTimeout(cancel, ms);
  signal.addEventListener("abort", cancel, { once: true });
  if (signal.aborted) cancel();
  try {
    const result = await within(close, ms + 5000, "observer shutdown unconfirmed");
    return { ...result, stdout: stdout.value(), stderr: stderr.value() };
  } finally { clearTimeout(timer); signal.removeEventListener("abort", cancel); }
}
