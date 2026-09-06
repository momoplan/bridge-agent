import type { UpdateEventContract, UpdateMethodContract } from "../local-app-updates";
export function legacyMethodContracts(names: string[]): UpdateMethodContract[] {
  return names.map((name) => ({
    name,
    description: "",
    inputSchema: { type: "object" },
    responseMode: "cmodel",
    path: "",
    httpMethod: "POST"
  }));
}

export function legacyEventContracts(names: string[]): UpdateEventContract[] {
  return names.map((name) => ({
    name,
    description: "",
    payloadSchema: { type: "object" }
  }));
}
