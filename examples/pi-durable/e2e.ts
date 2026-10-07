import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { watch } from "node:fs";
import { mkdir, mkdtemp, readFile, rm, access, copyFile, realpath, symlink, utimes } from "node:fs/promises";
import { createServer } from "node:http";
import { tmpdir } from "node:os";
import { delimiter, resolve } from "node:path";
import { createInterface } from "node:readline";
import { parseArgs } from "node:util";

const { values } = parseArgs({ options: { pinfold: { type: "string" }, image: { type: "string" } } });
assert(values.pinfold && values.image, "usage: node e2e.ts --pinfold ABSOLUTE_BINARY --image EXISTING_IMAGE");
const binary = resolve(values.pinfold);
const scratch = await mkdtemp(resolve(await realpath(tmpdir()), "pf-durable-"));
const env: NodeJS.ProcessEnv = { ...process.env, PINFOLD_DURABLE_API_KEY: "fixture-only", XDG_STATE_HOME: resolve(scratch, "pinfold-state"), XDG_CACHE_HOME: resolve(scratch, "pinfold-cache"), XDG_CONFIG_HOME: resolve(scratch, "pinfold-config") };
const children = new Set<ReturnType<typeof spawn>>();
const requests: Array<{ kind: string; messages: Array<{ role: string; content: unknown }> }> = [];
const generations = new Map<string, string>();
const ownedJobs = new Set<string>();
let fixtureFailure: unknown;

async function run(program: string, args: string[], childEnv = env) {
  const child = spawn(program, args, { env: childEnv, stdio: ["ignore", "pipe", "pipe"] });
  children.add(child);
  let stdout = "", stderr = "";
  child.stdout.setEncoding("utf8").on("data", (chunk) => { stdout += chunk; });
  child.stderr.setEncoding("utf8").on("data", (chunk) => { stderr += chunk; });
  const timer = setTimeout(() => child.kill("SIGKILL"), 90_000);
  try {
    const code = await new Promise<number | null>((done, fail) => { child.once("error", fail); child.once("close", done); });
    return { code, stdout, stderr };
  } finally { clearTimeout(timer); children.delete(child); }
}

async function boxes() {
  const result = await run(binary, ["box", "list", "--label", "dev.example.pi-durable.job"]);
  assert.equal(result.code, 0, result.stderr);
  return result.stdout.trim().split("\n").filter(Boolean).map((line) => JSON.parse(line)).filter((box) => ownedJobs.has(box.labels?.["dev.example.pi-durable.job"]));
}

async function guest(box: string, command: string) {
  const result = await run(binary, ["box", "exec", box, "--workdir", "/workspace", "--", "sh", "-c", command]);
  assert.equal(result.code, 0, `${result.stderr}\n${result.stdout}`);
  return result.stdout;
}

function waitForFile(project: string, name: string) {
  return new Promise<void>((done, fail) => {
    const watcher = watch(project, (_event, file) => { if (file?.toString() === name) void check(); });
    const timer = setTimeout(() => { watcher.close(); fail(new Error(`fixture marker ${name} timed out`)); }, 90_000);
    const check = async () => {
      try { await access(resolve(project, name)); clearTimeout(timer); watcher.close(); done(); }
      catch (error) { if ((error as NodeJS.ErrnoException).code !== "ENOENT") { clearTimeout(timer); watcher.close(); fail(error); } }
    };
    void check();
  });
}

const server = createServer(async (request, response) => {
  try {
    assert.equal(request.url, "/v1/chat/completions");
    let raw = "";
    for await (const chunk of request) raw += chunk.toString();
    const input = JSON.parse(raw);
    const prompt = input.messages.find((message: { role: string }) => message.role === "user")?.content;
    assert(["control", "crash", "cancel"].includes(prompt));
    const kind = prompt as string;
    requests.push({ kind, messages: input.messages });
    const tool = input.messages.findLast((message: { role: string }) => message.role === "tool");
    let delta: object, finish: string;
    if (!tool) {
      const command = kind === "control"
        ? "printf control-ok > control; cat control"
        : `printf 'once\\n' >> invocations; env PINFOLD_DURABLE_CHILD=${kind}-child sh -c 'printf ready > child-ready.tmp; mv child-ready.tmp child-ready; exec tail -f /dev/null' & wait`;
      delta = { role: "assistant", tool_calls: [{ index: 0, id: `call-${kind}`, type: "function", function: { name: "boxed_bash", arguments: JSON.stringify({ command }) } }] };
      finish = "tool_calls";
    } else {
      if (kind === "control") assert.match(JSON.stringify(tool.content), /control-ok/);
      if (kind === "crash") {
        assert.match(JSON.stringify(tool.content), /interrupted/);
        const listed = await boxes();
        assert.equal(listed.length, 1, "exactly the restarted fixture box exists");
        const box = listed[0];
        assert.notEqual(box.labels["dev.pinfold.generation"], generations.get(kind), "recovery must create a new box generation before the model resumes");
        const clean = await guest(box.name, "for p in /proc/[0-9]*/environ; do [ -r \"$p\" ] || continue; if tr '\\0' '\\n' < \"$p\" 2>/dev/null | grep -Fxq 'PINFOLD_DURABLE_CHILD=crash-child'; then echo survivor; exit 1; fi; done; echo clean");
        assert.match(clean, /clean/);
        assert.equal(await readFile(resolve(scratch, "project-crash", "invocations"), "utf8"), "once\n", "unsafe command must not run again");
      }
      delta = { role: "assistant", content: `${kind}-complete` };
      finish = "stop";
    }
    response.writeHead(200, { "Content-Type": "text/event-stream" });
    const part = (change: object, reason: string | null) => ({ id: `chatcmpl-${requests.length}`, object: "chat.completion.chunk", created: 0, model: "fixture", choices: [{ index: 0, delta: change, finish_reason: reason }] });
    response.write(`data: ${JSON.stringify(part(delta, null))}\n\n`);
    response.end(`data: ${JSON.stringify(part({}, finish))}\n\ndata: [DONE]\n\n`);
  } catch (error) { fixtureFailure = error; response.writeHead(500); response.end("fixture assertion failed"); }
});
await new Promise<void>((done) => server.listen(0, "127.0.0.1", done));
const address = server.address();
assert(address && typeof address === "object");
const endpoint = `http://127.0.0.1:${address.port}/v1`;

function launch(mode: "start" | "resume", kind: string) {
  const state = resolve(scratch, `checkpoint-${kind}`);
  const args = [resolve(import.meta.dirname, "prototype.ts"), mode, "--state", state];
  if (mode === "start") args.push("--pinfold", binary, "--image", values.image!, "--project", resolve(scratch, `project-${kind}`), "--base-url", endpoint, "--model", "fixture", "--prompt", kind);
  const child = spawn(process.execPath, args, { env, stdio: ["ignore", "pipe", "pipe"] });
  children.add(child);
  let stderr = "";
  child.stderr.setEncoding("utf8").on("data", (chunk) => { stderr += chunk; });
  const reader = createInterface({ input: child.stdout });
  reader.on("line", (line) => {
    const message = JSON.parse(line);
    if (message.box?.startsWith("durable-")) ownedJobs.add(message.box.slice("durable-".length));
  });
  const timer = setTimeout(() => child.kill("SIGKILL"), 90_000);
  const closed = new Promise<{ code: number | null; stderr: string }>((done, fail) => {
    child.once("error", fail);
    child.once("close", (code) => { clearTimeout(timer); reader.close(); children.delete(child); done({ code, stderr }); });
  });
  return { child, closed };
}

try {
  for (const kind of ["control", "crash", "cancel"]) {
    const project = resolve(scratch, `project-${kind}`);
    await mkdir(project, { recursive: true });
    for (const path of [".git", ".vscode", ".claude", ".idea"]) await mkdir(resolve(project, path));
  }
  // Sabotage: remove host execution-authority protection as a group, or
  // ignore canonical symlink targets. Refused inputs then reach host work.
  const project = resolve(scratch, "project-control");
  const copiedBinary = resolve(project, "pinfold");
  await copyFile(binary, copiedBinary);
  const alias = resolve(scratch, "project-alias");
  await symlink(project, alias);
  const external = resolve(scratch, "external-authority");
  await mkdir(external);
  const replaceableAlias = resolve(project, "authority-alias");
  await symlink(external, replaceableAlias);
  const entryAlias = resolve(project, "controller-link.ts");
  await symlink(resolve(import.meta.dirname, "prototype.ts"), entryAlias);
  const refusedCases = [
    { name: "binary", project, binary: copiedBinary },
    { name: "checkpoint", project, state: resolve(alias, "checkpoint") },
    { name: "controller", project: await realpath(import.meta.dirname) },
    { name: "controller-entry-alias", project, script: entryAlias },
    { name: "node", project: resolve(await realpath(process.execPath), "..") },
    { name: "replaceable-xdg-alias", project, childEnv: { ...env, XDG_STATE_HOME: replaceableAlias } },
    { name: "replaceable-path-alias", project, childEnv: { ...env, PATH: `${replaceableAlias}${delimiter}${env.PATH ?? ""}` } },
    ...["XDG_STATE_HOME", "XDG_CONFIG_HOME", "XDG_CACHE_HOME"].map((variable) => ({ name: variable, project, childEnv: { ...env, [variable]: resolve(alias, "missing", "xdg") } })),
  ];
  for (const item of refusedCases) {
    const state = "state" in item ? item.state : resolve(scratch, `refused-${item.name}`);
    const result = await run(process.execPath, ["script" in item ? item.script! : resolve(import.meta.dirname, "prototype.ts"), "start", "--state", state!, "--pinfold", "binary" in item ? item.binary! : binary, "--image", values.image!, "--project", item.project, "--base-url", endpoint, "--model", "fixture", "--prompt", "control"], "childEnv" in item ? item.childEnv : env);
    assert.notEqual(result.code, 0, item.name);
    assert.match(result.stderr, /host-boundary:/, item.name);
    await assert.rejects(access(state!), { code: "ENOENT" });
    assert.equal(requests.length, 0, "refusal must precede model work");
  }
  await rm(copiedBinary);
  await rm(replaceableAlias);
  await rm(entryAlias);
  // Sabotage: parse stderr as part of the JSON line stream. A real soft
  // maintenance failure then breaks list/reclaim despite successful stdout.
  const stamp = resolve(env.XDG_STATE_HOME!, "pinfold", "maintenance");
  await mkdir(stamp, { recursive: true });
  await utimes(stamp, 1, 1);
  const diagnostic = await run(binary, ["box", "list", "--label", "dev.example.pi-durable.job"]);
  assert.equal(diagnostic.code, 0, diagnostic.stderr);
  assert(diagnostic.stderr.length > 0, "real CLI maintenance fixture must produce a stderr diagnostic");
  const control = launch("start", "control");
  const completed = await control.closed;
  assert.equal(completed.code, 0, completed.stderr);
  assert.equal(await readFile(resolve(scratch, "project-control/control"), "utf8"), "control-ok");
  assert.equal((await boxes()).length, 0, "successful command removes its box");

  // Sabotage: declare boxed_bash replay safe. The recovery tool runs the
  // mutation twice and stays waiting on its new child instead of completing.
  // Sabotage: resume before removing old work. Generation/child checks fail.
  const reached = waitForFile(resolve(scratch, "project-crash"), "child-ready");
  const crash = launch("start", "crash");
  await reached;
  const before = await boxes();
  assert.equal(before.length, 1);
  generations.set("crash", before[0].labels["dev.pinfold.generation"]);
  assert.match(await guest(before[0].name, "printf live-control"), /live-control/);
  const competing = launch("resume", "crash");
  const refused = await competing.closed;
  assert.notEqual(refused.code, 0);
  assert.match(refused.stderr, /another writer owns this checkpoint/);
  assert.match(await guest(before[0].name, "printf still-live"), /still-live/, "a refused writer must leave the live job alone");
  crash.child.kill("SIGKILL");
  await crash.closed;
  const recovered = launch("resume", "crash");
  const recovery = await recovered.closed;
  assert.equal(recovery.code, 0, recovery.stderr);
  assert.equal(await readFile(resolve(scratch, "project-crash/invocations"), "utf8"), "once\n");
  assert.equal((await boxes()).length, 0);

  // Sabotage: kill only the host exec client on SIGTERM. Its box and
  // background tail survive, so the runtime box absence check fails.
  const cancelReady = waitForFile(resolve(scratch, "project-cancel"), "child-ready");
  const cancel = launch("start", "cancel");
  await cancelReady;
  cancel.child.kill("SIGTERM");
  const cancelled = await cancel.closed;
  assert.equal(cancelled.code, 130, cancelled.stderr);
  assert.equal((await boxes()).length, 0, "cancellation removes the entire box and its child");
  assert.equal(await readFile(resolve(scratch, "project-cancel/invocations"), "utf8"), "once\n");
  assert.equal(fixtureFailure, undefined, String(fixtureFailure));
  assert.equal(requests.filter((request) => request.kind === "crash").length, 2, "one original tool request and one resumed interrupted result");
  console.log("PASS: host-boundary refusals, boxed command, exclusive writer, crash recovery without unsafe replay, whole-box cancellation");
} finally {
  for (const child of children) child.kill("SIGKILL");
  // A startup failure may precede its first event. Read only our own immutable
  // manifests so teardown cannot select another prototype's job.
  for (const kind of ["control", "crash", "cancel"]) {
    try { ownedJobs.add(JSON.parse(await readFile(resolve(scratch, `checkpoint-${kind}`, "job.json"), "utf8")).id); }
    catch (error) { if ((error as NodeJS.ErrnoException).code !== "ENOENT") throw error; }
  }
  for (const box of await boxes()) await run(binary, ["box", "down", box.name]);
  server.closeAllConnections();
  await new Promise<void>((done, fail) => server.close((error) => error ? fail(error) : done()));
  await rm(scratch, { recursive: true, force: true });
}
