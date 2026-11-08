// npm run status-page: serves status/index.html on http://127.0.0.1:8494 (this machine only).
// The page reads the contract id from testnet/run.json (or ?contract=<id>) and everything else from
// the public testnet RPC in your browser. No key, nothing hosted.
import { createServer } from "node:http";
import { readFileSync } from "node:fs";

const files = { "/": ["status/index.html", "text/html"], "/testnet/run.json": ["testnet/run.json", "application/json"] };
createServer((req, res) => {
  const file = files[new URL(req.url, "http://x").pathname];
  if (!file) return res.writeHead(404).end();
  res.writeHead(200, { "content-type": file[1] }).end(readFileSync(file[0]));
}).listen(8494, "127.0.0.1", () => console.log("Calydon status page: http://127.0.0.1:8494  (Ctrl-C to stop)"));
