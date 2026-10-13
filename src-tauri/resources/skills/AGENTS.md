# 客户端内嵌技能

- `baijimu-platform/SKILL.md` 是发行副本，不是独立内容源。唯一人工维护源是 `momoplan/baijimu-platform-skill` 仓库根目录的 `SKILL.md`。
- 修改技能内容时先在上游审查并发布新版本，再通过 `scripts/sync-baijimu-platform-skill.mjs` 同步准确标签、正文与发行包 SHA-256。禁止在客户端副本中单独改正文，也不跟随 `main` 或 `latest` 自动更新。
- `PROVENANCE.json` 与正文一起提交。执行 `node scripts/check-bundled-platform-skill.mjs --upstream`，确认正文、标签和公开发行包一致。
- 平台定位是业务包（Bundle）的构建、分发与运行，企业级 AI Coding 和办公基础设施提供支撑；产品流程由官网维护，不在客户端复制开发手册或加入其他技能的强制依赖。
