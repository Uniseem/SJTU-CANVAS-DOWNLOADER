# AGENTS.md

## 双远程 / Dual remotes

提交后请同时推送到 GitHub 与 GitLab：

| Remote | URL |
|---|---|
| `origin` (GitHub) | `https://github.com/Uniseem/SJTU-CANVAS-DOWNLOADER.git` |
| `gitlab` | `https://gitlab.anylocate.cc/anylocate/SJTU-CANVAS-DOWNLOADER.git` |

```bash
git push origin HEAD && git push gitlab HEAD
```

## Fleet / 基础设施

SSH、Mesh、Runner、机器清单等规则在 **Uniseem/machine-management**（GitLab: `anylocate/machine-management`）。做 infra 相关工作前先读该仓的 `README.md` / `AGENTS.md` / `CLAUDE.md`。

## CI

本仓库若仅有 GitHub Actions、且无现成可在本机 GitLab Runner（标签 `linux`/`docker`）运行的 `.gitlab-ci.yml`，则 GitLab 侧为 **代码镜像 only**，不要发明会失败的流水线。清单见 machine-management 的 `GitLab/镜像清单.md`。
