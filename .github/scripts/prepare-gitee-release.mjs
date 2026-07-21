const [tagName, version] = process.argv.slice(2);
if (!tagName || !version || tagName !== `bridge-agent-v${version}`) {
  throw new Error("Usage: prepare-gitee-release.mjs <bridge-agent-vVERSION> <VERSION>");
}

const token = process.env.GITEE_ACCESS_TOKEN?.trim();
if (!token) {
  throw new Error("Missing GITEE_ACCESS_TOKEN");
}

const apiBase = "https://gitee.com/api/v5";
const owner = "zxflimit_admin";
const repository = "bridge-agent";
const retainedReleaseCount = 5;

let currentRelease;
try {
  currentRelease = await request(
    `/repos/${owner}/${repository}/releases/tags/${encodeURIComponent(tagName)}`,
  );
} catch (error) {
  if (error.status !== 404) throw error;
  currentRelease = await request(`/repos/${owner}/${repository}/releases`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({
      tag_name: tagName,
      name: `百积木 ${tagName}`,
      body: [
        "百积木桌面端国内镜像发布。",
        "",
        "安装包由 GitHub Actions 从同一 Git tag 构建、签名并同步到此 Release。",
      ].join("\n"),
      prerelease: /-(alpha|beta|rc)/.test(version),
    }),
  });
  console.log(`Created Gitee Release ${tagName}`);
}

if (!currentRelease?.id) {
  throw new Error(`Gitee Release ${tagName} has no id`);
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
    console.log(`Deleted old Gitee Release ${release.tag_name}; Git tag was preserved`);
  }
}

console.log(`Gitee Release ${tagName} is ready (retaining latest ${retainedReleaseCount})`);

async function request(path, options = {}) {
  let lastError;
  for (let attempt = 1; attempt <= 4; attempt += 1) {
    try {
      const url = new URL(`${apiBase}${path}`);
      const response = await fetch(url, {
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
}

function releaseTime(release) {
  return Date.parse(release.created_at ?? release.published_at ?? 0) || 0;
}
