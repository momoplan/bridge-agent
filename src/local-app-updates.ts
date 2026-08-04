export interface UpdatePermission {
  id: string;
  title: string;
}

export interface InstalledAppShape {
  methodNames: string[];
  eventNames: string[];
  permissions: UpdatePermission[];
}

export interface MarketAppShape {
  releaseNotes: string[];
  methodNames: string[];
  eventNames: string[];
  permissions: UpdatePermission[];
}

export interface LocalAppUpdateChanges {
  releaseNotes: string[];
  addedCapabilities: string[];
  removedCapabilities: string[];
  addedPermissions: string[];
  removedPermissions: string[];
  hasSpecificChanges: boolean;
}

export function describeLocalAppUpdate(
  installed: InstalledAppShape | null,
  market: MarketAppShape
): LocalAppUpdateChanges {
  const currentCapabilities = installed
    ? new Set([
        ...installed.methodNames.map((name) => `方法：${name}`),
        ...installed.eventNames.map((name) => `事件：${name}`)
      ])
    : new Set<string>();
  const targetCapabilities = new Set([
    ...market.methodNames.map((name) => `方法：${name}`),
    ...market.eventNames.map((name) => `事件：${name}`)
  ]);
  const currentPermissions = permissionMap(installed?.permissions ?? []);
  const targetPermissions = permissionMap(market.permissions);
  const releaseNotes = uniqueNonEmpty(market.releaseNotes);
  const addedCapabilities = difference(targetCapabilities, currentCapabilities);
  const removedCapabilities = difference(currentCapabilities, targetCapabilities);
  const addedPermissions = permissionDifference(targetPermissions, currentPermissions);
  const removedPermissions = permissionDifference(currentPermissions, targetPermissions);

  return {
    releaseNotes,
    addedCapabilities,
    removedCapabilities,
    addedPermissions,
    removedPermissions,
    hasSpecificChanges:
      releaseNotes.length > 0 ||
      addedCapabilities.length > 0 ||
      removedCapabilities.length > 0 ||
      addedPermissions.length > 0 ||
      removedPermissions.length > 0
  };
}

function uniqueNonEmpty(values: string[]): string[] {
  return Array.from(new Set(values.map((value) => value.trim()).filter(Boolean)));
}

function difference(left: Set<string>, right: Set<string>): string[] {
  return Array.from(left).filter((value) => !right.has(value)).sort((a, b) => a.localeCompare(b));
}

function permissionMap(permissions: UpdatePermission[]): Map<string, string> {
  return new Map(
    permissions
      .map((permission) => [permission.id.trim(), permission.title.trim()] as const)
      .filter(([id]) => Boolean(id))
  );
}

function permissionDifference(left: Map<string, string>, right: Map<string, string>): string[] {
  return Array.from(left)
    .filter(([id]) => !right.has(id))
    .map(([id, title]) => title || id)
    .sort((a, b) => a.localeCompare(b));
}
