# Shared Recipkg skills / 共享技能

This project pins eleven shared skills from their individual canonical GitHub repositories. The reviewed integration baseline is [Recipkg d41ccb6](https://github.com/recipkg/recipkg/commit/d41ccb6). See [the upstream catalog](skills/migrate-to-recipkg/PROJECT_SKILLS.md) for repositories and responsibilities.

```sh
git submodule update --init --recursive
recipkg --version
```

Use the separately installed Recipkg CLI for recipe checks and prompt generation. Shared skills are project development instructions, not runtime dependencies or recipe payload. Update skills in their own upstream repositories, inspect the changes, check out reviewed commits, and explicitly commit this project's gitlinks. Keep project-specific instructions in their existing files.

`generated/` is a container for source checkouts. New independently versioned source uses pinned child submodules at `generated/<source-repository-name>/`, with lowercase, hyphenated repository names. Existing local observation material and unfinished source work remain governed by the project's preservation rules.

本项目通过 11 个独立上游仓库的固定提交子模块复用 Recipkg 技能，不逐项目复制或新建技能仓库。先检查本地修改和上游差异，再更新子模块指针；已有项目自定义技能保持原文件。配方仅分发 UTF-8 Markdown，Git 元数据、源代码和用户数据不进入归档。源码独立仓库通过 `generated/` 下的子目录子模块接入；保留现有观察材料和未完成源码工作。结构检查与提示词生成通过不等于软件已经重建或上线。

## Current project ownership

This repository is now the recipe project itself. Recipes live in `recipes/`; the implementation is pinned at `generated/lmm-api-source/` from `TokenNotIncluded/lmm-api-source`. Run implementation commands from that submodule. Legacy custom skill files are preserved here; use their full implementation-owned counterparts under `generated/lmm-api-source/.agents/skills/` so their relative source/document paths resolve correctly.

## Recipe releases

Publish the implementation first, then update this repository's implementation gitlink, affected recipe patch versions, and `SOURCE.md` references. A release request at `.github/recipe-release-request.json` binds four fields: `version` (the `lmm-api` recipe), `core` (the `lmm-core` recipe), `source_version` (the published implementation version), and `source_revision` (its full commit SHA). Commit the request on `main` only after these identities agree.

The `Qualified recipe release` workflow verifies the published implementation tag and gitlink, runs recipe and archive checks with Recipkg CLI commit `7b7f11ce4aaa7d63f922c57a82b227c8aa6b1f3f`, and generates the recursive build prompt. It publishes an immutable `recipes-v<version>` prerelease containing both recipe archives with recursive Markdown dependencies, a parent repository source archive, `source.json`, the check log, and `SHA256SUMS`. The parent archive excludes implementation and skill submodule contents; `source.json` records the implementation repository and exact revision. The tag and existing assets are never overwritten.

配方发布只证明 Markdown 结构、递归归档和重建提示词可生成。实现验收结果由所绑定的源码发布单独提供，安装和生产部署另行执行。发布后核对工作流结论、tag 提交、来源记录与下载资产校验和；不要把配方归档发布描述为应用已构建或已上线。
