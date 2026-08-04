export type UpdateChangeTone = "compatible" | "attention" | "breaking";
export type UpdateContractDeclaration = "declared" | "not_applicable" | "undeclared";

export interface UpdatePermission {
  id: string;
  title: string;
}

export interface UpdateMethodContract {
  name: string;
  description: string;
  inputSchema: unknown;
  responseMode: string;
  path: string;
  httpMethod: string;
}

export interface UpdateEventContract {
  name: string;
  description: string;
  payloadSchema: unknown;
}

export interface UpdateDatabaseChange {
  operation: string;
  target: string;
  description: string;
  destructive: boolean;
}

export interface UpdateDatabaseMigration {
  id: string;
  fromVersion: string;
  toVersion: string;
  description: string;
  changes: UpdateDatabaseChange[];
  destructive: boolean;
  rollback: "automatic" | "manual" | "unsupported" | "not_declared" | string;
  downtime: "none" | "brief" | "required" | "not_declared" | string;
}

export interface UpdateDatabaseContract {
  engine: string;
  schemaVersion: string;
  migrations: UpdateDatabaseMigration[];
}

export interface InstalledAppShape {
  configSchema: unknown | null;
  methods: UpdateMethodContract[];
  events: UpdateEventContract[];
  database: UpdateDatabaseContract | null;
  permissions: UpdatePermission[];
}

export interface MarketAppShape extends InstalledAppShape {
  releaseNotes: string[];
  configurationDeclaration: UpdateContractDeclaration;
  interfaceDeclaration: UpdateContractDeclaration;
  databaseDeclaration: UpdateContractDeclaration;
}

export interface UpdateContractChange {
  id: string;
  tone: UpdateChangeTone;
  title: string;
  detail?: string;
}

export interface UpdateContractSection {
  id: "configuration" | "interfaces" | "database" | "permissions";
  title: string;
  declared: boolean;
  applicable: boolean;
  unchanged: boolean;
  emptyMessage: string;
  changes: UpdateContractChange[];
}

export interface LocalAppUpdateChanges {
  releaseNotes: string[];
  sections: UpdateContractSection[];
  highestRisk: UpdateChangeTone | "unknown";
  hasSpecificChanges: boolean;
  undeclaredSections: string[];
}

interface FlatConfigField {
  path: string;
  type: string;
  required: boolean;
  defaultValue: unknown;
  enumValues: unknown[] | null;
}

export function describeLocalAppUpdate(
  installed: InstalledAppShape | null,
  market: MarketAppShape
): LocalAppUpdateChanges {
  const sections = [
    describeConfigurationChanges(installed?.configSchema ?? null, market.configSchema, market.configurationDeclaration),
    describeInterfaceChanges(installed, market, market.interfaceDeclaration),
    describeDatabaseChanges(installed?.database ?? null, market.database, market.databaseDeclaration),
    describePermissionChanges(installed?.permissions ?? [], market.permissions)
  ];
  const releaseNotes = uniqueNonEmpty(market.releaseNotes);
  const tones = sections.flatMap((section) => section.changes.map((change) => change.tone));
  const undeclaredSections = sections
    .filter((section) => section.applicable && !section.declared && section.id !== "permissions")
    .map((section) => section.title);

  return {
    releaseNotes,
    sections,
    highestRisk: tones.includes("breaking")
      ? "breaking"
      : tones.includes("attention")
        ? "attention"
        : tones.includes("compatible")
          ? "compatible"
          : undeclaredSections.length > 0
            ? "unknown"
            : "compatible",
    hasSpecificChanges: releaseNotes.length > 0 || tones.length > 0,
    undeclaredSections
  };
}

function describeConfigurationChanges(
  currentSchema: unknown | null,
  targetSchema: unknown | null,
  declaration: UpdateContractDeclaration
): UpdateContractSection {
  if (declaration === "not_applicable") {
    return section("configuration", "配置变化", true, [], "该 package 明确声明不涉及运行配置。", false);
  }
  if (!isRecord(targetSchema)) {
    if (declaration === "declared") {
      return section("configuration", "配置变化", false, [{
        id: "config:invalid-declaration",
        tone: "breaking",
        title: "配置契约声明不完整",
        detail: "upgradeReview.configuration 标记为 declared，但目标清单缺少 configSchema。"
      }], "目标版本的配置契约声明无效。");
    }
    return section("configuration", "配置变化", false, [], "目标版本未声明 configSchema，无法判断配置变化。");
  }
  const targetFields = flattenConfigSchema(targetSchema);
  if (!isRecord(currentSchema)) {
    const changes = targetFields.map((field) => ({
      id: `config:add:${field.path}`,
      tone: field.required ? "attention" as const : "compatible" as const,
      title: `${field.required ? "新增必填配置" : "新增可选配置"}：${field.path}`,
      detail: configFieldDetail(field)
    }));
    return section(
      "configuration",
      "配置变化",
      true,
      changes.length > 0
        ? changes
        : [{
            id: "config:first-contract",
            tone: "attention",
            title: "目标版本首次声明配置契约",
            detail: "当前安装版本没有 configSchema，无法验证历史配置兼容性。"
          }],
      "配置契约未发生变化。"
    );
  }

  const currentFields = fieldMap(flattenConfigSchema(currentSchema));
  const targetFieldMap = fieldMap(targetFields);
  const changes: UpdateContractChange[] = [];
  for (const [path, target] of targetFieldMap) {
    const current = currentFields.get(path);
    if (!current) {
      changes.push({
        id: `config:add:${path}`,
        tone: target.required ? "breaking" : "compatible",
        title: `${target.required ? "新增必填配置" : "新增可选配置"}：${path}`,
        detail: configFieldDetail(target)
      });
      continue;
    }
    const details: string[] = [];
    let tone: UpdateChangeTone = "compatible";
    if (current.type !== target.type) {
      tone = "breaking";
      details.push(`类型 ${current.type} → ${target.type}`);
    }
    if (!current.required && target.required) {
      tone = "breaking";
      details.push("由可选改为必填");
    } else if (current.required && !target.required) {
      details.push("由必填改为可选");
    }
    if (!sameJson(current.defaultValue, target.defaultValue)) {
      if (tone !== "breaking") tone = "attention";
      details.push(`默认值 ${formatValue(current.defaultValue)} → ${formatValue(target.defaultValue)}`);
    }
    if (!sameJson(current.enumValues, target.enumValues)) {
      if (enumNarrowed(current.enumValues, target.enumValues)) tone = "breaking";
      else if (tone !== "breaking") tone = "attention";
      details.push(`可选值 ${formatValue(current.enumValues)} → ${formatValue(target.enumValues)}`);
    }
    if (details.length > 0) {
      changes.push({ id: `config:change:${path}`, tone, title: `配置契约变化：${path}`, detail: details.join("；") });
    }
  }
  for (const [path, current] of currentFields) {
    if (!targetFieldMap.has(path)) {
      changes.push({
        id: `config:remove:${path}`,
        tone: "breaking",
        title: `移除配置：${path}`,
        detail: configFieldDetail(current)
      });
    }
  }
  return section("configuration", "配置变化", true, changes, "配置契约未发生变化。");
}

function describeInterfaceChanges(
  installed: InstalledAppShape | null,
  market: MarketAppShape,
  declaration: UpdateContractDeclaration
): UpdateContractSection {
  if (declaration === "not_applicable") {
    return section("interfaces", "接口变化", true, [], "该 package 明确声明不提供可调用接口或事件。", false);
  }
  const declared = market.methods.length > 0 || market.events.length > 0;
  if (!declared) {
    if (declaration === "declared") {
      return section("interfaces", "接口变化", false, [{
        id: "api:invalid-declaration",
        tone: "breaking",
        title: "接口契约声明不完整",
        detail: "upgradeReview.interfaces 标记为 declared，但目标清单没有 methods/events。"
      }], "目标版本的接口契约声明无效。");
    }
    return section("interfaces", "接口变化", false, [], "目标版本未声明 methods/events 契约，无法判断接口变化。");
  }
  if (!installed) {
    return section("interfaces", "接口变化", true, [], "当前版本接口契约不可用，无法计算版本差异。");
  }
  const changes: UpdateContractChange[] = [];
  diffNamedContracts(
    installed.methods,
    market.methods,
    "方法",
    (method) => method.name,
    compareMethodContract,
    changes
  );
  diffNamedContracts(
    installed.events,
    market.events,
    "事件",
    (event) => event.name,
    compareEventContract,
    changes
  );
  return section("interfaces", "接口变化", true, changes, "方法、事件及其数据契约未发生变化。");
}

function compareMethodContract(current: UpdateMethodContract, target: UpdateMethodContract): Omit<UpdateContractChange, "id" | "title"> | null {
  const details: string[] = [];
  let tone: UpdateChangeTone = "compatible";
  if (current.httpMethod !== target.httpMethod || current.path !== target.path) {
    tone = "breaking";
    details.push(`端点 ${current.httpMethod} ${current.path || "-"} → ${target.httpMethod} ${target.path || "-"}`);
  }
  if (current.responseMode !== target.responseMode) {
    tone = "breaking";
    details.push(`响应模式 ${current.responseMode} → ${target.responseMode}`);
  }
  if (!sameJson(current.inputSchema, target.inputSchema)) {
    const schemaChanges = describeSchemaContractChanges(current.inputSchema, target.inputSchema, "请求参数");
    if (schemaChanges.tone === "breaking" || tone !== "breaking") tone = schemaChanges.tone;
    details.push(...schemaChanges.details);
  }
  if (current.description.trim() !== target.description.trim()) {
    if (tone === "compatible") tone = "attention";
    details.push("接口说明已更新");
  }
  return details.length > 0 ? { tone, detail: details.join("；") } : null;
}

function compareEventContract(current: UpdateEventContract, target: UpdateEventContract): Omit<UpdateContractChange, "id" | "title"> | null {
  const details: string[] = [];
  let tone: UpdateChangeTone = "compatible";
  if (!sameJson(current.payloadSchema, target.payloadSchema)) {
    const schemaChanges = describeSchemaContractChanges(current.payloadSchema, target.payloadSchema, "事件载荷");
    tone = schemaChanges.tone;
    details.push(...schemaChanges.details);
  }
  if (current.description.trim() !== target.description.trim()) {
    if (tone === "compatible") tone = "attention";
    details.push("事件说明已更新");
  }
  return details.length > 0 ? { tone, detail: details.join("；") } : null;
}

function diffNamedContracts<T>(
  currentValues: T[],
  targetValues: T[],
  label: string,
  nameOf: (value: T) => string,
  compare: (current: T, target: T) => Omit<UpdateContractChange, "id" | "title"> | null,
  changes: UpdateContractChange[]
) {
  const current = new Map(currentValues.map((value) => [nameOf(value), value]));
  const target = new Map(targetValues.map((value) => [nameOf(value), value]));
  for (const [name, value] of target) {
    const previous = current.get(name);
    if (!previous) {
      changes.push({ id: `api:add:${label}:${name}`, tone: "compatible", title: `新增${label}：${name}` });
      continue;
    }
    const change = compare(previous, value);
    if (change) changes.push({ id: `api:change:${label}:${name}`, title: `${label}契约变化：${name}`, ...change });
  }
  for (const name of current.keys()) {
    if (!target.has(name)) {
      changes.push({ id: `api:remove:${label}:${name}`, tone: "breaking", title: `移除${label}：${name}` });
    }
  }
}

function describeDatabaseChanges(
  current: UpdateDatabaseContract | null,
  target: UpdateDatabaseContract | null,
  declaration: UpdateContractDeclaration
): UpdateContractSection {
  if (declaration === "not_applicable") {
    return section("database", "数据库变化", true, [], "该 package 明确声明不使用持久化数据库。", false);
  }
  if (!target) {
    if (declaration === "declared") {
      return section("database", "数据库变化", false, [{
        id: "database:invalid-declaration",
        tone: "breaking",
        title: "数据库契约声明不完整",
        detail: "upgradeReview.database 标记为 declared，但目标清单缺少 database。"
      }], "目标版本的数据库契约声明无效。");
    }
    return section("database", "数据库变化", false, [], "目标版本未声明 database migration 契约，无法判断数据库变化。");
  }
  const changes: UpdateContractChange[] = [];
  if (!current) {
    changes.push({
      id: "database:first-contract",
      tone: "attention",
      title: `目标数据库：${target.engine} schema ${target.schemaVersion}`,
      detail: "当前安装版本未声明数据库契约，无法自动确定完整迁移路径。"
    });
    target.migrations.forEach((migration) => changes.push(describeMigration(migration)));
    return section("database", "数据库变化", true, changes, "数据库 Schema 未发生变化。");
  }
  if (current.engine !== target.engine) {
    changes.push({
      id: "database:engine",
      tone: "breaking",
      title: `数据库引擎变化：${current.engine} → ${target.engine}`,
      detail: "需要发布方提供跨引擎迁移和回滚方案。"
    });
  }
  if (current.schemaVersion !== target.schemaVersion) {
    const path = resolveMigrationPath(current.schemaVersion, target.schemaVersion, target.migrations);
    if (!path) {
      changes.push({
        id: "database:path-missing",
        tone: "breaking",
        title: `缺少数据库迁移路径：${current.schemaVersion} → ${target.schemaVersion}`,
        detail: "目标 package 没有声明从当前 Schema 到目标 Schema 的完整 migration 链。"
      });
    } else {
      path.forEach((migration) => changes.push(describeMigration(migration)));
    }
  }
  return section("database", "数据库变化", true, changes, "数据库引擎和 Schema 版本未发生变化。");
}

function resolveMigrationPath(
  fromVersion: string,
  toVersion: string,
  migrations: UpdateDatabaseMigration[]
): UpdateDatabaseMigration[] | null {
  const queue: Array<{ version: string; path: UpdateDatabaseMigration[] }> = [{ version: fromVersion, path: [] }];
  const visited = new Set([fromVersion]);
  while (queue.length > 0) {
    const current = queue.shift()!;
    if (current.version === toVersion) return current.path;
    for (const migration of migrations.filter((candidate) => candidate.fromVersion === current.version)) {
      if (!visited.has(migration.toVersion)) {
        visited.add(migration.toVersion);
        queue.push({ version: migration.toVersion, path: [...current.path, migration] });
      }
    }
  }
  return null;
}

function describeMigration(migration: UpdateDatabaseMigration): UpdateContractChange {
  const destructive = migration.destructive || migration.changes.some((change) => change.destructive);
  const tone: UpdateChangeTone = destructive || migration.rollback === "unsupported" || migration.downtime === "required"
    ? "breaking"
    : "attention";
  const details = migration.changes.map((change) => `${change.operation} ${change.target}：${change.description}`);
  details.push(`回滚：${rollbackLabel(migration.rollback)}；停机：${downtimeLabel(migration.downtime)}`);
  return {
    id: `database:migration:${migration.id}`,
    tone,
    title: `${migration.description}（${migration.fromVersion} → ${migration.toVersion}）`,
    detail: details.join("；")
  };
}

function describePermissionChanges(
  currentPermissions: UpdatePermission[],
  targetPermissions: UpdatePermission[]
): UpdateContractSection {
  const current = permissionMap(currentPermissions);
  const target = permissionMap(targetPermissions);
  const changes: UpdateContractChange[] = [];
  for (const [id, title] of target) {
    if (!current.has(id)) changes.push({ id: `permission:add:${id}`, tone: "attention", title: `新增权限：${title || id}` });
  }
  for (const [id, title] of current) {
    if (!target.has(id)) changes.push({ id: `permission:remove:${id}`, tone: "compatible", title: `移除权限：${title || id}` });
  }
  return section("permissions", "权限变化", true, changes, "权限声明未发生变化。");
}

function section(
  id: UpdateContractSection["id"],
  title: string,
  declared: boolean,
  changes: UpdateContractChange[],
  emptyMessage: string,
  applicable = true
): UpdateContractSection {
  return { id, title, declared, applicable, unchanged: declared && changes.length === 0, emptyMessage, changes };
}

function flattenConfigSchema(schema: Record<string, unknown>, parentPath = ""): FlatConfigField[] {
  const properties = isRecord(schema.properties) ? schema.properties : {};
  const required = new Set(Array.isArray(schema.required) ? schema.required.filter((value): value is string => typeof value === "string") : []);
  const result: FlatConfigField[] = [];
  for (const [name, value] of Object.entries(properties)) {
    if (!isRecord(value)) continue;
    const path = parentPath ? `${parentPath}.${name}` : name;
    result.push({
      path,
      type: schemaType(value),
      required: required.has(name),
      defaultValue: value.default,
      enumValues: Array.isArray(value.enum) ? value.enum : null
    });
    result.push(...flattenConfigSchema(value, path));
  }
  return result;
}

function describeSchemaContractChanges(
  current: unknown,
  target: unknown,
  label: string
): { tone: UpdateChangeTone; details: string[] } {
  if (!isRecord(current) || !isRecord(target)) {
    return { tone: "attention", details: [`${label} Schema 已替换`] };
  }
  const details: string[] = [];
  let tone: UpdateChangeTone = "attention";
  if (schemaType(current) !== schemaType(target)) {
    return {
      tone: "breaking",
      details: [`${label}根类型 ${schemaType(current)} → ${schemaType(target)}`]
    };
  }
  const currentFields = fieldMap(flattenConfigSchema(current));
  const targetFields = fieldMap(flattenConfigSchema(target));
  for (const [path, field] of currentFields) {
    const next = targetFields.get(path);
    if (!next) {
      tone = "breaking";
      details.push(`${label}移除字段 ${path}`);
    } else if (next.type !== field.type) {
      tone = "breaking";
      details.push(`${label}字段 ${path} 类型 ${field.type} → ${next.type}`);
    } else if (field.required && !next.required) {
      details.push(`${label}字段 ${path} 改为可选`);
    } else if (!field.required && next.required) {
      tone = "breaking";
      details.push(`${label}字段 ${path} 改为必填`);
    }
    if (next && !sameJson(field.enumValues, next.enumValues)) {
      if (enumNarrowed(field.enumValues, next.enumValues)) tone = "breaking";
      details.push(`${label}字段 ${path} 可选值已变化`);
    }
  }
  for (const [path, field] of targetFields) {
    const previous = currentFields.get(path);
    if (!previous) {
      if (field.required) tone = "breaking";
      details.push(`${label}新增${field.required ? "必填" : "可选"}字段 ${path}`);
    }
  }
  return {
    tone,
    details: details.length > 0 ? details : [`${label} Schema 约束已变化`]
  };
}

function schemaType(value: Record<string, unknown>): string {
  if (Array.isArray(value.type)) return value.type.map(String).sort().join(" | ");
  return typeof value.type === "string" ? value.type : "未声明";
}

function configFieldDetail(field: FlatConfigField): string {
  const parts = [`类型：${field.type}`];
  if (field.defaultValue !== undefined) parts.push(`默认值：${formatValue(field.defaultValue)}`);
  if (field.enumValues) parts.push(`可选值：${formatValue(field.enumValues)}`);
  return parts.join("；");
}

function fieldMap(fields: FlatConfigField[]): Map<string, FlatConfigField> {
  return new Map(fields.map((field) => [field.path, field]));
}

function enumNarrowed(current: unknown[] | null, target: unknown[] | null): boolean {
  if (!current || !target) return false;
  const targetValues = new Set(target.map(stableJson));
  return current.some((value) => !targetValues.has(stableJson(value)));
}

function permissionMap(permissions: UpdatePermission[]): Map<string, string> {
  return new Map(
    permissions
      .map((permission) => [permission.id.trim(), permission.title.trim()] as const)
      .filter(([id]) => Boolean(id))
  );
}

function rollbackLabel(value: string): string {
  return ({ automatic: "自动", manual: "人工", unsupported: "不支持", not_declared: "未声明" } as Record<string, string>)[value] ?? value;
}

function downtimeLabel(value: string): string {
  return ({ none: "无需停机", brief: "短暂停机", required: "必须停机", not_declared: "未声明" } as Record<string, string>)[value] ?? value;
}

function uniqueNonEmpty(values: string[]): string[] {
  return Array.from(new Set(values.map((value) => value.trim()).filter(Boolean)));
}

function sameJson(left: unknown, right: unknown): boolean {
  return stableJson(left) === stableJson(right);
}

function stableJson(value: unknown): string {
  if (Array.isArray(value)) return `[${value.map(stableJson).join(",")}]`;
  if (isRecord(value)) return `{${Object.keys(value).sort().map((key) => `${JSON.stringify(key)}:${stableJson(value[key])}`).join(",")}}`;
  return JSON.stringify(value) ?? "undefined";
}

function formatValue(value: unknown): string {
  if (value === undefined) return "未设置";
  if (value === null) return "null";
  if (typeof value === "string") return value || "空字符串";
  return stableJson(value);
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
