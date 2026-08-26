import { readFileSync } from "node:fs";
import { basename, relative, resolve } from "node:path";
import { spawnSync } from "node:child_process";
import assert from "node:assert/strict";

const repositoryRoot = resolve(import.meta.dirname, "..");
const config = readJson("config/rust-architecture.json");
const baseline = readJson("config/rust-architecture-baseline.json");
const printBaseline = process.argv.includes("--print-baseline");

runSelfTests();

const sources = trackedRustSources();
const sourceFunctions = new Map();
const observed = {
  fileLines: {},
  functionLines: {},
  cognitiveComplexity: {},
};
const warnings = [];

for (const sourcePath of sources) {
  const source = readFileSync(resolve(repositoryRoot, sourcePath), "utf8");
  const lineCount = countLines(source);
  const entrypoint = ["main.rs", "lib.rs"].includes(basename(sourcePath));
  const limits = entrypoint ? config.fileLines.entrypoint : config.fileLines.ordinary;
  if (lineCount > limits.warning) {
    warnings.push(`${sourcePath}: ${lineCount} 行（提醒阈值 ${limits.warning}）`);
  }
  if (lineCount > limits.maximum) {
    observed.fileLines[sourcePath] = lineCount;
  }

  const functions = parseRustFunctions(sourcePath, source);
  sourceFunctions.set(sourcePath, functions);
  for (const fn of functions) {
    if (fn.lines > config.functionLines.warning) {
      warnings.push(`${fn.key}: ${fn.lines} 行（提醒阈值 ${config.functionLines.warning}）`);
    }
    if (fn.lines > config.functionLines.maximum) {
      observed.functionLines[fn.key] = fn.lines;
    }
  }
}

for (const diagnostic of collectCognitiveComplexityDiagnostics()) {
  const functions = sourceFunctions.get(diagnostic.path) ?? [];
  const owner = functions.find(
    (fn) => fn.startLine <= diagnostic.line && diagnostic.line <= fn.endLine,
  );
  if (!owner) {
    throw new Error(
      `无法把 Clippy 复杂度诊断映射到函数：${diagnostic.path}:${diagnostic.line}`,
    );
  }
  if (diagnostic.value > config.cognitiveComplexity.warning) {
    warnings.push(
      `${owner.key}: 认知复杂度 ${diagnostic.value}（提醒阈值 ${config.cognitiveComplexity.warning}）`,
    );
  }
  if (diagnostic.value > config.cognitiveComplexity.maximum) {
    observed.cognitiveComplexity[owner.key] = diagnostic.value;
  }
}

sortRecord(observed.fileLines);
sortRecord(observed.functionLines);
sortRecord(observed.cognitiveComplexity);

if (printBaseline) {
  process.stdout.write(`${JSON.stringify(observed, null, 2)}\n`);
  process.exit(0);
}

const errors = [
  ...compareDebt("文件长度", observed.fileLines, baseline.fileLines),
  ...compareDebt("函数长度", observed.functionLines, baseline.functionLines),
  ...compareDebt(
    "函数认知复杂度",
    observed.cognitiveComplexity,
    baseline.cognitiveComplexity,
  ),
];

if (errors.length > 0) {
  process.stderr.write("Rust 架构门禁失败：\n");
  process.stderr.write(`${errors.map((error) => `- ${error}`).join("\n")}\n`);
  process.stderr.write(
    "请拆分职责；存量债务下降时同步降低或删除 baseline，禁止提高 baseline。\n",
  );
  process.exit(1);
}

process.stdout.write(
  `Rust 架构门禁通过：${sources.length} 个源文件，${warnings.length} 项提醒，未新增或扩大硬上限债务。\n`,
);

function readJson(path) {
  return JSON.parse(readFileSync(resolve(repositoryRoot, path), "utf8"));
}

function trackedRustSources() {
  const result = run("git", ["ls-files", "-co", "--exclude-standard", "--", "*.rs"]);
  return result.stdout
    .split("\n")
    .map((line) => line.trim())
    .filter(Boolean)
    .filter(
      (path) =>
        !config.excludedPathSegments.some(
          (segment) => path === segment || path.startsWith(`${segment}/`) || path.includes(`/${segment}/`),
        ),
    )
    .sort();
}

function countLines(source) {
  if (source.length === 0) return 0;
  return source.endsWith("\n") ? source.split("\n").length - 1 : source.split("\n").length;
}

function parseRustFunctions(path, source) {
  const sanitized = sanitizeRust(source);
  const lineStarts = [0];
  for (let index = 0; index < source.length; index += 1) {
    if (source[index] === "\n") lineStarts.push(index + 1);
  }

  const functions = [];
  const occurrences = new Map();
  const functionPattern = /\bfn\s+([A-Za-z_][A-Za-z0-9_]*)/g;
  for (const match of sanitized.matchAll(functionPattern)) {
    const name = match[1];
    let cursor = match.index + match[0].length;
    let angleDepth = 0;
    let parenDepth = 0;
    let bracketDepth = 0;
    let bodyStart = -1;
    for (; cursor < sanitized.length; cursor += 1) {
      const char = sanitized[cursor];
      if (char === "<") angleDepth += 1;
      else if (char === ">" && angleDepth > 0) angleDepth -= 1;
      else if (char === "(") parenDepth += 1;
      else if (char === ")" && parenDepth > 0) parenDepth -= 1;
      else if (char === "[") bracketDepth += 1;
      else if (char === "]" && bracketDepth > 0) bracketDepth -= 1;
      else if (char === ";" && angleDepth === 0 && parenDepth === 0 && bracketDepth === 0) break;
      else if (char === "{" && angleDepth === 0 && parenDepth === 0 && bracketDepth === 0) {
        bodyStart = cursor;
        break;
      }
    }
    if (bodyStart < 0) continue;

    const bodyEnd = matchingBrace(sanitized, bodyStart);
    if (bodyEnd < 0) throw new Error(`无法解析函数结束位置：${path}:${name}`);
    const startLine = lineNumber(lineStarts, match.index);
    const endLine = lineNumber(lineStarts, bodyEnd);
    const occurrence = (occurrences.get(name) ?? 0) + 1;
    occurrences.set(name, occurrence);
    functions.push({
      name,
      key: `${path}::${name}#${occurrence}`,
      startLine,
      endLine,
      lines: endLine - startLine + 1,
    });
  }
  return functions;
}

function sanitizeRust(source) {
  const output = [...source];
  let state = "code";
  let blockDepth = 0;
  let rawHashes = 0;
  for (let index = 0; index < source.length; index += 1) {
    const char = source[index];
    const next = source[index + 1];
    if (state === "code") {
      if (char === "/" && next === "/") {
        output[index] = output[index + 1] = " ";
        index += 1;
        state = "line-comment";
      } else if (char === "/" && next === "*") {
        output[index] = output[index + 1] = " ";
        index += 1;
        blockDepth = 1;
        state = "block-comment";
      } else if (char === '"') {
        output[index] = " ";
        state = "string";
      } else if (char === "r") {
        const raw = source.slice(index).match(/^r(#{0,255})"/);
        if (raw) {
          rawHashes = raw[1].length;
          for (let offset = 0; offset < raw[0].length; offset += 1) output[index + offset] = " ";
          index += raw[0].length - 1;
          state = "raw-string";
        }
      } else if (char === "'") {
        const character = source.slice(index).match(/^'(?:\\.|[^\\'\n])'/);
        if (character) {
          for (let offset = 0; offset < character[0].length; offset += 1) output[index + offset] = " ";
          index += character[0].length - 1;
        }
      }
    } else if (state === "line-comment") {
      if (char === "\n") state = "code";
      else output[index] = " ";
    } else if (state === "block-comment") {
      output[index] = char === "\n" ? "\n" : " ";
      if (char === "/" && next === "*") {
        output[index + 1] = " ";
        index += 1;
        blockDepth += 1;
      } else if (char === "*" && next === "/") {
        output[index + 1] = " ";
        index += 1;
        blockDepth -= 1;
        if (blockDepth === 0) state = "code";
      }
    } else if (state === "string") {
      output[index] = char === "\n" ? "\n" : " ";
      if (char === "\\") {
        if (index + 1 < source.length) output[index + 1] = source[index + 1] === "\n" ? "\n" : " ";
        index += 1;
      } else if (char === '"') state = "code";
    } else if (state === "raw-string") {
      output[index] = char === "\n" ? "\n" : " ";
      if (char === '"' && source.slice(index + 1, index + 1 + rawHashes) === "#".repeat(rawHashes)) {
        for (let offset = 1; offset <= rawHashes; offset += 1) output[index + offset] = " ";
        index += rawHashes;
        state = "code";
      }
    }
  }
  return output.join("");
}

function matchingBrace(source, start) {
  let depth = 0;
  for (let index = start; index < source.length; index += 1) {
    if (source[index] === "{") depth += 1;
    else if (source[index] === "}") {
      depth -= 1;
      if (depth === 0) return index;
    }
  }
  return -1;
}

function lineNumber(lineStarts, offset) {
  let low = 0;
  let high = lineStarts.length;
  while (low + 1 < high) {
    const middle = Math.floor((low + high) / 2);
    if (lineStarts[middle] <= offset) low = middle;
    else high = middle;
  }
  return low + 1;
}

function collectCognitiveComplexityDiagnostics() {
  const commands = [
    ["cargo", ["clippy", "--workspace", "--all-targets", "--message-format=json", "--", "--cap-lints", "warn", "-W", "clippy::cognitive_complexity"]],
    ["cargo", ["clippy", "--manifest-path", "src-tauri/Cargo.toml", "--all-targets", "--message-format=json", "--", "--cap-lints", "warn", "-W", "clippy::cognitive_complexity"]],
    ["cargo", ["clippy", "--locked", "--manifest-path", "migration-artifacts/unified-app-id/Cargo.toml", "--all-targets", "--message-format=json", "--", "--cap-lints", "warn", "-W", "clippy::cognitive_complexity"]],
  ];
  const diagnostics = [];
  for (const [command, args] of commands) {
    const result = run(command, args, {
      maxBuffer: 128 * 1024 * 1024,
      suppressStdoutOnFailure: true,
    });
    for (const line of result.stdout.split("\n")) {
      if (!line.startsWith("{")) continue;
      let message;
      try {
        message = JSON.parse(line);
      } catch {
        continue;
      }
      const diagnostic = message?.message;
      if (
        message?.reason !== "compiler-message" ||
        diagnostic?.code?.code !== "clippy::cognitive_complexity"
      ) continue;
      const span = diagnostic.spans?.find((candidate) => candidate.is_primary);
      const match = diagnostic.message.match(/cognitive complexity of \(?(\d+)/);
      if (!span || !match) throw new Error(`无法解析 Clippy 复杂度诊断：${diagnostic.message}`);
      const absolute = resolve(repositoryRoot, span.file_name);
      const path = relative(repositoryRoot, absolute).replaceAll("\\", "/");
      diagnostics.push({ path, line: span.line_start, value: Number(match[1]) });
    }
  }
  return diagnostics;
}

function compareDebt(label, current, accepted) {
  const errors = [];
  for (const [key, value] of Object.entries(current)) {
    const acceptedValue = accepted[key];
    if (acceptedValue === undefined) errors.push(`${label}新增超限：${key} = ${value}`);
    else if (value > acceptedValue) errors.push(`${label}债务扩大：${key} ${acceptedValue} -> ${value}`);
    else if (value < acceptedValue) errors.push(`${label}已下降：${key} ${acceptedValue} -> ${value}，请同步收紧 baseline`);
  }
  for (const key of Object.keys(accepted)) {
    if (current[key] === undefined) errors.push(`${label}已回到硬上限内：${key}，请删除 baseline 条目`);
  }
  return errors;
}

function sortRecord(record) {
  const sorted = Object.fromEntries(Object.entries(record).sort(([left], [right]) => left.localeCompare(right)));
  for (const key of Object.keys(record)) delete record[key];
  Object.assign(record, sorted);
}

function runSelfTests() {
  const fixture = [
    "fn first() {",
    "  let ignored = \"fn fake() { }\";",
    "  // fn commented() { }",
    "  if true { println!(\"ok\"); }",
    "}",
    "fn declaration();",
    "fn second<T>()",
    "where",
    "  T: Default,",
    "{",
    "  let raw = r##\"} fn also_fake() {\"##;",
    "}",
    "",
  ].join("\n");
  const functions = parseRustFunctions("fixture.rs", fixture);
  assert.deepEqual(
    functions.map(({ key, startLine, endLine, lines }) => ({ key, startLine, endLine, lines })),
    [
      { key: "fixture.rs::first#1", startLine: 1, endLine: 5, lines: 5 },
      { key: "fixture.rs::second#1", startLine: 7, endLine: 12, lines: 6 },
    ],
  );
  assert.deepEqual(compareDebt("测试", { item: 11 }, { item: 10 }), [
    "测试债务扩大：item 10 -> 11",
  ]);
  assert.deepEqual(compareDebt("测试", { item: 9 }, { item: 10 }), [
    "测试已下降：item 10 -> 9，请同步收紧 baseline",
  ]);
}

function run(command, args, options = {}) {
  const result = spawnSync(command, args, {
    cwd: repositoryRoot,
    encoding: "utf8",
    maxBuffer: options.maxBuffer ?? 16 * 1024 * 1024,
    env: { ...process.env, CARGO_TERM_COLOR: "never" },
  });
  if (result.status !== 0) {
    if (!options.suppressStdoutOnFailure) process.stderr.write(result.stdout ?? "");
    process.stderr.write(result.stderr ?? "");
    throw new Error(`${command} ${args.join(" ")} 执行失败（退出码 ${result.status}）`);
  }
  return result;
}
