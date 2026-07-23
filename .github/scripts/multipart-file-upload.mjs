import { createReadStream } from "node:fs";
import { stat } from "node:fs/promises";
import { request as httpRequest } from "node:http";
import { request as httpsRequest } from "node:https";
import { randomBytes } from "node:crypto";

const defaultInactivityTimeoutMs = 30 * 60 * 1_000;
const maximumResponseBytes = 1_000_000;

export async function uploadMultipartFile({
  url: inputUrl,
  headers = {},
  filePath,
  fileName,
  contentType,
  inactivityTimeoutMs = defaultInactivityTimeoutMs,
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
              new Error(`Upload response exceeded ${maximumResponseBytes} bytes`),
            );
            return;
          }
          chunks.push(chunk);
        });
        response.once("end", () => {
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

    request.setTimeout(inactivityTimeoutMs, () => {
      request.destroy(
        new Error(`Gitee upload had no network activity for ${inactivityTimeoutMs} ms`),
      );
    });
    request.once("error", reject);

    request.write(prefix);
    const file = createReadStream(filePath);
    file.once("error", (error) => request.destroy(error));
    file.once("end", () => request.end(suffix));
    file.pipe(request, { end: false });
  });

  return responsePromise;
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
