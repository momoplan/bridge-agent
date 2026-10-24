import { expect, it } from "vitest";
import { readError, isCommandError } from "./formatters";
it("preserves structured consumer diagnostics through the desktop command boundary", () => {
  const error = { code: "consumer_request_failed", failure: {
    message: "依赖服务暂不可用", errorCode: "TEST_UNAVAILABLE", retryable: true,
    requestId: "request-1", traceId: "trace-1", status: 200, details: '{"field":"version"}'
  }};
  expect(isCommandError(error)).toBe(true);
  const text = readError(error);
  for (const field of ["依赖服务暂不可用", "TEST_UNAVAILABLE", "可稍后重试", "request-1", "trace-1", '"version"']) {
    expect(text).toContain(field);
  }
  expect(isCommandError({ ...error, failure: { ...error.failure, details: {} } })).toBe(false);
});
