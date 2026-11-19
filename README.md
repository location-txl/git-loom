# git loom

用原生 Git 管理单仓库或多个关联仓库的开发环境，创建 worktree 时自动带上本地配置。使用 Rust 实现，无需改造仓库为 submodule，不安装 hook。

## 安装

一键安装目前仅支持 **macOS Apple Silicon（M 系列芯片）**，无需安装 Rust。使用时需要系统 Git。

```sh
curl -fsSL https://raw.githubusercontent.com/location-txl/git-loom/main/install.sh | sh
```

默认安装到 `~/.local/bin/git-loom`，重复运行即可升级。

卸载：

```sh
git loom uninstall
```

### 从源码安装

需要 Rust 1.90 或更新版本及系统 Git。

```sh
cargo install --locked --path .
git loom --help
git loom --version
```

确保 Cargo 的 bin 目录在 PATH 中。可执行文件为 `git-loom`，Git 会将 `git loom` 分派给它。

## 配置

运行 `git loom config` 查看内置英文配置指南，包含单仓库和关联仓库示例、字段说明及验证步骤。该命令可在仓库外使用，不会创建或修改文件。

在主仓库根目录创建并提交 `.git-loom.json`，可直接参考 `examples/android.git-loom.json`：

```json
{
  "copyFiles": ["local.properties", ".env", ".env.*"],
  "peers": [
    {
      "path": "../platform",
      "branchPrefix": "tob/",
      "tagPrefix": "tob/",
      "copyFiles": ["local.properties", ".env", ".env.*"]
    }
  ]
}
```

单仓库省略 `peers`。没有配置时按单仓库运行，不额外复制文件。未知字段、错误类型、无效路径或 glob 会直接报错。

- `peers[].path` 相对于主仓库根目录，必须指向仓库根目录；成员不得嵌套、重复或指向同一个 Git 公共目录。
- 分支前缀和 tag 前缀分别配置，缺省为空字符串。只有完整分支名 `main` 和 `master` 不加前缀，例如 `fix/main` 仍会加前缀；同名 tag 不受影响。peer 配置不会被递归读取。
- 如果之前的版本使用了 `tob/main` 等带前缀的主分支，Loom 不会重命名或回退到旧名字。请先确认 peer 存在对应的不带前缀分支，或用 `--repo` 按实际分支名操作。
- 从主仓库子目录调用也使用仓库根目录为路径基准；在 peer 内独立调用不会反向寻找主仓库，可用 `--config /主仓库/.git-loom.json` 指定。
- 全局 `--config` 的路径相对于当前调用目录；指定配置文件应位于主仓库内。
- `.env` 等本地文件仍由项目自己的 `.gitignore` 管理，Loom 不修改忽略规则。

## 日常命令

```sh
git loom status
git loom branch fix/example
git loom switch fix/example
git loom switch -c fix/new
git loom checkout fix/example
git loom checkout -b fix/another
git loom checkout -- app/src/example.kt
git loom diff
git loom diff --cached
git loom diff main...fix/example
```

Loom 全局选项放在子命令前。Loom 自有命令支持逐级 `--help`（例如 `git loom worktree add --help`），拼错自有选项会报错。Git 子命令之后是 Git 参数，`--` 分隔符不会被 Loom 吞掉：

```sh
git loom --repo . diff HEAD~1
git loom --repo ../platform checkout <commit>
git loom --dry-run switch fix/example
```

默认联动全部仓库。使用 `--repo` 时仅操作所选成员，引用使用它的真实名字，不再追加前缀。

分支、tag、远程分支及 `main~2`、`main...feature` 的引用部分自动映射。`HEAD`、`HEAD~1`、`-` 按各仓库自身状态解释。commit SHA 无法推导对应提交，多仓库下要求 `--repo`。路径用 `--` 分隔，相对于各自仓库根目录；有歧义的分支/tag 用完整 `refs/heads/…` 或 `refs/tags/…`。

checkout 的文件恢复、patch、detach、orphan 等仍由 Git 执行。多仓库按子命令识别支持的参数，不猜测未知选项的含义；不支持的组合请用 `--repo` 或原生 Git。单仓库或指定 `--repo` 时使用原始 Git 参数，仍执行 Loom 的检出保护；如果文件名不是 UTF-8，路径可放在 `diff`、`checkout` 或 `status` 的 `--` 之后。

**改变检出位置前，只要任一参与仓库有已暂存、未暂存、未跟踪文件或未完成的 Git 操作，全部拒绝。** Git 的强制切换参数也不会绕过此限制。忽略文件不阻止切换；文件恢复和 patch 模式允许存在改动。

操作串行执行。跨仓库删除分支时，先逐个确认分支存在、未被任何 worktree 检出，并在非强制删除时检查是否合入上游（没有可用上游时检查 HEAD）。跨仓库只接受本地分支的明确删除参数；远程跟踪分支请用 `--repo` 或原生 Git。预检查不能保证跨仓库原子性；执行中失败会停止后续操作，不自动 stash、回滚或删除已经创建的分支。查看输出及 `git loom status`，再决定恢复方式。

Diff 的仓库标题输出到 stderr，Git 内容输出到 stdout。`--exit-code` / `--quiet` 发现差异返回 1，但继续检查其他仓库；真正错误才中止。多仓库输出保留每个仓库自身的路径，不应直接把合并输出当作单仓库补丁应用。

## 创建 worktree

```sh
git loom worktree add ../修复环境 -b fix/example
git loom worktree add ../已有分支环境 fix/existing
git loom worktree add ../临时查看 --detach HEAD
git loom --dry-run worktree add ../预览环境 -b fix/preview
```

当前实现 add 接收目标目录、`-b` 新分支、可选起点及 `--detach`；目标目录必须不存在，分支必须显式指定。单仓库目标直接作为 worktree；多仓库目标是环境容器，按源目录共同祖先保留相对布局：

```text
修复环境/
├── robotfoodapp/
│   ├── .git-loom.json
│   └── local.properties
└── platform/
    └── local.properties
```

允许源仓库有未提交改动。新分支默认从各仓库当前 HEAD 创建；普通代码改动不会被带入，仅复制 `copyFiles` 指定的本地文件。

`local.properties` 原样复制，所以 `../platform` 仍指向新环境的 platform。Loom 不解析 Android 或前端配置内容，也不会验证项目是否能够编译。

复制规则：

- `*` 不跨目录，`**` 可以跨目录；支持 `?`、字符集合及点文件。
- 读取被 Git 忽略的文件，保留相对路径和普通权限。
- 重复匹配去重，缺失文件跳过，已有目标保留并提示。
- 不复制 `.git` 元数据，不允许绝对或越界模式，不跟随符号链接。
- 不打印文件内容、不暂存。复制失败会明确标注 worktree 已创建、配置未准备完成。
- 新主仓库缺少 `.git-loom.json` 时复制本次配置；已有则保留并提示差异。新复制的配置若尚未被目标分支跟踪，会成为未跟踪文件，需自行提交后才能切换。

## 查看与删除

```sh
git loom worktree list
git loom worktree remove ../修复环境
git loom worktree remove ../修复环境 --force
```

多仓库删除只接受 Loom 登记的环境；单仓库也可删除原生 Git 登记的 linked worktree。先检查全部成员，已暂存、未暂存的代码改动，以及未被 Git 忽略的未跟踪文件都会阻止默认删除。被 Git 忽略的文件（如 `.env`、`local.properties`）不阻止删除，会随 worktree 一起删除，源仓库中的原文件不受影响。显式传 `--force` 可丢弃本地改动和未被忽略的未跟踪文件。

登记环境按整组删除；如果 `--repo` 排除了环境中的成员，会在删除前拒绝操作。`--force` 只允许丢弃所选范围内的本地文件，不扩大仓库选择范围。

不自动解锁，不删除分支，不删除主工作树。环境容器只有为空时才移除，额外文件保留。Git 公共目录下的 `loom/worktrees.json` 保存分组和进度，不保存配置内容。同一 Git 公共目录的 Loom 创建和删除操作互斥，已有写入操作时会拒绝并提示稍后重试；这不锁定原生 Git 命令。失败时保留实际完成记录，`doctor` 可检查状态偏差；原生 Git 操作导致的失效记录不会自动修复。

## Doctor

```sh
git loom doctor
git loom doctor --json
```

只读检查 Git、配置格式、成员关系、前缀、复制规则与匹配数量、当前分支/改动、worktree 分组记录和已知 Zsh 加载配置。不联网、不执行构建、不自动修复，也不显示本地配置内容。

- `ERROR`：配置或环境不可用，退出码 1。
- `WARN`：有改动、分支不匹配、文件未匹配、分组不完整、补全未安装，退出码 0。
- `OK`：检查通过。

JSON 包含 `version: 1` 和 `checks`，各检查含 `id`、`level`、`repo`、`message`、`suggestion`。Zsh 接入是静态检查，不保证当前终端已经加载；自定义安装目录需要自行确认 fpath。

## Zsh 补全

```sh
git loom completions zsh --install
# 当前终端立即加载（需要已经初始化 compinit）
eval "$(git loom completions zsh)"
# 自定义目录
git loom completions zsh --install --dir ~/.zsh/completions
```

默认安装到 `~/.zsh/completions/_git-loom`，只更新带 Loom 标记的文件，拒绝覆盖其他同名文件或符号链接。不修改 `.zshrc`，不清理补全缓存。

在 `compinit` 或 Oh My Zsh 初始化之前加入补全目录（自定义安装目录用同一个路径）：

```zsh
fpath=(~/.zsh/completions $fpath)
```

在最后一次 `compinit` 或加载 Oh My Zsh 和插件之后，显式注册补全：

```zsh
autoload -Uz _git-loom
compdef _git-loom git-loom
```

这样即使 Zsh 复用了安装补全前的缓存，`git-loom` 也能加载已安装的脚本。不需要为了加载 Loom 再执行一次 `compinit`；如果配置里已经有多次初始化，把注册放到最后一次之后。

打开新终端，输入 `git-loom doc` 再按 Tab，应该补成 `doctor`。如果无效，可运行 `print -r -- ${_comps[git-loom]:-未注册}` 检查当前终端是否已注册（应显示 `_git-loom`）。升级二进制后，如使用文件安装方式，重新运行 `git loom completions zsh --install` 刷新脚本。

支持 `git loom` 和 `git-loom`，补全子命令、选项、仓库、分支/tag、worktree 与路径；原生 Git 选项复用系统 Git 补全。内部 `__complete` 只读查询，配置错误时静默返回，保留静态与路径补全。

## 开发与验证

```sh
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
cargo build --locked --release
sh tests/install.sh
```

集成测试使用独立的临时 Git 仓库，覆盖联动拒绝、引用映射、原生 checkout、worktree 复制/删除、并发登记保护、CLI 帮助、doctor 和 Zsh。安装器测试验证正常安装、升级及校验失败时保留旧版本。`ci.yml` 在 Ubuntu 上为 main 和 PR 跑格式、lint、测试及安装脚本检查，不构建 Release、不生成发布包，也不上传产物。`release.yml` 只在推送 `v*` 标签时触发，检查标签与 Cargo 版本一致后，在 macOS ARM 上构建并打包，随后发布。交接用的安装包在 Actions 中仅保留 1 天。

Ubuntu CI 复用 Linux 编译缓存。main 还按 `Cargo.lock` 缓存可跨平台复用的 Cargo 下载，在发版时供 macOS 还原；不跨平台缓存编译产物。macOS 发版的测试与打包使用同一 Release profile 和目标，复用同一次运行的编译结果。按 GitHub 的缓存隔离规则，macOS 编译缓存只加速同一标签重跑，不会跨标签命中。
