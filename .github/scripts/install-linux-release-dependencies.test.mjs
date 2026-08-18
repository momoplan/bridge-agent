import {
  chmodSync,
  mkdtempSync,
  mkdirSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { spawnSync } from "node:child_process";

import { afterEach, describe, expect, test } from "vitest";

const scriptPath = ".github/scripts/install-linux-release-dependencies.sh";
const temporaryDirectories = [];

afterEach(() => {
  while (temporaryDirectories.length > 0) {
    rmSync(temporaryDirectories.pop(), { force: true, recursive: true });
  }
});

function createExecutable(path, contents) {
  writeFileSync(path, contents);
  chmodSync(path, 0o755);
}

describe("Linux release dependency installation", () => {
  test.runIf(process.platform !== "win32")(
    "replaces the unavailable runner mirror and bounds every apt command",
    () => {
      const fixtureRoot = mkdtempSync(join(tmpdir(), "bridge-agent-apt-test-"));
      temporaryDirectories.push(fixtureRoot);
      const aptRoot = join(fixtureRoot, "apt");
      const binRoot = join(fixtureRoot, "bin");
      const commandLog = join(fixtureRoot, "commands.log");
      mkdirSync(join(aptRoot, "sources.list.d"), { recursive: true });
      mkdirSync(binRoot, { recursive: true });

      const mirrorList = join(aptRoot, "apt-mirrors.txt");
      writeFileSync(
        mirrorList,
        [
          "http://azure.archive.ubuntu.com/ubuntu",
          "https://archive.ubuntu.com/ubuntu",
          "",
        ].join("\n"),
      );
      writeFileSync(
        join(aptRoot, "sources.list"),
        "deb http://azure.archive.ubuntu.com/ubuntu jammy main\n",
      );

      createExecutable(
        join(binRoot, "timeout"),
        [
          "#!/usr/bin/env bash",
          'printf \'timeout %s\\n\' "$*" >> "$APT_TEST_COMMAND_LOG"',
          "shift 2",
          'exec "$@"',
          "",
        ].join("\n"),
      );
      createExecutable(
        join(binRoot, "apt-get"),
        [
          "#!/usr/bin/env bash",
          'printf \'apt-get %s\\n\' "$*" >> "$APT_TEST_COMMAND_LOG"',
          "",
        ].join("\n"),
      );

      const result = spawnSync("bash", [scriptPath], {
        encoding: "utf8",
        env: {
          ...process.env,
          APT_ACQUIRE_RETRIES: "7",
          APT_COMMAND_TIMEOUT_SECONDS: "42",
          APT_GET_COMMAND: join(binRoot, "apt-get"),
          APT_SOURCES_ROOT: aptRoot,
          APT_SUDO_COMMAND: "",
          APT_TEST_COMMAND_LOG: commandLog,
          APT_TIMEOUT_COMMAND: join(binRoot, "timeout"),
        },
      });

      expect(result.status, result.stderr).toBe(0);
      expect(readFileSync(mirrorList, "utf8")).not.toContain(
        "azure.archive.ubuntu.com",
      );
      expect(readFileSync(join(aptRoot, "sources.list"), "utf8")).toContain(
        "https://archive.ubuntu.com/ubuntu",
      );

      const commands = readFileSync(commandLog, "utf8");
      expect(commands).toContain("--signal=TERM 42s");
      expect(commands.match(/Acquire::Retries=7/g)).toHaveLength(4);
      expect(commands).toContain("apt-get -o Acquire::Retries=7");
      expect(commands).toContain(" update");
      expect(commands).toContain(" install -y --no-install-recommends");
    },
  );
});
