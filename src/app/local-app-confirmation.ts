import { invoke } from "@tauri-apps/api/core";

export async function confirmLocalAppChange(message: string): Promise<boolean> {
  const result = await invoke<string>("plugin:dialog|message", {
    message,
    title: "确认本机应用操作",
    kind: "warning",
    buttons: "OkCancel"
  });
  return result === "Ok";
}
