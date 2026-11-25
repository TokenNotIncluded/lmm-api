# Shared Recipkg skills / 共享技能

Shared Recipkg skills are pinned in [.gitmodules](../.gitmodules). The [reviewed integration baseline](https://github.com/recipkg/recipkg/commit/d41ccb6) records the original selection. Update a skill in its upstream repository, then review and commit its new gitlink here. To initialize only the recipe skills:

```sh
git submodule update --init --recursive -- .agents/skills/
recipkg --version
```

配方位于 `recipes/`，使用独立安装的 Recipkg 检查 Markdown 包并生成重建提示词。应用开发使用 `generated/lmm-api-source/` 固定提交中的源码、文档与技能。见[项目入口](../README.md)和[技能索引](skills/README.md)。

## Recipe releases

Publish the implementation first, then update this repository's implementation gitlink, affected recipe patch versions, and `SOURCE.md` references. A release request at `.github/recipe-release-request.json` binds four fields: `version` (the `lmm-api` recipe), `core` (the `lmm-core` recipe), `source_version` (the published implementation version), and `source_revision` (its full commit SHA). Commit the request on `main` only after these identities agree.

The `Qualified recipe release` workflow verifies the published implementation tag and gitlink, runs recipe and archive checks with Recipkg CLI commit `7b7f11ce4aaa7d63f922c57a82b227c8aa6b1f3f`, and generates the recursive build prompt. It publishes an immutable `recipes-v<version>` prerelease containing both recipe archives with recursive Markdown dependencies, a parent repository source archive, `source.json`, the check log, and `SHA256SUMS`. The parent archive excludes implementation and skill submodule contents; `source.json` records the implementation repository and exact revision. The tag and existing assets are never overwritten.

配方发布只证明 Markdown 结构、递归归档和重建提示词可生成。实现验收结果由所绑定的源码发布单独提供，安装和生产部署另行执行。发布后核对工作流结论、tag 提交、来源记录与下载资产校验和；不要把配方归档发布描述为应用已构建或已上线。
