import { createReadStream } from "node:fs";
import { stat } from "node:fs/promises";
import { request as httpRequest } from "node:http";
import { request as httpsRequest } from "node:https";
import { randomBytes } from "node:crypto";

const defaultInactivityTimeoutMs = 5 * 60 * 1_000;
const minimumTotalTimeoutMs = 20 * 60 * 1_000;
const maximumTotalTimeoutMs = 3 * 60 * 60 * 1_000;
const minimumSustainedUploadBytesPerSecond = 10_000;
const defaultResponseTimeoutMs = 2 * 60 * 1_000;
const maximumResponseBytes = 1_000_000;

export async function uploadMultipartFile({
  url: inputUrl,
  headers = {},
  filePath,
  fileName,
  contentType,
  inactivityTimeoutMs = defaultInactivityTimeoutMs,
  totalTimeoutMs,
  responseTimeoutMs = defaultResponseTimeoutMs,
}) {
  const url = new URL(inputUrl);
  const requestImpl = requestImplementation(url);
  const boundary = `----bridge-agent-${randomBytes(16).toString("hex")}`;
  const safeFileName = multipartFileName(fileName);
  const prefix = Buffer.from(
    [
      `--${boundary}`,
      `Content-Disposition: form-data; name="file"; filename="${safeFileName}"`,
      `Content-Type: ${contentType}`,
      "",
      "",
    ].join("\r\n"),
  );
  const suffix = Buffer.from(`\r\n--${boundary}--\r\n`);
  const { size } = await stat(filePath);
  const resolvedTotalTimeoutMs =
    totalTimeoutMs ?? defaultTotalTimeoutForSize(size, responseTimeoutMs);

  let totalTimeout;
  let responseTimeout;
  const responsePromise = new Promise((resolve, reject) => {
    const request = requestImpl(
      url,
      {
        method: "POST",
        headers: {
          ...headers,
          "content-type": `multipart/form-data; boundary=${boundary}`,
          "content-length": String(prefix.length + size + suffix.length),
        },
      },
      (response) => {
        const chunks = [];
        let responseBytes = 0;
        response.on("data", (chunk) => {
          responseBytes += chunk.length;
          if (responseBytes > maximumResponseBytes) {
            response.destroy(
              new Error(
                `Upload response exceeded ${maximumResponseBytes} bytes`,
              ),
            );
            return;
          }
          chunks.push(chunk);
        });
        response.once("end", () => {
          clearTimeout(responseTimeout);
          const body = Buffer.concat(chunks).toString("utf8");
          resolve({
            ok: response.statusCode >= 200 && response.statusCode < 300,
            status: response.statusCode,
            text: async () => body,
          });
        });
        response.once("error", reject);
      },
    );

    request.once("finish", () => {
      responseTimeout = setTimeout(() => {
        request.destroy(
          new Error(
            `Gitee upload response exceeded ${responseTimeoutMs} ms after the request body finished`,
          ),
        );
      }, responseTimeoutMs);
    });
    request.setTimeout(inactivityTimeoutMs, () => {
      request.destroy(
        new Error(
          `Gitee upload had no network activity for ${inactivityTimeoutMs} ms`,
        ),
      );
    });
    totalTimeout = setTimeout(() => {
      request.destroy(
        new Error(
            `Gitee upload exceeded total timeout of ${resolvedTotalTimeoutMs} ms`,
          ),
        );
    }, resolvedTotalTimeoutMs);
    request.once("error", reject);

    request.write(prefix);
    const file = createReadStream(filePath);
    file.once("error", (error) => request.destroy(error));
    file.once("end", () => request.end(suffix));
    file.pipe(request, { end: false });
  });

  return responsePromise.finally(() => {
    clearTimeout(totalTimeout);
    clearTimeout(responseTimeout);
  });
}

export function defaultTotalTimeoutForSize(
  size,
  responseTimeoutMs = defaultResponseTimeoutMs,
) {
  if (!Number.isSafeInteger(size) || size < 0) {
    throw new Error(`Invalid upload size: ${size}`);
  }
  const transferBudgetMs = Math.ceil(
    (size / minimumSustainedUploadBytesPerSecond) * 1_000,
  );
  return Math.min(
    maximumTotalTimeoutMs,
    Math.max(
      minimumTotalTimeoutMs,
      transferBudgetMs + responseTimeoutMs,
    ),
  );
}

function requestImplementation(url) {
  if (url.protocol === "https:") return httpsRequest;
  if (url.protocol === "http:") return httpRequest;
  throw new Error(`Unsupported multipart upload protocol: ${url.protocol}`);
}

function multipartFileName(value) {
  if (!value || /[\0\r\n"\\/]/.test(value)) {
    throw new Error(`Unsafe multipart file name: ${JSON.stringify(value)}`);
  }
  return value;
}
