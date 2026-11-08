// npm run page: serves the page (status/) on http://127.0.0.1:8494 (this machine only).
// The page is static: any static host serving this repository's status/ and testnet/run.json as-is
// works the same. It reads the contract id from testnet/run.json (or ?contract=<id>), reads state
// from the public testnet RPC and signs through the wallet you sign in with. No key here, nothing hosted.
import { createServer } from "node:http";
import { readFileSync, existsSync } from "node:fs";
import { normalize, extname } from "node:path";

const types = { ".html": "text/html", ".js": "text/javascript", ".json": "application/json" };
const allowed = (p) => p === "testnet/run.json" || (p.startsWith("status/") && !p.startsWith("status/vendor-src/"));
createServer((req, res) => {
  let path = decodeURIComponent(new URL(req.url, "http://x").pathname);
  if (path === "/") return res.writeHead(302, { location: "/status/" }).end();
  if (path.endsWith("/")) path += "index.html";
  const file = normalize(path.slice(1));
  if (!allowed(file) || !existsSync(file)) return res.writeHead(404).end();
  res.writeHead(200, { "content-type": types[extname(file)] || "application/octet-stream" }).end(readFileSync(file));
}).listen(8494, "127.0.0.1", () => console.log("Calydon page: http://127.0.0.1:8494/status/  (Ctrl-C to stop)"));
