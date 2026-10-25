// Arithmetic expressions in number fields: `512*2`, `(1+2)*3`, `sqrt(2)`, `pi/4`, `2^10`, `7 % 3`.
// A small recursive-descent parser — no `Function`, no `eval` — that returns null for anything it
// does not understand, so a plain number path stays the default and garbage never becomes NaN.

const CONSTS: Record<string, number> = { pi: Math.PI, e: Math.E, tau: 2 * Math.PI };
const FNS: Record<string, (...a: number[]) => number> = {
  sqrt: Math.sqrt,
  abs: Math.abs,
  floor: Math.floor,
  ceil: Math.ceil,
  round: Math.round,
  sin: Math.sin,
  cos: Math.cos,
  tan: Math.tan,
  log: Math.log,
  log2: Math.log2,
  exp: Math.exp,
  min: Math.min,
  max: Math.max,
  pow: Math.pow,
};

/** Evaluate an arithmetic expression; null when it is not one (or divides by zero etc.). */
export function evalExpr(text: string): number | null {
  const src = text.trim();
  if (src === "") return null;
  let i = 0;
  const peek = () => src[i] ?? "";
  const skip = () => {
    while (peek() === " ") i++;
  };
  const fail = (): never => {
    throw new Error("expr");
  };
  const number = (): number => {
    const m = /^(\d+\.?\d*|\.\d+)([eE][+-]?\d+)?/.exec(src.slice(i));
    if (!m) fail();
    i += m![0].length;
    return Number(m![0]);
  };
  const ident = (): string => {
    const m = /^[a-zA-Z_][a-zA-Z0-9_]*/.exec(src.slice(i));
    if (!m) fail();
    i += m![0].length;
    return m![0];
  };
  const args = (): number[] => {
    const out: number[] = [];
    skip();
    if (peek() === ")") return out;
    for (;;) {
      out.push(expr());
      skip();
      if (peek() === ",") {
        i++;
        continue;
      }
      return out;
    }
  };
  const atom = (): number => {
    skip();
    const c = peek();
    if (c === "(") {
      i++;
      const v = expr();
      skip();
      if (peek() !== ")") fail();
      i++;
      return v;
    }
    if (c === "-") {
      i++;
      return -unary();
    }
    if (c === "+") {
      i++;
      return unary();
    }
    if (/[0-9.]/.test(c)) return number();
    if (/[a-zA-Z_]/.test(c)) {
      const name = ident().toLowerCase();
      skip();
      if (peek() === "(") {
        i++;
        const a = args();
        skip();
        if (peek() !== ")") fail();
        i++;
        const f = FNS[name];
        if (!f) fail();
        return f!(...a);
      }
      if (name in CONSTS) return CONSTS[name]!;
      fail();
    }
    return fail();
  };
  const unary = (): number => {
    const base = atom();
    skip();
    if (peek() === "^") {
      i++;
      return Math.pow(base, unary()); // right-associative
    }
    return base;
  };
  const term = (): number => {
    let v = unary();
    for (;;) {
      skip();
      const c = peek();
      if (c === "*") {
        i++;
        v *= unary();
      } else if (c === "/") {
        i++;
        v /= unary();
      } else if (c === "%") {
        i++;
        v %= unary();
      } else return v;
    }
  };
  const expr = (): number => {
    let v = term();
    for (;;) {
      skip();
      const c = peek();
      if (c === "+") {
        i++;
        v += term();
      } else if (c === "-") {
        i++;
        v -= term();
      } else return v;
    }
  };
  try {
    const v = expr();
    skip();
    if (i !== src.length) return null;
    return Number.isFinite(v) ? v : null;
  } catch {
    return null;
  }
}

/** A number field's text as a number: a plain number, else an expression, else NaN (as `Number` would give). */
export function num(text: string): number {
  const plain = Number(text);
  if (text.trim() !== "" && Number.isFinite(plain)) return plain;
  const v = evalExpr(text);
  return v === null ? Number.NaN : v;
}
