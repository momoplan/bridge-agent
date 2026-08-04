import { describe, expect, it } from "vitest";
import {
  describeLocalAppUpdate,
  type InstalledAppShape,
  type MarketAppShape
} from "./local-app-updates";

const method = (name: string, inputSchema: unknown = { type: "object" }) => ({
  name,
  description: `${name} description`,
  inputSchema,
  responseMode: "cmodel",
  path: `/invoke/${name}`,
  httpMethod: "POST"
});

const installed = (overrides: Partial<InstalledAppShape> = {}): InstalledAppShape => ({
  configSchema: { type: "object", properties: {} },
  methods: [method("message.list")],
  events: [],
  database: { engine: "sqlite", schemaVersion: "1", migrations: [] },
  permissions: [],
  ...overrides
});

const market = (overrides: Partial<MarketAppShape> = {}): MarketAppShape => ({
  releaseNotes: [],
  configurationDeclaration: "declared",
  interfaceDeclaration: "declared",
  databaseDeclaration: "declared",
  configSchema: { type: "object", properties: {} },
  methods: [method("message.list")],
  events: [],
  database: { engine: "sqlite", schemaVersion: "1", migrations: [] },
  permissions: [],
  ...overrides
});

describe("describeLocalAppUpdate", () => {
  it("classifies required configuration and interface removals as breaking", () => {
    const result = describeLocalAppUpdate(
      installed({
        configSchema: {
          type: "object",
          properties: { endpoint: { type: "string" } }
        },
        methods: [method("message.list"), method("message.send")]
      }),
      market({
        configSchema: {
          type: "object",
          required: ["token"],
          properties: {
            endpoint: { type: "string" },
            token: { type: "string" }
          }
        }
      })
    );

    expect(result.highestRisk).toBe("breaking");
    expect(result.sections.find((section) => section.id === "configuration")?.changes).toEqual(
      expect.arrayContaining([expect.objectContaining({ title: "新增必填配置：token", tone: "breaking" })])
    );
    expect(result.sections.find((section) => section.id === "interfaces")?.changes).toEqual(
      expect.arrayContaining([expect.objectContaining({ title: "移除方法：message.send", tone: "breaking" })])
    );
  });

  it("reports endpoint, request schema and event payload changes", () => {
    const result = describeLocalAppUpdate(
      installed({
        methods: [method("message.list")],
        events: [{ name: "message.received", description: "old", payloadSchema: { type: "object" } }]
      }),
      market({
        methods: [{ ...method("message.list", { type: "object", required: ["cursor"], properties: { cursor: { type: "string" } } }), path: "/v2/messages" }],
        events: [{ name: "message.received", description: "new", payloadSchema: { type: "object", properties: { id: { type: "string" } } } }]
      })
    );

    const changes = result.sections.find((section) => section.id === "interfaces")?.changes ?? [];
    expect(changes).toEqual(expect.arrayContaining([
      expect.objectContaining({ title: "方法契约变化：message.list", tone: "breaking" }),
      expect.objectContaining({ title: "事件契约变化：message.received", tone: "attention" })
    ]));
  });

  it("resolves a multi-step database migration path and exposes rollback and downtime", () => {
    const result = describeLocalAppUpdate(
      installed(),
      market({
        database: {
          engine: "sqlite",
          schemaVersion: "3",
          migrations: [
            {
              id: "001-add-status",
              fromVersion: "1",
              toVersion: "2",
              description: "新增消息状态",
              changes: [{ operation: "add_column", target: "messages.status", description: "新增状态字段", destructive: false }],
              destructive: false,
              rollback: "automatic",
              downtime: "none"
            },
            {
              id: "002-rebuild-index",
              fromVersion: "2",
              toVersion: "3",
              description: "重建消息索引",
              changes: [{ operation: "replace_index", target: "messages.idx_time", description: "替换时间索引", destructive: false }],
              destructive: false,
              rollback: "manual",
              downtime: "brief"
            }
          ]
        }
      })
    );

    const database = result.sections.find((section) => section.id === "database");
    expect(database?.changes).toHaveLength(2);
    expect(database?.changes[1]).toEqual(expect.objectContaining({ tone: "attention" }));
    expect(database?.changes[1].detail).toContain("回滚：人工；停机：短暂停机");
  });

  it("blocks an upgrade review when the database migration chain is incomplete", () => {
    const result = describeLocalAppUpdate(
      installed(),
      market({ database: { engine: "sqlite", schemaVersion: "3", migrations: [] } })
    );

    expect(result.highestRisk).toBe("breaking");
    expect(result.sections.find((section) => section.id === "database")?.changes[0]).toEqual(
      expect.objectContaining({ title: "缺少数据库迁移路径：1 → 3", tone: "breaking" })
    );
  });

  it("marks missing target contracts as undeclared instead of claiming there are no changes", () => {
    const result = describeLocalAppUpdate(
      installed(),
      market({
        configurationDeclaration: "undeclared",
        interfaceDeclaration: "undeclared",
        databaseDeclaration: "undeclared",
        configSchema: null,
        methods: [],
        events: [],
        database: null
      })
    );

    expect(result.highestRisk).toBe("unknown");
    expect(result.undeclaredSections).toEqual(["配置变化", "接口变化", "数据库变化"]);
    expect(result.sections.filter((section) => !section.declared).every((section) => !section.unchanged)).toBe(true);
  });

  it("distinguishes an explicit not-applicable declaration from missing metadata", () => {
    const result = describeLocalAppUpdate(
      null,
      market({
        configurationDeclaration: "not_applicable",
        interfaceDeclaration: "not_applicable",
        databaseDeclaration: "not_applicable",
        configSchema: null,
        methods: [],
        events: [],
        database: null
      })
    );

    expect(result.undeclaredSections).toEqual([]);
    expect(result.sections.slice(0, 3).every((section) => !section.applicable && section.unchanged)).toBe(true);
  });

  it("keeps release notes and permission changes as supporting review information", () => {
    const result = describeLocalAppUpdate(
      installed(),
      market({
        releaseNotes: ["修复重连", "修复重连", ""],
        permissions: [{ id: "filesystem", title: "文件读取" }]
      })
    );

    expect(result.releaseNotes).toEqual(["修复重连"]);
    expect(result.sections.find((section) => section.id === "permissions")?.changes).toEqual([
      expect.objectContaining({ title: "新增权限：文件读取", tone: "attention" })
    ]);
  });
});
