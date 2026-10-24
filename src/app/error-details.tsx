import { useState } from "react";

export function ErrorDetails({ error }: { error: string }) {
  const [copyStatus, setCopyStatus] = useState("");
  async function copy() {
    try { await navigator.clipboard.writeText(error); setCopyStatus("已复制"); }
    catch { setCopyStatus("复制失败，请选择详情文字复制"); }
  }
  const newline = error.indexOf("\n");
  return <>
    <p>{newline < 0 ? error : error.slice(0, newline)}</p>
    {newline >= 0 && <details className="error-details"><summary>查看错误详情</summary><pre>{error.slice(newline + 1)}</pre></details>}
    <button className="ghost" onClick={() => void copy()}>复制错误信息</button>
    {copyStatus && <span role="status">{copyStatus}</span>}
  </>;
}
