#!/usr/bin/env node

import { randomBytes } from "node:crypto";
import { createServer } from "node:http";
import { spawn } from "node:child_process";
import { resolve } from "node:path";

const host = "127.0.0.1";
const nonce = randomBytes(24).toString("hex");
const route = `/${nonce}`;
const captureScript = resolve(process.cwd(), "scripts", "capture-linear.mjs");
let used = false;

const server = createServer(async (request, response) => {
  response.setHeader("Cache-Control", "no-store, max-age=0");
  response.setHeader(
    "Content-Security-Policy",
    "default-src 'none'; style-src 'unsafe-inline'; form-action 'self'; base-uri 'none'",
  );
  response.setHeader("Referrer-Policy", "no-referrer");
  response.setHeader("X-Content-Type-Options", "nosniff");

  if (request.url !== route) {
    respond(response, 404, "Not found");
    return;
  }

  if (request.method === "GET" && !used) {
    response.writeHead(200, { "Content-Type": "text/html; charset=utf-8" });
    response.end(`<!doctype html>
      <meta charset="utf-8">
      <meta name="viewport" content="width=device-width">
      <title>Direct Linear capture</title>
      <style>
        body { font: 16px system-ui; max-width: 42rem; margin: 5rem auto; padding: 0 1rem; color: #171717; }
        label, input, button { display: block; width: 100%; box-sizing: border-box; }
        input, button { font: inherit; padding: .75rem; margin-top: .5rem; }
        button { margin-top: 1rem; cursor: pointer; }
        small { color: #555; }
      </style>
      <h1>Direct Linear migration capture</h1>
      <p>This one-time page sends the temporary read-only key only to the local capture process on this computer.</p>
      <form method="post" action="${route}" autocomplete="off">
        <label for="key">Temporary Linear API key</label>
        <input id="key" name="key" type="password" required pattern="lin_api_.+" autocomplete="off" spellcheck="false">
        <button type="submit">Run private capture</button>
      </form>
      <p><small>The key is held in memory for this run and is not written to disk.</small></p>`);
    return;
  }

  if (request.method !== "POST" || used) {
    respond(response, 410, "This handoff has expired.");
    return;
  }

  used = true;
  try {
    const body = await readBoundedBody(request, 2048);
    const key = new URLSearchParams(body).get("key")?.trim();
    if (!key?.startsWith("lin_api_")) {
      respond(response, 400, "The submitted value was not a Linear API key.");
      shutdown();
      return;
    }

    response.writeHead(200, { "Content-Type": "text/html; charset=utf-8" });
    response.write(
      "<!doctype html><meta charset=utf-8><title>Direct capture running</title><h1>Capture running</h1><p>Keep this page open. The result will appear here when verification completes.</p>",
    );

    const child = spawn(process.execPath, [captureScript], {
      cwd: process.cwd(),
      env: { ...process.env, LINEAR_API_KEY: key },
      stdio: ["ignore", "inherit", "inherit"],
    });

    child.on("error", (error) => {
      response.end(
        `<h2>Capture could not start</h2><pre>${escapeHtml(error.message)}</pre>`,
      );
      shutdown();
    });
    child.on("exit", (code, signal) => {
      if (code === 0) {
        response.end(
          "<h2>Capture completed successfully</h2><p>The temporary key can now be revoked.</p>",
        );
      } else {
        response.end(
          `<h2>Capture needs attention</h2><p>Exit code: ${escapeHtml(String(code ?? signal ?? "unknown"))}</p>`,
        );
      }
      shutdown();
    });
  } catch (error) {
    respond(
      response,
      400,
      error instanceof Error ? error.message : String(error),
    );
    shutdown();
  }
});

server.listen(0, host, () => {
  const address = server.address();
  if (!address || typeof address === "string")
    throw new Error("Could not determine the local handoff port.");
  console.log(`READY http://${host}:${address.port}${route}`);
});

function respond(response, status, message) {
  response.writeHead(status, { "Content-Type": "text/plain; charset=utf-8" });
  response.end(message);
}

async function readBoundedBody(request, limit) {
  let body = "";
  for await (const chunk of request) {
    body += chunk;
    if (Buffer.byteLength(body) > limit)
      throw new Error("Submitted value was too large.");
  }
  return body;
}

function escapeHtml(value) {
  return value.replace(
    /[&<>"']/g,
    (character) =>
      ({
        "&": "&amp;",
        "<": "&lt;",
        ">": "&gt;",
        '"': "&quot;",
        "'": "&#39;",
      })[character],
  );
}

function shutdown() {
  server.close();
  setTimeout(() => process.exit(), 250).unref();
}
