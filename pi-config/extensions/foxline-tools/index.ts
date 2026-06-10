import type { ExtensionAPI, ToolResult } from "@earendil-works/pi-coding-agent";
import { Type, type Static } from "typebox";
import { promises as fs } from "node:fs";
import path from "node:path";

const MAX_LIMIT = 200;
const DEFAULT_LIMIT = 30;
const DEFAULT_LIST_LIMIT = 100;
const DEFAULT_WEB_LIMIT = 8;
const DEFAULT_IGNORES = [".git", "node_modules", "dist", "target"];
const EXA_MCP_URL = process.env.EXA_MCP_URL ?? "https://mcp.exa.ai/mcp";
const EXA_TIMEOUT_MS = Number(process.env.FOXLINE_EXA_TIMEOUT_MS ?? 900);
const JINA_TIMEOUT_MS = Number(process.env.FOXLINE_JINA_TIMEOUT_MS ?? 6000);
const DDG_TIMEOUT_MS = Number(process.env.FOXLINE_DDG_TIMEOUT_MS ?? 3000);
const MAX_FETCH_CHARS = Number(process.env.FOXLINE_FETCH_MAX_CHARS ?? 20000);

type Engine = "rg" | "grep" | "fd" | "find" | "builtin";

type SearchAction = Static<typeof SearchActionSchema>;
const SearchActionSchema = Type.Union([
  Type.Literal("capabilities"),
  Type.Literal("find_paths"),
  Type.Literal("grep_content"),
  Type.Literal("list_dir"),
]);

const FileParamsSchema = Type.Object({
  action: SearchActionSchema,
  query: Type.Optional(Type.String()),
  pattern: Type.Optional(Type.String()),
  path: Type.Optional(Type.String()),
  exclude: Type.Optional(Type.Union([Type.String(), Type.Array(Type.String())])),
  type: Type.Optional(Type.Union([Type.Literal("file"), Type.Literal("dir"), Type.Literal("any")])),
  recursive: Type.Optional(Type.Boolean()),
  limit: Type.Optional(Type.Integer({ minimum: 1, maximum: MAX_LIMIT })),
  cursor: Type.Optional(Type.String()),
  caseSensitive: Type.Optional(Type.Boolean()),
  regex: Type.Optional(Type.Boolean()),
  context: Type.Optional(Type.Integer({ minimum: 0, maximum: 10 })),
});

type FileParams = Static<typeof FileParamsSchema>;

type InternetAction = Static<typeof InternetActionSchema>;
const InternetActionSchema = Type.Union([
  Type.Literal("search_web"),
  Type.Literal("fetch_url"),
  Type.Literal("providers_status"),
]);

const InternetParamsSchema = Type.Object({
  action: InternetActionSchema,
  query: Type.Optional(Type.String()),
  url: Type.Optional(Type.String()),
  maxResults: Type.Optional(Type.Integer({ minimum: 1, maximum: 20 })),
  provider: Type.Optional(Type.Union([Type.Literal("auto"), Type.Literal("exa_mcp"), Type.Literal("ddg")])),
});

type InternetParams = Static<typeof InternetParamsSchema>;

type Capabilities = {
  hasRg: boolean;
  hasGrep: boolean;
  hasFd: boolean;
  hasFind: boolean;
};

export default function foxlineToolsExtension(pi: ExtensionAPI) {
  let capabilities: Capabilities = {
    hasRg: false,
    hasGrep: false,
    hasFd: false,
    hasFind: false,
  };
  let exaReachableCache: boolean | null = null;

  const detectCapabilities = async () => {
    capabilities = {
      hasRg: await hasCommand(pi, "rg"),
      hasGrep: await hasCommand(pi, "grep"),
      hasFd: await hasCommand(pi, "fd"),
      hasFind: await hasCommand(pi, "find"),
    };
  };

  pi.on("session_start", async () => {
    await detectCapabilities();
  });

  pi.registerTool({
    name: "pi_file_searcher",
    label: "PI File Searcher",
    description: "Fast local file/path/content search with rg/grep and fd/find fallbacks",
    parameters: FileParamsSchema,
    async execute(_toolCallId, params, _signal, _onUpdate, ctx): Promise<ToolResult> {
      const workspaceRoot = ctx.cwd;
      const action: SearchAction = params.action;

      if (action === "capabilities") {
        return okResult({
          ok: true,
          workspaceRoot,
          engines: {
            contentSearch: ["rg", "grep", "builtin"],
            pathSearch: ["fd", "find", "builtin"],
          },
          selected: {
            contentSearch: capabilities.hasRg ? "rg" : capabilities.hasGrep ? "grep" : "builtin",
            pathSearch: capabilities.hasFd ? "fd" : capabilities.hasFind ? "find" : "builtin",
          },
          limits: { maxLimit: MAX_LIMIT },
          ignores: DEFAULT_IGNORES,
        });
      }

      if (action === "find_paths") return okResult(await findPaths(pi, capabilities, workspaceRoot, params));
      if (action === "grep_content") {
        if (!params.pattern) return errorResult("pattern is required for grep_content");
        return okResult(await grepContent(pi, capabilities, workspaceRoot, params));
      }
      if (action === "list_dir") return okResult(await listDir(workspaceRoot, params));

      return errorResult(`unsupported action: ${action}`);
    },
  });

  pi.registerTool({
    name: "pi_internet_search",
    label: "PI Internet Search",
    description: "Web search + page fetch (Exa MCP first, DDG fallback, Jina reader extraction)",
    parameters: InternetParamsSchema,
    async execute(_toolCallId, params): Promise<ToolResult> {
      try {
        if (params.action === "providers_status") {
          const exaOk = await exaMcpPing(EXA_TIMEOUT_MS);
          exaReachableCache = exaOk;
          return okResult({
            ok: true,
            providers: {
              exa_mcp: { url: EXA_MCP_URL, reachable: exaOk },
              ddg: { reachable: true },
              jina_reader: { reachable: true },
            },
          });
        }

        if (params.action === "search_web") {
          if (!params.query) return errorResult("query is required for search_web");
          const maxResults = Math.max(1, Math.min(20, params.maxResults ?? DEFAULT_WEB_LIMIT));
          const provider = params.provider ?? "auto";

          let results: any[] = [];
          let used = "ddg";

          if (provider === "exa_mcp" || provider === "auto") {
            const shouldTryExa = provider === "exa_mcp" || exaReachableCache !== false;
            const exa = shouldTryExa ? await exaMcpSearch(params.query, maxResults, EXA_TIMEOUT_MS) : { ok: false, results: [] };
            if (exa.ok && exa.results.length > 0) {
              results = exa.results;
              used = "exa_mcp";
              exaReachableCache = true;
            } else {
              exaReachableCache = false;
            }
          }

          if (results.length === 0 && (provider === "ddg" || provider === "auto" || provider === "exa_mcp")) {
            results = await ddgSearch(params.query, maxResults, DDG_TIMEOUT_MS);
            used = "ddg";
          }

          return okResult({ ok: true, provider: used, query: params.query, results });
        }

        if (params.action === "fetch_url") {
          if (!params.url) return errorResult("url is required for fetch_url");
          const content = await fetchViaJina(params.url, JINA_TIMEOUT_MS);
          return okResult({ ok: true, provider: "jina_reader", url: params.url, content });
        }

        return errorResult(`unsupported action: ${params.action}`);
      } catch (error) {
        return errorResult(error instanceof Error ? error.message : String(error));
      }
    },
  });
}

function okResult(details: unknown): ToolResult {
  return { content: [{ type: "text", text: JSON.stringify(details, null, 2) }], details };
}

function errorResult(message: string): ToolResult {
  return {
    content: [{ type: "text", text: message }],
    isError: true,
    details: { ok: false, error: message },
  };
}

async function hasCommand(pi: ExtensionAPI, command: string): Promise<boolean> {
  const result = await pi.exec("sh", ["-lc", `command -v ${shellEscape(command)} >/dev/null 2>&1`]);
  return result.code === 0;
}

function parseExclude(value: FileParams["exclude"]): string[] {
  if (!value) return [];
  if (Array.isArray(value)) return value;
  return value.split(/[\s,]+/).map((v) => v.trim()).filter(Boolean);
}

function clampLimit(limit: number | undefined, fallback: number): number {
  return Math.max(1, Math.min(MAX_LIMIT, limit ?? fallback));
}

function shellEscape(v: string): string {
  return `'${v.replace(/'/g, `"'"'`)}'`;
}

function withinRoot(root: string, requested?: string): string {
  const normalized = path.resolve(root, requested ?? ".");
  if (!normalized.startsWith(root)) throw new Error(`Path escapes workspace root: ${requested}`);
  return normalized;
}

async function findPaths(pi: ExtensionAPI, caps: Capabilities, root: string, params: FileParams) {
  const limit = clampLimit(params.limit, DEFAULT_LIMIT);
  const query = (params.query ?? "").trim();
  const baseAbs = withinRoot(root, params.path ?? ".");
  const exclude = parseExclude(params.exclude);

  if (caps.hasFd) {
    const args = ["--hidden", "--follow", "--color", "never", "--base-directory", baseAbs, "."];
    const type = params.type ?? "any";
    if (type === "file") args.unshift("--type", "f");
    if (type === "dir") args.unshift("--type", "d");
    for (const ex of [...DEFAULT_IGNORES, ...exclude]) args.unshift("--exclude", ex);
    if (query) args.unshift(query);
    const result = await pi.exec("fd", args);
    const items = splitLines(result.stdout).slice(0, limit).map((entry) => ({ path: path.relative(root, path.resolve(baseAbs, entry)) }));
    return { ok: true, engine: "fd" as Engine, items, nextCursor: null };
  }

  if (caps.hasFind) {
    const result = await pi.exec("find", [baseAbs, "-mindepth", "1"]);
    const items = splitLines(result.stdout)
      .filter((entry) => !isIgnored(path.relative(root, entry), exclude))
      .filter((entry) => (query ? path.basename(entry).toLowerCase().includes(query.toLowerCase()) : true))
      .slice(0, limit)
      .map((entry) => ({ path: path.relative(root, entry) }));
    return { ok: true, engine: "find" as Engine, items, nextCursor: null };
  }

  const items = await builtinWalk(root, baseAbs, { limit, recursive: true, exclude, query, type: params.type ?? "any" });
  return { ok: true, engine: "builtin" as Engine, items, nextCursor: null };
}

async function grepContent(pi: ExtensionAPI, caps: Capabilities, root: string, params: FileParams) {
  const limit = clampLimit(params.limit, 20);
  const baseAbs = withinRoot(root, params.path ?? ".");
  const exclude = parseExclude(params.exclude);
  const context = Math.max(0, Math.min(10, params.context ?? 0));
  const pattern = params.pattern ?? "";

  if (caps.hasRg) {
    const args = ["--line-number", "--column", "--no-heading", "--color", "never", "--max-count", String(limit)];
    if (context > 0) args.push("--context", String(context));
    if (params.caseSensitive) args.push("--case-sensitive");
    if (!params.regex) args.push("--fixed-strings");
    for (const ex of [...DEFAULT_IGNORES, ...exclude]) args.push("--glob", `!${ex}`);
    args.push(pattern, baseAbs);

    const result = await pi.exec("rg", args);
    const matches = splitLines(result.stdout).map((line) => parseRgLine(root, line)).filter(Boolean).slice(0, limit);
    return { ok: true, engine: "rg" as Engine, matches, nextCursor: null };
  }

  if (caps.hasGrep) {
    const args = ["-R", "-n", "-H", "--binary-files=without-match"];
    if (params.caseSensitive !== true) args.push("-i");
    if (!params.regex) args.push("-F");
    args.push(pattern, baseAbs);
    const result = await pi.exec("grep", args);
    const matches = splitLines(result.stdout)
      .map((line) => parseGrepLine(root, line))
      .filter((m): m is { path: string; line: number; match: string } => Boolean(m))
      .filter((m) => !isIgnored(m.path, exclude))
      .slice(0, limit);
    return { ok: true, engine: "grep" as Engine, matches, nextCursor: null };
  }

  const matches = await builtinGrep(root, baseAbs, {
    pattern,
    regex: params.regex ?? false,
    caseSensitive: params.caseSensitive ?? false,
    limit,
    exclude,
  });
  return { ok: true, engine: "builtin" as Engine, matches, nextCursor: null };
}

async function listDir(root: string, params: FileParams) {
  const limit = clampLimit(params.limit, DEFAULT_LIST_LIMIT);
  const baseAbs = withinRoot(root, params.path ?? ".");
  const exclude = parseExclude(params.exclude);
  const recursive = params.recursive ?? false;
  const items = await builtinWalk(root, baseAbs, {
    limit,
    recursive,
    exclude,
    query: "",
    type: "any",
    withStat: true,
  });
  return { ok: true, engine: "builtin" as Engine, items, nextCursor: null };
}

function splitLines(value: string): string[] {
  return value.split(/\r?\n/).map((v) => v.trim()).filter(Boolean);
}

function isIgnored(relPath: string, extra: string[]): boolean {
  const parts = relPath.split(path.sep);
  for (const ignored of DEFAULT_IGNORES) if (parts.includes(ignored)) return true;
  for (const ex of extra) if (ex && relPath.includes(ex.replace("/**", ""))) return true;
  return false;
}

function parseRgLine(root: string, line: string) {
  const m = line.match(/^(.*?):(\d+):(\d+):(.*)$/);
  if (!m) return null;
  return { path: path.relative(root, m[1]), line: Number(m[2]), column: Number(m[3]), match: m[4] };
}

function parseGrepLine(root: string, line: string) {
  const m = line.match(/^(.*?):(\d+):(.*)$/);
  if (!m) return null;
  return { path: path.relative(root, m[1]), line: Number(m[2]), match: m[3] };
}

async function builtinWalk(
  root: string,
  start: string,
  options: {
    limit: number;
    recursive: boolean;
    exclude: string[];
    query: string;
    type: "file" | "dir" | "any";
    withStat?: boolean;
  }
) {
  const results: Array<Record<string, unknown>> = [];
  const queue: string[] = [start];

  while (queue.length > 0 && results.length < options.limit) {
    const current = queue.shift()!;
    const entries = await fs.readdir(current, { withFileTypes: true });

    for (const entry of entries) {
      if (results.length >= options.limit) break;
      if (DEFAULT_IGNORES.includes(entry.name)) continue;

      const abs = path.join(current, entry.name);
      const rel = path.relative(root, abs);
      if (isIgnored(rel, options.exclude)) continue;
      if (options.query && !rel.toLowerCase().includes(options.query.toLowerCase())) {
        if (entry.isDirectory() && options.recursive) queue.push(abs);
        continue;
      }

      const type = entry.isDirectory() ? "dir" : "file";
      if (options.type === "any" || options.type === type) {
        if (options.withStat) {
          const st = await fs.stat(abs);
          results.push({ path: rel, type, size: type === "file" ? st.size : null, mtime: st.mtime.toISOString() });
        } else {
          results.push({ path: rel, type });
        }
      }

      if (entry.isDirectory() && options.recursive) queue.push(abs);
    }
  }

  return results;
}

async function builtinGrep(
  root: string,
  start: string,
  options: {
    pattern: string;
    regex: boolean;
    caseSensitive: boolean;
    limit: number;
    exclude: string[];
  }
) {
  const walker = await builtinWalk(root, start, {
    limit: Number.MAX_SAFE_INTEGER,
    recursive: true,
    exclude: options.exclude,
    query: "",
    type: "file",
  });

  const matcher = options.regex ? new RegExp(options.pattern, options.caseSensitive ? "g" : "gi") : null;
  const matches: Array<{ path: string; line: number; match: string }> = [];

  for (const item of walker) {
    if (matches.length >= options.limit) break;
    const relPath = String(item.path);
    const abs = path.resolve(root, relPath);
    let text: string;
    try {
      text = await fs.readFile(abs, "utf8");
    } catch {
      continue;
    }

    const lines = text.split(/\r?\n/);
    for (let idx = 0; idx < lines.length; idx += 1) {
      if (matches.length >= options.limit) break;
      const line = lines[idx];
      const ok = matcher
        ? matcher.test(line)
        : options.caseSensitive
          ? line.includes(options.pattern)
          : line.toLowerCase().includes(options.pattern.toLowerCase());
      if (ok) matches.push({ path: relPath, line: idx + 1, match: line });
      if (matcher) matcher.lastIndex = 0;
    }
  }

  return matches;
}

async function exaMcpPing(timeoutMs: number): Promise<boolean> {
  try {
    const res = await fetchWithTimeout(
      EXA_MCP_URL,
      {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ jsonrpc: "2.0", id: "ping", method: "tools/list", params: {} }),
      },
      timeoutMs
    );
    return res.ok;
  } catch {
    return false;
  }
}

async function exaMcpSearch(query: string, maxResults: number, timeoutMs: number): Promise<{ ok: boolean; results: any[] }> {
  try {
    const payloads = [
      { jsonrpc: "2.0", id: "search-1", method: "tools/call", params: { name: "search", arguments: { query, numResults: maxResults } } },
      { jsonrpc: "2.0", id: "search-2", method: "tools/call", params: { name: "web_search_exa", arguments: { query, numResults: maxResults } } },
      { jsonrpc: "2.0", id: "search-3", method: "tools/call", params: { name: "exa_search", arguments: { query, numResults: maxResults } } },
    ];

    for (const body of payloads) {
      const res = await fetchWithTimeout(
        EXA_MCP_URL,
        {
          method: "POST",
          headers: { "content-type": "application/json" },
          body: JSON.stringify(body),
        },
        timeoutMs
      );
      if (!res.ok) continue;
      const data: any = await res.json().catch(() => null);
      const extracted = normalizeExaMcpResults(data);
      if (extracted.length > 0) return { ok: true, results: extracted.slice(0, maxResults) };
    }
  } catch {
    // fall through
  }
  return { ok: false, results: [] };
}

function normalizeExaMcpResults(data: any): any[] {
  const textBlocks = data?.result?.content?.filter((c: any) => c?.type === "text") ?? [];
  const parsedObjects: any[] = [];
  for (const block of textBlocks) {
    const txt = String(block.text ?? "").trim();
    if (!txt) continue;
    try {
      const parsed = JSON.parse(txt);
      if (Array.isArray(parsed)) parsedObjects.push(...parsed);
      else parsedObjects.push(parsed);
    } catch {
      // ignore non-json blocks
    }
  }

  const fromData = data?.result?.data?.results ?? data?.result?.results ?? [];
  const merged = [...(Array.isArray(fromData) ? fromData : []), ...parsedObjects.flatMap((v) => (Array.isArray(v?.results) ? v.results : [v]))];

  return merged
    .map((item: any) => ({
      title: item.title ?? item.name ?? "(untitled)",
      url: item.url ?? item.link ?? item.id ?? "",
      snippet: item.snippet ?? item.text ?? item.description ?? "",
    }))
    .filter((item: any) => item.url);
}

async function ddgSearch(query: string, maxResults: number, timeoutMs: number): Promise<any[]> {
  const url = `https://html.duckduckgo.com/html/?q=${encodeURIComponent(query)}`;
  const res = await fetchWithTimeout(url, { headers: { "user-agent": "foxline-pi/1.0" } }, timeoutMs);
  if (!res.ok) return [];
  const html = await res.text();

  const matches = [...html.matchAll(/<a[^>]*class="result__a"[^>]*href="([^"]+)"[^>]*>(.*?)<\/a>/g)];
  return matches.slice(0, maxResults).map((m) => ({
    title: stripHtml(m[2]),
    url: decodeDdgRedirect(m[1]),
    snippet: "",
  }));
}

function decodeDdgRedirect(url: string): string {
  try {
    const u = new URL(url, "https://duckduckgo.com");
    const target = u.searchParams.get("uddg");
    return target ? decodeURIComponent(target) : u.toString();
  } catch {
    return url;
  }
}

function stripHtml(s: string): string {
  return s.replace(/<[^>]+>/g, "").replace(/\s+/g, " ").trim();
}

async function fetchViaJina(url: string, timeoutMs: number): Promise<string> {
  const normalized = url.match(/^https?:\/\//) ? url : `https://${url}`;
  const res = await fetchWithTimeout(`https://r.jina.ai/http://${normalized.replace(/^https?:\/\//, "")}`, {}, timeoutMs);
  if (!res.ok) throw new Error(`jina reader failed: ${res.status}`);
  const text = await res.text();
  return text.length > MAX_FETCH_CHARS ? `${text.slice(0, MAX_FETCH_CHARS)}\n\n[truncated]` : text;
}

async function fetchWithTimeout(url: string, init: RequestInit, timeoutMs: number): Promise<Response> {
  const controller = new AbortController();
  const timeout = setTimeout(() => controller.abort(), timeoutMs);
  try {
    return await fetch(url, { ...init, signal: controller.signal });
  } finally {
    clearTimeout(timeout);
  }
}
