// is-my-json-valid (via @near-js/accounts) builds validators with util.format("%s", ...); this is
// the subset it uses, so the browser bundle does not need Node's util.
export function format(f, ...args) {
  let i = 0;
  const s = String(f).replace(/%[sdj%]/g, (m) => (m === "%%" ? "%" : m === "%j" ? JSON.stringify(args[i++]) : String(args[i++])));
  return args.slice(i).reduce((acc, a) => acc + " " + String(a), s);
}
export default { format };
