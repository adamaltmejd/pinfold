import { createHash } from "node:crypto";
import { spawn } from "node:child_process";
import { mkdir, readFile, realpath, rename, stat, writeFile } from "node:fs/promises";
import { delimiter, isAbsolute, relative, resolve, sep } from "node:path";
import { homedir } from "node:os";
import { createInterface } from "node:readline";
import { DatabaseSync } from "node:sqlite";
import { parseArgs } from "node:util";
import { BACKGROUND_CONTEXT } from "@earendil-works/chord/context";
import { Type } from "@earendil-works/pi-ai";
import { createModels, createProvider } from "@earendil-works/pi-ai/models";
import { openAICompletionsApi } from "@earendil-works/pi-ai/api/openai-completions.lazy";
import { AssistantEntry, createRegistry, defineExtension, defineTool, Harness } from "@earendil-works/pi-durable";
import { openNodeSqliteStorage } from "@earendil-works/pi-durable/storage/sqlite/node";

const context = BACKGROUND_CONTEXT;
const event = (name: string, fields: object = {}) => console.log(JSON.stringify({ event: name, ...fields }));

type Job = {
  version: 1;
  id: string;
  pinfold: string;
  project: string;
  image: string;
  baseUrl: string;
  model: string;
  prompt: string;
};

function within(parent: string, child: string): boolean {
  const path = relative(parent, child);
  return path === "" || (!path.startsWith(`..${sep}`) && path !== ".." && !path.startsWith(sep));
}

async function canonicalDestination(path: string): Promise<string> {
  try { return await realpath(path); }
  catch (error) {
    if ((error as NodeJS.ErrnoException).code !== "ENOENT") throw error;
    const parent = resolve(path, "..");
    if (parent === path) throw error;
    return resolve(await canonicalDestination(parent), relative(parent, path));
  }
}

async function hostBoundary(project: string, state: string, binary: string) {
  const roots: Array<[string, string]> = [
    ["checkpoint", state], ["Pinfold executable", binary], ["Node executable", process.execPath],
    ["controller", import.meta.dirname], ["controller entrypoint", process.argv[1]!],
    ["dependencies", resolve(import.meta.dirname, "node_modules")],
    ["Pinfold ownership", resolve(await realpath("/tmp"), `pinfold-${process.getuid!()}`)],
  ];
  if (isAbsolute(process.argv0)) roots.push(["Node launch alias", process.argv0]);
  const home = homedir();
  if (!isAbsolute(home)) throw new Error("host HOME must be absolute");
  for (const [variable, fallback] of [["XDG_STATE_HOME", ".local/state"], ["XDG_CONFIG_HOME", ".config"], ["XDG_CACHE_HOME", ".cache"]]) {
    const configured = process.env[variable!];
    roots.push([variable!, resolve(configured && isAbsolute(configured) ? configured : resolve(home, fallback!), "pinfold")]);
  }
  if (process.platform === "linux") roots.push(["runtime authority", process.env.XDG_RUNTIME_DIR ?? `/run/user/${process.getuid!()}`]);
  // Pinfold resolves its runtime and host helpers on PATH. Guest-writable
  // PATH entries would let boxed edits select a host executable later.
  for (const path of (process.env.PATH ?? "").split(delimiter)) roots.push(["host PATH", resolve(path || ".")]);
  for (const [name, path] of roots) {
    const lexical = resolve(path);
    const canonical = await canonicalDestination(lexical);
    // A symlink located in the project can be replaced by the guest even
    // when its current target is outside the project.
    if ([lexical, canonical].some((root) => within(project, root) || within(root, project))) throw new Error(`host-boundary: writable project overlaps ${name}`);
  }
}

async function cli(binary: string, args: string[], timeout = 30_000): Promise<string> {
  const child = spawn(binary, args, { stdio: ["ignore", "pipe", "pipe"] });
  const chunks: Buffer[] = [];
  let bytes = 0;
  let overflow = false;
  for (const stream of [child.stdout, child.stderr]) stream.on("data", (chunk: Buffer) => {
    bytes += chunk.length;
    if (bytes <= 1024 * 1024) { if (stream === child.stdout) chunks.push(chunk); }
    else { overflow = true; child.kill("SIGKILL"); }
  });
  const timer = setTimeout(() => child.kill("SIGKILL"), timeout);
  try {
    await new Promise<void>((done, fail) => {
      child.once("error", fail);
      child.once("close", (code, signal) => code === 0 && !overflow ? done() : fail(new Error(`pinfold ${args.slice(0, 2).join(" ")} failed (${signal ?? code})`)));
    });
    return Buffer.concat(chunks).toString("utf8");
  } finally { clearTimeout(timer); }
}

class Box {
  readonly name: string;
  #owner?: ReturnType<typeof spawn>;
  #closed?: Promise<void>;
  #stop?: Promise<void>;
  #cancelled = false;
  #authorized = false;
  readonly job: Job;

  constructor(job: Job) { this.job = job; this.name = `durable-${job.id}`; }

  async listed() {
    const text = await cli(this.job.pinfold, ["box", "list", "--label", "dev.pinfold.owner"]);
    return text.trim().split("\n").filter(Boolean).map((line) => JSON.parse(line)).find((box) => box.name === this.name);
  }

  async reclaim() {
    const old = await this.listed();
    if (old && old.labels?.["dev.example.pi-durable.job"] !== this.job.id) throw new Error("box name belongs to another caller");
    this.#authorized = true;
    await cli(this.job.pinfold, ["box", "down", this.name]);
    if (await this.listed()) throw new Error("old box remains; refusing to resume");
    event("old-work-removed", { box: this.name });
  }

  async start() {
    const mounts = [{ host: this.job.project, guest: "/workspace", readonly: false }];
    // Existing editor/git directories are protected. This prototype requires
    // absent protected directories to be prepared explicitly by the caller.
    for (const name of [".git", ".vscode", ".claude", ".idea"]) {
      const path = resolve(this.job.project, name);
      const real = await realpath(path);
      if (real !== path || !(await stat(real)).isDirectory()) throw new Error(`${name} must be a real prepared directory`);
      mounts.push({ host: real, guest: `/workspace/${name}`, readonly: true });
    }
    if (this.#cancelled) throw new Error("cancelled before box startup");
    const owner = spawn(this.job.pinfold, ["box", "up"], { stdio: ["pipe", "pipe", "inherit"] });
    this.#owner = owner;
    this.#closed = new Promise((done, fail) => { owner.once("error", fail); owner.once("close", () => done()); });
    // Store a handler immediately so startup failure cannot leave an unhandled rejection.
    void this.#closed.catch(() => {});
    const lines = createInterface({ input: owner.stdout! });
    let timer: ReturnType<typeof setTimeout>;
    owner.stdin!.on("error", () => {});
    owner.stdin!.write(`${JSON.stringify({ name: this.name, image: this.job.image, labels: { "dev.example.pi-durable.job": this.job.id }, mounts, env: { HOME: "/tmp" }, cpus: 2, memory: "1G" })}\n`);
    try {
      await Promise.race([
        (async () => {
          for await (const line of lines) {
            const message = JSON.parse(line);
            if (message.event === "ready") return;
            if (["refused", "failed", "down"].includes(message.event)) throw new Error(`box startup ${message.event}: ${message.reason ?? "failure"}`);
          }
          throw new Error("box owner exited before ready");
        })(),
        new Promise<never>((_, fail) => { timer = setTimeout(() => fail(new Error("box readiness timeout")), 60_000); }),
      ]);
      event("box-ready", { box: this.name });
    } finally { clearTimeout(timer!); lines.close(); }
  }

  stop(): Promise<void> {
    this.#cancelled = true;
    if (!this.#authorized) return Promise.resolve();
    return this.#stop ??= (async () => {
      // Signal our owner even before it claims the name, so startup workers
      // are cancelled through Pinfold's lifecycle rather than left running.
      this.#owner?.kill("SIGTERM");
      await cli(this.job.pinfold, ["box", "down", this.name], 60_000);
      this.#owner?.stdin?.end();
      if (this.#closed) {
        let timer: ReturnType<typeof setTimeout>;
        try {
          await Promise.race([this.#closed, new Promise<never>((_, fail) => { timer = setTimeout(() => { this.#owner?.kill("SIGKILL"); fail(new Error("box owner shutdown timeout")); }, 30_000); })]);
        } finally { clearTimeout(timer!); }
      }
      if (await this.listed()) throw new Error("box removal failed");
      event("box-stopped", { box: this.name });
    })();
  }

  async bash(command: string, signal: AbortSignal, output: (chunk: string | Uint8Array) => void): Promise<number> {
    if (this.#cancelled || signal.aborted) throw new Error("whole box was cancelled");
    const child = spawn(this.job.pinfold, ["box", "exec", this.name, "--workdir", "/workspace", "--", "/bin/sh", "-c", command], { stdio: ["ignore", "pipe", "pipe"] });
    for (const stream of [child.stdout, child.stderr]) stream.on("data", (chunk: Buffer) => output(chunk));
    const abort = () => { void this.stop().catch(() => child.kill("SIGKILL")); };
    signal.addEventListener("abort", abort, { once: true });
    if (signal.aborted) abort();
    try {
      return await new Promise<number>((done, fail) => { child.once("error", fail); child.once("close", (code) => done(code ?? 1)); });
    } finally { signal.removeEventListener("abort", abort); }
  }
}

async function main() {
  const { values, positionals } = parseArgs({ allowPositionals: true, options: {
    state: { type: "string" }, pinfold: { type: "string" }, project: { type: "string" }, image: { type: "string" },
    "base-url": { type: "string" }, model: { type: "string" }, prompt: { type: "string" },
  } });
  const mode = positionals[0];
  if (!values.state || positionals.length !== 1 || !["start", "resume"].includes(mode!)) throw new Error("usage: prototype.ts start|resume --state PATH [--pinfold PATH --project PATH --image REF --base-url URL --model ID --prompt TEXT]");
  const state = await canonicalDestination(resolve(values.state));
  let job: Job;
  if (mode === "resume") {
    if (Object.keys(values).some((key) => key !== "state")) throw new Error("resume accepts only --state; the job configuration is immutable");
    job = JSON.parse(await readFile(resolve(state, "job.json"), "utf8"));
    if (job.version !== 1) throw new Error("unsupported job format");
  } else {
    for (const key of ["pinfold", "project", "image", "base-url", "model", "prompt"] as const) if (!values[key]) throw new Error(`--${key} is required`);
    const url = new URL(values["base-url"]!);
    if (!["http:", "https:"].includes(url.protocol) || url.username || url.password || url.search || url.hash) throw new Error("base URL must be an HTTP(S) endpoint without credentials, query or fragment");
    job = { version: 1, id: createHash("sha256").update(state).digest("hex").slice(0, 24), pinfold: await realpath(values.pinfold!), project: await realpath(values.project!), image: values.image!, baseUrl: url.toString(), model: values.model!, prompt: values.prompt! };
  }
  await hostBoundary(job.project, state, job.pinfold);
  if (await realpath(job.project) !== job.project || await realpath(job.pinfold) !== job.pinfold) throw new Error("job paths changed; refusing recovery");
  if (createHash("sha256").update(state).digest("hex").slice(0, 24) !== job.id) throw new Error("checkpoint namespace moved; refusing recovery");
  await mkdir(state, { recursive: true, mode: 0o700 });
  const guard = new DatabaseSync(resolve(state, "writer.sqlite"));
  try { guard.exec("PRAGMA busy_timeout = 0; BEGIN EXCLUSIVE"); }
  catch { guard.close(); throw new Error("checkpoint-in-use: another writer owns this checkpoint; no box was stopped"); }
  const box = new Box(job);
  let harness: Harness | undefined;
  let root: Awaited<ReturnType<Harness["root"]>> | undefined;
  let cancelling = false;
  const cancel = () => {
    cancelling = true;
    void Promise.all([box.stop(), root?.abort(context)]).catch(() => { process.exitCode = 1; });
  };
  process.on("SIGTERM", cancel);
  process.on("SIGINT", cancel);
  try {
    if (mode === "start") {
      try { await readFile(resolve(state, "job.json")); throw new Error("job already exists; use resume"); }
      catch (error) { if ((error as NodeJS.ErrnoException).code !== "ENOENT") throw error; }
      const temporary = resolve(state, "job.json.tmp");
      await writeFile(temporary, `${JSON.stringify(job)}\n`, { mode: 0o600 });
      await rename(temporary, resolve(state, "job.json"));
    }
    await box.reclaim();
    if (cancelling) throw new Error("cancelled before startup");
    await box.start();
    if (cancelling) throw new Error("cancelled before resume");
    const registry = createRegistry();
    registry.install(defineExtension({ name: "pinfold-box", tools: [defineTool({
      name: "boxed_bash", description: "Run a shell command inside the isolated project box. Read, write and edit project files there. Commands may partially execute; interruption is never automatically replayed. Cancellation stops the whole box.",
      parameters: Type.Object({ command: Type.String() }), replay: "unsafe", executionMode: "sequential", outputLimits: { maxBytes: 32 * 1024, maxLines: 400 },
      execute: async ({ command }, api, ctx) => {
        event("tool-started", { call: api.callId });
        const code = await box.bash(command, ctx.abortSignal ?? new AbortController().signal, (chunk) => api.output(chunk));
        api.output(`\nExit code: ${code}\n`);
        return { isError: code !== 0 };
      },
    })] }));
    const models = createModels();
    models.setProvider(createProvider({ id: "endpoint", baseUrl: job.baseUrl, auth: { apiKey: { name: "Endpoint API key", resolve: async () => ({ auth: { apiKey: process.env.PINFOLD_DURABLE_API_KEY ?? "local-fixture" }, source: "PINFOLD_DURABLE_API_KEY" }) } }, models: [{ id: job.model, name: job.model, api: "openai-completions", provider: "endpoint", baseUrl: job.baseUrl, reasoning: false, input: ["text"], cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 }, contextWindow: 128_000, maxTokens: 4096 }], api: openAICompletionsApi() }));
    harness = await Harness.open(await openNodeSqliteStorage(resolve(state, "checkpoint.sqlite")), { models, registry, settings: { toolExecution: "sequential", retry: { enabled: false }, stream: { timeoutMs: 30_000, maxRetries: 0 }, compaction: { enabled: false } } }, context);
    root = await harness.root(context, { agent: { model: { provider: "endpoint", modelId: job.model }, instructions: "All filesystem and process operations use boxed_bash. The checkpoint is host-only. Do not automatically repeat an interrupted command: it may already have changed the project. Report uncertainty to the caller." } });
    event("resuming", { box: box.name });
    harness.resume();
    const submission = await root.submit({ type: "input", content: job.prompt, requestId: job.id }, context);
    const settled = await submission.wait(context);
    if (settled.status === "done" && settled.type === "input") {
      const answer = await root.commit((tx) => tx.entry(AssistantEntry, settled.answer), context);
      event("completed", { answer: answer?.model });
    } else { event("unanswered", { status: settled.status }); process.exitCode = 1; }
    if (cancelling) process.exitCode = 130;
  } finally {
    try { await box.stop(); }
    finally {
      try { await harness?.close(context); }
      finally {
        process.off("SIGTERM", cancel);
        process.off("SIGINT", cancel);
        try { guard.exec("ROLLBACK"); }
        finally { guard.close(); }
      }
    }
  }
}

main().catch((error) => { console.error(error instanceof Error ? error.message : "prototype failed"); process.exitCode = 1; });
