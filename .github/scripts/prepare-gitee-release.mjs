import { pathToFileURL } from "node:url";

export async function prepareGiteeRelease({
  tagName,
  version,
  targetCommitish,
  token,
  apiBase = "https://gitee.com/api/v5",
  owner = "zxflimit_admin",
  repository = "bridge-agent",
  retainedReleaseCount = 5,
  fetchImpl = fetch,
  logger = console,
}) {
  if (!tagName || !version || tagName !== `bridge-agent-v${version}`) {
    throw new Error("Usage: prepare-gitee-release.mjs <bridge-agent-vVERSION> <VERSION>");
  }
  if (!/^[0-9a-f]{40}$/.test(targetCommitish ?? "")) {
    throw new Error("GITHUB_SHA must be the exact 40-character release commit");
  }
  if (!token?.trim()) {
    throw new Error("Missing GITEE_ACCESS_TOKEN");
  }

  const request = createRequester({ apiBase, token: token.trim(), fetchImpl });
  let currentRelease;
  try {
    currentRelease = await request(
      `/repos/${owner}/${repository}/releases/tags/${encodeURIComponent(tagName)}`,
    );
  } catch (error) {
    if (error.status !== 404) throw error;
    currentRelease = null;
  }

  // Gitee currently returns HTTP 200 with a JSON null body when a release tag
  // does not exist. Keep supporting HTTP 404 as well so both API behaviours
  // resolve to the same explicit create operation.
  if (currentRelease === null) {
    currentRelease = await request(`/repos/${owner}/${repository}/releases`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({
        tag_name: tagName,
        // Gitee requires an explicit branch or commit SHA even when tag_name
        // already exists. Pin the Release to the workflow's exact source SHA.
        target_commitish: targetCommitish,
        name: `百积木 ${tagName}`,
        body: [
          "百积木桌面端国内镜像发布。",
          "",
          "安装包由 GitHub Actions 从同一 Git tag 构建、签名并同步到此 Release。",
        ].join("\n"),
        prerelease: /-(alpha|beta|rc)/.test(version),
      }),
    });
    logger.log(`Created Gitee Release ${tagName}`);
  }

  if (!currentRelease?.id) {
    throw new Error(
      `Gitee Release ${tagName} has no id: ${summarizeUnexpectedRelease(currentRelease)}`,
    );
  }

  const releases = await request(`/repos/${owner}/${repository}/releases?page=1&per_page=100`);
  const managed = releases
    .filter((release) => release.tag_name?.startsWith("bridge-agent-v"))
    .sort((left, right) => releaseTime(right) - releaseTime(left));
  const keepIds = new Set(
    [currentRelease, ...managed.filter((release) => release.id !== currentRelease.id)]
      .slice(0, retainedReleaseCount)
      .map((release) => release.id),
  );
  for (const release of managed) {
    if (!keepIds.has(release.id)) {
      await request(`/repos/${owner}/${repository}/releases/${release.id}`, { method: "DELETE" });
      logger.log(`Deleted old Gitee Release ${release.tag_name}; Git tag was preserved`);
    }
  }

  logger.log(`Gitee Release ${tagName} is ready (retaining latest ${retainedReleaseCount})`);
  return currentRelease;
}

function createRequester({ apiBase, token, fetchImpl }) {
  return async function request(path, options = {}) {
    let lastError;
    for (let attempt = 1; attempt <= 4; attempt += 1) {
      try {
        const url = new URL(`${apiBase}${path}`);
        const response = await fetchImpl(url, {
          ...options,
          headers: {
            Authorization: `Bearer ${token}`,
            ...options.headers,
          },
        });
        const body = await response.text();
        if (!response.ok) {
          const error = new Error(
            `Gitee API ${options.method ?? "GET"} ${url.pathname} failed: ${response.status} ${body.slice(0, 500)}`,
          );
          error.status = response.status;
          throw error;
        }
        return body.trim() ? JSON.parse(body) : {};
      } catch (error) {
        lastError = error;
        if (error.status === 404 || attempt === 4) throw error;
        await new Promise((resolve) => setTimeout(resolve, attempt * 5_000));
      }
    }
    throw lastError;
  };
}

function releaseTime(release) {
  return Date.parse(release.created_at ?? release.published_at ?? 0) || 0;
}

function summarizeUnexpectedRelease(value) {
  if (value === null) return "null response";
  if (Array.isArray(value)) return `array response (${value.length} items)`;
  if (typeof value !== "object") return `${typeof value} response`;
  const summary = {
    keys: Object.keys(value).sort(),
    message: typeof value.message === "string" ? value.message : undefined,
    errors: Array.isArray(value.errors) ? value.errors : undefined,
  };
  return JSON.stringify(summary);
}

async function main() {
  const [tagName, version] = process.argv.slice(2);
  await prepareGiteeRelease({
    tagName,
    version,
    targetCommitish: process.env.GITHUB_SHA,
    token: process.env.GITEE_ACCESS_TOKEN,
  });
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  await main();
}
